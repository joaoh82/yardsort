//! The wire format, pinned. `fixtures/v1/` holds messages written by hand, not generated from
//! these types: a change that alters what goes over the wire fails here, and has to be made on
//! purpose — in the fixture, and in every client that reads it.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::*;

fn fixture(name: &str) -> serde_json::Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/v1/");
    let text = std::fs::read_to_string(format!("{path}{name}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The fixture parses as `T`, and `T` writes it back exactly.
fn pinned<T: Serialize + DeserializeOwned>(name: &str) -> T {
    let json = fixture(name);
    let value: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|error| panic!("{name} does not parse: {error}"));
    assert_eq!(serde_json::to_value(&value).unwrap(), json, "{name}");
    value
}

#[test]
fn requests_match_their_fixtures() {
    let request: Request = pinned("request-workspace-list.json");
    assert_eq!(request.operation.name(), "workspace.list");
    let request: Request = pinned("request-host-describe.json");
    assert_eq!(request.operation, Operation::HostDescribe);
    let request: Request = pinned("request-activity-list.json");
    assert_eq!(request.operation.scope(), Some(Scope::ActivityRead));
    let request: Request = pinned("request-terminal-input.json");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalControl));
    let request: Request = pinned("request-activity-subscribe.json");
    assert_eq!(request.operation.scope(), Some(Scope::ActivityRead));
    assert!(request.operation.validate().is_ok());
    let hello: Hello = pinned("hello.json");
    assert_eq!(negotiate(&hello), Ok(ProtocolVersion::CURRENT));
}

#[test]
fn responses_match_their_fixtures() {
    for name in [
        "response-host.json",
        "response-host-terminals-later.json",
        "response-workspaces.json",
        "response-sessions.json",
        "response-activity.json",
        "response-error-incompatible.json",
        "response-attached.json",
        "response-attached-resumed.json",
        "response-subscribed.json",
    ] {
        let _: Response = pinned(name);
    }
    // A host before minor 4 says nothing of it; one that keeps an earlier daemon says why.
    let Outcome::Result(Reply::Host(older)) = pinned::<Response>("response-host.json").outcome
    else {
        panic!("not a host");
    };
    assert_eq!(older.terminals_later, None);
    let Outcome::Result(Reply::Host(waiting)) =
        pinned::<Response>("response-host-terminals-later.json").outcome
    else {
        panic!("not a host");
    };
    let later = waiting.terminals_later.unwrap();
    assert_eq!(later.reason, TerminalsLaterReason::EarlierDaemon);
    assert!(!waiting
        .capabilities
        .iter()
        .any(|c| c.starts_with("terminal.")));
    let newer: TerminalsLater =
        serde_json::from_value(serde_json::json!({"reason": "somethingNew", "message": "Later."}))
            .unwrap();
    assert_eq!(newer.reason, TerminalsLaterReason::Unknown);
    let subscribed: Response = pinned("response-subscribed.json");
    assert!(matches!(
        subscribed.outcome,
        Outcome::Result(Reply::Subscribed(Subscribed {
            subscription: 1,
            newer: Some(_)
        }))
    ));
    let resumed: Response = pinned("response-attached-resumed.json");
    assert!(matches!(
        resumed.outcome,
        Outcome::Result(Reply::Attached(Attached {
            start: AttachStart::Resumed,
            ..
        }))
    ));
    let response: Response = pinned("response-error-incompatible.json");
    assert_eq!(
        response.outcome,
        Outcome::Error(RemoteError::incompatible())
    );
}

#[test]
fn account_messages_match_their_fixtures() {
    use crate::account::*;
    let server: ServerDescription = pinned("account-server.json");
    assert_eq!(server.issuer, "https://auth.example.invalid");
    let sign_in: TokenRequest = pinned("account-token-sign-in.json");
    assert!(sign_in.validate().is_ok());
    let refresh: TokenRequest = pinned("account-token-refresh.json");
    assert!(refresh.validate().is_ok());
    let _: Credentials = pinned("account-credentials.json");
    let page: DevicePage = pinned("account-devices.json");
    assert_eq!(page.devices[1].kind, DeviceKind::Host);
}

#[test]
fn pairing_messages_match_their_fixtures() {
    use crate::pairing::*;
    let registration: HostRegistration = pinned("pairing-host-registration.json");
    assert!(registration.validate().is_ok());
    let _: HostPage = pinned("pairing-hosts.json");
    let offer: Offer = pinned("pairing-offer.json");
    assert!(normalize_code(offer.code.expose()).is_some());
    let request: PairingRequest = pinned("pairing-request.json");
    assert!(request.validate().is_ok());
    let page: GrantPage = pinned("pairing-grants.json");
    assert_eq!(page.grants[0].state, GrantState::Approved);
    assert_eq!(page.grants[1].state, GrantState::Pending);
    let decision: GrantDecision = pinned("pairing-decision.json");
    assert!(decision.validate(&request.scopes).is_ok());
    // A newer server's state reads as unknown, not as a failure to read the page.
    let mut json = fixture("pairing-grants.json");
    json["grants"][0]["state"] = "suspended".into();
    let page: GrantPage = serde_json::from_value(json).unwrap();
    assert_eq!(page.grants[0].state, GrantState::Unknown);
}

#[test]
fn relay_messages_and_events_match_their_fixtures() {
    use crate::relay::*;
    let request: TicketRequest = pinned("relay-ticket-request.json");
    assert!(matches!(request, TicketRequest::Device { .. }));
    let ticket: RelayTicket = pinned("relay-ticket.json");
    assert_eq!(ticket.expires_in_ms, TICKET_TTL_MS);
    assert_eq!(ticket.route, None, "a server of one instance");
    let routed: RelayTicket = pinned("relay-ticket-routed.json");
    assert!(routed.route.as_deref().is_some_and(valid_route));
    let _: Presented = pinned("relay-presented.json");
    let notice: HostNotice = pinned("relay-notice-pairing.json");
    assert!(matches!(notice, HostNotice::PairingRequested { .. }));
    // A newer server's notice reads as one to ignore, not as a broken link.
    let newer: HostNotice = serde_json::from_str(r#"{"kind":"hostRenamed","name":"x"}"#).unwrap();
    assert_eq!(newer, HostNotice::Unknown);
    let greeting: Greeting = pinned("relay-greeting.json");
    assert!(matches!(greeting, Greeting::Connected { .. }));
    let reply: HelloReply = pinned("hello-welcome.json");
    assert!(matches!(reply, HelloReply::Welcome(_)));
    let resized: Event = pinned("event-terminal-resized.json");
    assert!(matches!(resized, Event::TerminalResized(_)));
    let detached: Event = pinned("event-terminal-detached.json");
    let Event::TerminalDetached(detached) = detached else {
        panic!("{detached:?}")
    };
    assert_eq!(detached.error.code, ErrorCode::ResnapshotRequired);
    let control: Event = pinned("event-terminal-control.json");
    assert!(matches!(
        control,
        Event::TerminalControl(ControlChanged {
            controller: Controller::Device { .. },
            ..
        })
    ));
    let exited: Event = pinned("event-terminal-exited.json");
    assert!(matches!(
        exited,
        Event::TerminalExited(Exited {
            exit: ProgramExit {
                code: 0,
                success: true
            },
            ..
        })
    ));
    let batch: Event = pinned("event-activity-events.json");
    let Event::ActivityEvents(batch) = batch else {
        panic!("{batch:?}")
    };
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.newer.as_str(), "a1.n4223");
    let ended: Event = pinned("event-activity-ended.json");
    let Event::ActivityEnded(ended) = ended else {
        panic!("{ended:?}")
    };
    assert_eq!(ended.error, RemoteError::not_found());
    // An event is told from a response by its `event` field.
    assert!(serde_json::from_value::<Event>(fixture("response-host.json")).is_err());
}

#[test]
fn every_operation_has_the_name_it_is_sent_under() {
    let terminal = TerminalId::new("t").unwrap();
    let lease = LeaseRef {
        terminal: terminal.clone(),
        lease: LeaseId::new("l").unwrap(),
        generation: 1,
    };
    let workspace = WorkspaceId::new("w").unwrap();
    let all = [
        Operation::HostDescribe,
        Operation::WorkspaceList(WorkspaceListParams::default()),
        Operation::WorkspaceDescribe(WorkspaceRef {
            workspace: workspace.clone(),
        }),
        Operation::SessionList(SessionListParams {
            workspace,
            include_ended: false,
            cursor: None,
            limit: None,
        }),
        Operation::ActivityList(ActivityListParams {
            workspace: WorkspaceId::new("w").unwrap(),
            session: None,
            cursor: None,
            limit: None,
        }),
        Operation::MemoryList(MemoryListParams {
            project: ProjectId::new("p").unwrap(),
            cursor: None,
            limit: None,
        }),
        Operation::TerminalAttach(AttachRequest {
            terminal: terminal.clone(),
            resume_after: None,
        }),
        Operation::TerminalDetach(DetachRequest {
            terminal: terminal.clone(),
            stream: 1,
        }),
        Operation::TerminalAcquire(AcquireRequest {
            terminal: terminal.clone(),
            take_over: false,
        }),
        Operation::TerminalRenew(lease.clone()),
        Operation::TerminalRelease(lease.clone()),
        Operation::TerminalInput(InputRequest {
            lease: lease.clone(),
            seq: 1,
            data: "x".into(),
            paste: false,
        }),
        Operation::TerminalResize(ResizeRequest {
            lease,
            size: TermSize { cols: 80, rows: 24 },
        }),
        Operation::ActivitySubscribe(ActivitySubscribeParams::default()),
        Operation::ActivityUnsubscribe(ActivityUnsubscribeParams { subscription: 1 }),
    ];
    assert_eq!(all.len(), OPERATIONS.len());
    for operation in all {
        let json = serde_json::to_value(&operation).unwrap();
        assert_eq!(json["op"], operation.name());
    }
}

#[test]
fn a_sessions_page_without_terminals_reads_as_having_none() {
    // What a host from before terminals were listed sends: still 1.x, and still understood.
    let mut json = fixture("response-sessions.json");
    json["result"].as_object_mut().unwrap().remove("terminals");
    let response: Response = serde_json::from_value(json).unwrap();
    let Outcome::Result(Reply::Sessions(page)) = response.outcome else {
        panic!("{response:?}")
    };
    assert_eq!(page.sessions.len(), 1);
    assert!(page.terminals.is_empty());
}

fn error_of(bytes: &[u8]) -> (Option<RequestId>, RemoteError) {
    match decode_request(bytes) {
        Ok(request) => panic!("decoded {request:?}"),
        Err(response) => match *response {
            Response {
                request_id,
                outcome: Outcome::Error(error),
            } => (request_id, error),
            other => panic!("not an error: {other:?}"),
        },
    }
}

#[test]
fn another_major_is_refused_with_the_majors_spoken_here() {
    let (id, error) = error_of(
        br#"{"requestId":"r","protocol":{"major":2,"minor":0},"op":"workspace.list","params":{"newThing":{"a":1},"cursor":7}}"#,
    );
    assert_eq!(id.unwrap().as_str(), "r");
    assert_eq!(error.code, ErrorCode::ProtocolIncompatible);
    assert_eq!(error.supported_protocols, Some(vec![PROTOCOL_MAJOR]));
    assert!(!error.retryable);

    let hello = Hello {
        client: "old".into(),
        protocols: vec![0, 2],
    };
    assert_eq!(
        negotiate(&hello).unwrap_err().code,
        ErrorCode::ProtocolIncompatible
    );
}

#[test]
fn a_newer_minor_with_fields_this_build_does_not_know_is_understood() {
    let request = decode_request(
        br#"{"requestId":"r","protocol":{"major":1,"minor":7},"op":"workspace.list","params":{"includeArchived":true,"sortBy":"recent"},"trace":"x"}"#,
    )
    .unwrap();
    assert!(matches!(
        request.operation,
        Operation::WorkspaceList(WorkspaceListParams {
            include_archived: true,
            ..
        })
    ));
}

#[test]
fn an_unknown_operation_is_unsupported_not_malformed() {
    let (_, error) = error_of(
        br#"{"requestId":"r","protocol":{"major":1,"minor":3},"op":"workspace.delete","params":{}}"#,
    );
    assert_eq!(error.code, ErrorCode::Unsupported);
}

#[test]
fn malformed_requests_are_refused_without_echoing_them() {
    for bytes in [
        &b"not json"[..],
        br#"{"requestId":"r","protocol":{"major":1,"minor":0},"op":"workspace.describe","params":{"workspace":"/home/someone/secret"}}"#,
        br#"{"requestId":"r","protocol":{"major":1,"minor":0},"op":"session.list"}"#,
        br#"[1,2,3]"#,
    ] {
        let (_, error) = error_of(bytes);
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(!error.message.contains("secret"), "{}", error.message);
    }
}

#[test]
fn a_request_id_that_is_not_an_id_is_not_echoed() {
    let (id, _) = error_of(br#"{"requestId":"has space","op":"nope"}"#);
    assert_eq!(id, None);
}

#[test]
fn oversized_requests_are_refused_before_parsing() {
    let mut big =
        br#"{"requestId":"r","protocol":{"major":1,"minor":0},"op":"host.describe","pad":""#
            .to_vec();
    big.extend(std::iter::repeat_n(b'a', MAX_CONTROL_FRAME));
    big.extend_from_slice(br#""}"#);
    let (id, error) = error_of(&big);
    assert_eq!(id, None, "not parsed at all");
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

#[test]
fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
    let mut deep =
        br#"{"requestId":"r","protocol":{"major":1,"minor":0},"op":"host.describe","x":"#.to_vec();
    deep.extend(std::iter::repeat_n(b'[', 10_000));
    deep.extend(std::iter::repeat_n(b']', 10_000));
    deep.push(b'}');
    assert!(deep.len() < MAX_CONTROL_FRAME);
    let (_, error) = error_of(&deep);
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

#[test]
fn bounds_are_checked_after_parsing() {
    let (id, error) = error_of(
        br#"{"requestId":"r","protocol":{"major":1,"minor":0},"op":"workspace.list","params":{"limit":0}}"#,
    );
    assert_eq!(id.unwrap().as_str(), "r");
    assert_eq!(error.code, ErrorCode::InvalidRequest);

    let data = "a".repeat(MAX_INPUT_BYTES + 1);
    let request = format!(
        r#"{{"requestId":"r","protocol":{{"major":1,"minor":0}},"op":"terminal.input","params":{{"terminal":"t","lease":"l","generation":1,"seq":1,"data":"{data}"}}}}"#
    );
    assert_eq!(
        error_of(request.as_bytes()).1.code,
        ErrorCode::InvalidRequest
    );

    // A conversation is followed within its workspace.
    let (_, error) = error_of(
        br#"{"requestId":"r","protocol":{"major":1,"minor":3},"op":"activity.subscribe","params":{"session":"s"}}"#,
    );
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    // Everything is optional: all the grant reaches, from now on.
    let request = decode_request(
        br#"{"requestId":"r","protocol":{"major":1,"minor":3},"op":"activity.subscribe","params":{}}"#,
    )
    .unwrap();
    assert_eq!(
        request.operation,
        Operation::ActivitySubscribe(ActivitySubscribeParams::default())
    );

    assert_eq!(page_size(None), DEFAULT_PAGE as usize);
    assert_eq!(page_size(Some(10_000)), MAX_PAGE as usize);
}

#[test]
fn an_error_code_from_a_newer_peer_is_read_as_unknown() {
    let error: RemoteError =
        serde_json::from_str(r#"{"code":"quota_exceeded","message":"m","retryable":true}"#)
            .unwrap();
    assert_eq!(error.code, ErrorCode::Unknown);
    assert!(!ErrorCode::Unknown.retryable());
}

#[test]
fn only_transient_failures_are_retryable() {
    for code in [
        ErrorCode::HostOffline,
        ErrorCode::RateLimited,
        ErrorCode::Internal,
    ] {
        assert!(RemoteError::new(code, "m").retryable, "{code:?}");
    }
    for code in [
        ErrorCode::Forbidden,
        ErrorCode::ProtocolIncompatible,
        ErrorCode::ControlConflict,
        ErrorCode::LeaseLost,
        ErrorCode::InvalidRequest,
    ] {
        assert!(!RemoteError::new(code, "m").retryable, "{code:?}");
    }
}

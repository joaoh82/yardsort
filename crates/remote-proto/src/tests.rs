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
    let request: Request = pinned("request-workspace-start.json");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalControl));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-session-resume.json");
    assert_eq!(request.operation.name(), "session.resume");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalControl));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-upload-chunk.json");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalControl));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-workflow-run-list.json");
    assert_eq!(request.operation.name(), "workflow.run.list");
    assert_eq!(request.operation.scope(), Some(Scope::WorkflowRead));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-workflow-decide.json");
    assert_eq!(request.operation.name(), "workflow.decide");
    assert_eq!(request.operation.scope(), Some(Scope::WorkflowDecide));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-workflow-challenge.json");
    assert_eq!(request.operation.scope(), Some(Scope::WorkflowDecide));
    assert!(request.operation.validate().is_ok());
    for (name, op, scope) in [
        (
            "request-workspace-start-full.json",
            "workspace.start",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-start-branch.json",
            "workspace.start",
            Scope::TerminalControl,
        ),
        (
            "request-project-branches.json",
            "project.branches",
            Scope::WorkspaceRead,
        ),
        (
            "request-project-clone.json",
            "project.clone",
            Scope::TerminalControl,
        ),
        (
            "request-shell-start.json",
            "shell.start",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-rename.json",
            "workspace.rename",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-archive.json",
            "workspace.archive",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-delete.json",
            "workspace.delete",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-changes.json",
            "workspace.changes",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-files.json",
            "workspace.files",
            Scope::TerminalControl,
        ),
        (
            "request-file-read.json",
            "file.read",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-publish.json",
            "workspace.publish",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-commit.json",
            "workspace.commit",
            Scope::TerminalControl,
        ),
        (
            "request-workspace-push.json",
            "workspace.push",
            Scope::TerminalControl,
        ),
        (
            "request-pull-request-open.json",
            "pull_request.open",
            Scope::TerminalControl,
        ),
        (
            "request-pull-request-merge.json",
            "pull_request.merge",
            Scope::TerminalControl,
        ),
    ] {
        let request: Request = pinned(name);
        assert_eq!(request.operation.name(), op, "{name}");
        assert_eq!(request.operation.scope(), Some(scope), "{name}");
        assert!(request.operation.validate().is_ok(), "{name}");
    }
    // Archiving says nothing of force unless it is asked for; deleting here asks.
    let Operation::WorkspaceArchive(archive) =
        pinned::<Request>("request-workspace-archive.json").operation
    else {
        panic!("not an archive");
    };
    assert!(!archive.force);
    let Operation::WorkspaceDelete(delete) =
        pinned::<Request>("request-workspace-delete.json").operation
    else {
        panic!("not a delete");
    };
    assert!(delete.force);
    let request: Request = pinned("request-conversation-read.json");
    assert_eq!(request.operation.name(), "conversation.read");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalRead));
    assert!(request.operation.validate().is_ok());
    let request: Request = pinned("request-approval-answer.json");
    assert_eq!(request.operation.name(), "approval.answer");
    assert_eq!(request.operation.scope(), Some(Scope::TerminalControl));
    assert!(request.operation.validate().is_ok());
    let hello: Hello = pinned("hello.json");
    assert_eq!(negotiate(&hello), Ok(ProtocolVersion::CURRENT));
}

#[test]
fn a_conversation_is_read_from_turn_one_and_an_answer_names_a_question_yardsort_asked() {
    let read = |from: Option<u32>| {
        Operation::ConversationRead(conversation::ConversationReadParams {
            workspace: WorkspaceId::new("8c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f").unwrap(),
            session: SessionRecordId::new("3d4e5f6a-7b8c-4d9e-8f0a-1b2c3d4e5f6a").unwrap(),
            from,
        })
        .validate()
    };
    assert!(read(None).is_ok());
    assert!(read(Some(1)).is_ok());
    assert_eq!(read(Some(0)).unwrap_err().code, ErrorCode::InvalidRequest);
    let answer = |ask: &str| {
        Operation::ApprovalAnswer(conversation::ApprovalAnswerParams {
            workspace: WorkspaceId::new("8c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f").unwrap(),
            session: SessionRecordId::new("3d4e5f6a-7b8c-4d9e-8f0a-1b2c3d4e5f6a").unwrap(),
            ask: ask.to_owned(),
            decision: conversation::ApprovalDecision::Allow,
        })
        .validate()
    };
    assert!(answer("7f6e5d4c-3b2a-4190-8f7e-6d5c4b3a2918").is_ok());
    for bad in ["", "../asks/x", "a b", &"x".repeat(65)] {
        assert_eq!(
            answer(bad).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "{bad}"
        );
    }
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
        "response-harnesses.json",
        "response-started.json",
        "response-uploaded.json",
        "response-workflow-runs.json",
        "response-workflow-run.json",
        "response-workflow-decision.json",
        "response-workflow-challenge.json",
        "response-conversation.json",
        "response-harnesses-choices.json",
        "response-branches.json",
        "response-cloned.json",
        "response-started-shell.json",
        "response-done.json",
        "response-changes.json",
        "response-files.json",
        "response-file.json",
        "response-file-too-large.json",
        "response-publish.json",
        "response-pull-request-opened.json",
    ] {
        let _: Response = pinned(name);
    }
    // Since minor 11: a file that is not sent says why, and carries nothing.
    let Outcome::Result(Reply::File(large)) =
        pinned::<Response>("response-file-too-large.json").outcome
    else {
        panic!("not a file");
    };
    assert_eq!(large.state, FileState::TooLarge);
    assert!(large.data.is_empty() && large.done);
    // Since minor 10: why archiving or deleting was refused, for a client to ask again.
    let Outcome::Error(dirty) = pinned::<Response>("response-error-uncommitted.json").outcome
    else {
        panic!("not an error");
    };
    assert_eq!(dirty.code, ErrorCode::UncommittedWork);
    assert!(!dirty.retryable);
    // A harness before minor 10 lists no models and no efforts, and reads as having none.
    let Outcome::Result(Reply::Harnesses(listed)) =
        pinned::<Response>("response-harnesses-choices.json").outcome
    else {
        panic!("not harnesses");
    };
    assert_eq!(listed.harnesses[0].efforts, ["low", "high"]);
    assert!(listed.harnesses[1].models.is_empty());
    let Outcome::Result(Reply::Harnesses(older)) =
        pinned::<Response>("response-harnesses.json").outcome
    else {
        panic!("not harnesses");
    };
    assert!(older.harnesses.iter().all(|h| h.models.is_empty()));
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
    let notice: HostNotice = pinned("relay-notice-sync.json");
    assert_eq!(notice, HostNotice::SyncChanged);
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
        Operation::HarnessList,
        Operation::WorkspaceStart(WorkspaceStartParams::plain(
            ProjectId::new("p").unwrap(),
            "Fix it",
            None,
        )),
        Operation::SessionStart(SessionStartParams {
            workspace: WorkspaceId::new("w").unwrap(),
            harness: None,
            prompt: None,
        }),
        Operation::UploadBegin(UploadBeginParams {
            workspace: WorkspaceId::new("w").unwrap(),
            name: "photo.jpg".into(),
            size: 1,
        }),
        Operation::UploadChunk(UploadChunkParams {
            upload: 1,
            offset: 0,
            data: "aGk=".into(),
        }),
        Operation::SessionResume(SessionResumeParams {
            session: SessionRecordId::new("s").unwrap(),
            prompt: None,
        }),
        Operation::WorkflowRunList(workflow::WorkflowRunListParams {
            cursor: None,
            limit: None,
        }),
        Operation::WorkflowRunGet(workflow::WorkflowRunRef {
            run_id: "run-1".into(),
        }),
        Operation::WorkflowChallenge(workflow::WorkflowStepRef {
            run_id: "run-1".into(),
            step_id: "sign-off".into(),
            attempt: 1,
            round: 0,
        }),
        Operation::WorkflowDecide(workflow::WorkflowDecideParams {
            run_id: "run-1".into(),
            step_id: Some("sign-off".into()),
            action: workflow::WorkflowAction::Approve,
            note: None,
            code: Some("ABCD2345".into()),
            decision_id: "d-1".into(),
            attempt: 1,
            round: 0,
        }),
        Operation::ConversationRead(conversation::ConversationReadParams {
            workspace: WorkspaceId::new("w").unwrap(),
            session: SessionRecordId::new("s").unwrap(),
            from: None,
        }),
        Operation::ApprovalAnswer(conversation::ApprovalAnswerParams {
            workspace: WorkspaceId::new("w").unwrap(),
            session: SessionRecordId::new("s").unwrap(),
            ask: "a".into(),
            decision: conversation::ApprovalDecision::Deny,
        }),
        Operation::ProjectBranches(ProjectRef {
            project: ProjectId::new("p").unwrap(),
        }),
        Operation::ProjectClone(ProjectCloneParams {
            remote: "https://example.com/a.git".into(),
            name: None,
        }),
        Operation::ShellStart(ShellStartParams {
            workspace: WorkspaceId::new("w").unwrap(),
        }),
        Operation::WorkspaceRename(WorkspaceRenameParams {
            workspace: WorkspaceId::new("w").unwrap(),
            name: "n".into(),
        }),
        Operation::WorkspaceArchive(WorkspaceRemoveParams {
            workspace: WorkspaceId::new("w").unwrap(),
            force: false,
        }),
        Operation::WorkspaceDelete(WorkspaceRemoveParams {
            workspace: WorkspaceId::new("w").unwrap(),
            force: false,
        }),
        Operation::WorkspaceChanges(WorkspaceRef {
            workspace: WorkspaceId::new("w").unwrap(),
        }),
        Operation::WorkspaceFiles(WorkspaceFilesParams {
            workspace: WorkspaceId::new("w").unwrap(),
            dir: String::new(),
        }),
        Operation::FileRead(FileReadParams {
            workspace: WorkspaceId::new("w").unwrap(),
            path: "a".into(),
            old_path: None,
            of: FileOf::Working,
            offset: 0,
        }),
        Operation::WorkspacePublish(publish::WorkspacePublishParams {
            workspace: WorkspaceId::new("w").unwrap(),
            forge: false,
        }),
        Operation::WorkspaceCommit(publish::WorkspaceCommitParams {
            workspace: WorkspaceId::new("w").unwrap(),
            message: "m".into(),
        }),
        Operation::WorkspacePush(WorkspaceRef {
            workspace: WorkspaceId::new("w").unwrap(),
        }),
        Operation::PullRequestOpen(publish::PullRequestOpenParams {
            workspace: WorkspaceId::new("w").unwrap(),
            title: "t".into(),
            body: String::new(),
            draft: false,
        }),
        Operation::PullRequestMerge(publish::PullRequestMergeParams {
            workspace: WorkspaceId::new("w").unwrap(),
            number: 1,
            head_oid: "abcdef0".into(),
            method: publish::MergeMethod::Squash,
        }),
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
    assert_eq!(page.sessions.len(), 2);
    assert!(page.terminals.is_empty());
}

#[test]
fn a_session_from_a_host_before_minor_6_is_not_resumable() {
    let mut json = fixture("response-sessions.json");
    let ended = &mut json["result"]["sessions"][1];
    assert_eq!(ended["resumable"], true);
    ended.as_object_mut().unwrap().remove("resumable");
    let response: Response = serde_json::from_value(json).unwrap();
    let Outcome::Result(Reply::Sessions(page)) = response.outcome else {
        panic!("{response:?}")
    };
    assert!(!page.sessions[1].resumable);
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
        br#"{"requestId":"r","protocol":{"major":1,"minor":3},"op":"memory.edit","params":{}}"#,
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

fn hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "{text}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn output_frames_match_their_fixtures() {
    let json = fixture("frame-output.json");
    let frames = json["frames"].as_array().unwrap();
    assert!(!frames.is_empty());
    for case in frames {
        let wire = hex(case["hex"].as_str().unwrap());
        let frame = OutputFrame::decode(&wire).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert_eq!(u64::from(frame.stream), case["stream"].as_u64().unwrap());
        assert_eq!(
            u64::from(frame.position.epoch),
            case["epoch"].as_u64().unwrap()
        );
        assert_eq!(frame.position.seq, case["seq"].as_u64().unwrap());
        assert_eq!(frame.bytes, hex(case["bytes"].as_str().unwrap()));
        let mut written = Vec::new();
        frame.encode(&mut written).unwrap();
        let expected = case.get("reencoded").unwrap_or(&case["hex"]);
        assert_eq!(written, hex(expected.as_str().unwrap()), "{case}");
    }
}

#[test]
fn refused_frames_match_their_fixtures() {
    use crate::terminal::FrameError;
    let json = fixture("frame-refused.json");
    for case in json["frames"].as_array().unwrap() {
        let mut wire = hex(case["hex"].as_str().unwrap());
        let padding = case["padding"].as_u64().unwrap_or(0);
        wire.resize(wire.len() + padding as usize, 0);
        let error = OutputFrame::decode(&wire).expect_err(&case.to_string());
        let named = match error {
            FrameError::Truncated => "truncated",
            FrameError::WrongKind => "wrongKind",
            FrameError::UnknownVersion(_) => "unknownVersion",
            FrameError::TooLarge(_) => "tooLarge",
        };
        assert_eq!(named, case["error"], "{case}");
    }
}

/// Every fixture is read by a test here, as every one is by the TypeScript client's
/// (`packages/remote-client/test/fixtures.test.ts`): a file only one side reads is a wire format
/// only one side is held to.
#[test]
fn every_fixture_is_read_here() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/v1/");
    let here = include_str!("tests.rs");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 43, "{names:?}");
    for name in names {
        assert!(name.ends_with(".json"), "{name} is not a fixture");
        assert!(
            here.contains(&format!("\"{name}\"")),
            "no test reads {name}"
        );
    }
}

fn sync_fixture<T: Serialize + DeserializeOwned>(name: &str) -> T {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/sync-v1/");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{path}{name}")).unwrap()).unwrap();
    let parsed: T = serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json, "{name}");
    parsed
}

#[test]
fn sync_messages_match_their_fixtures() {
    use crate::sync::*;
    let push: PushRequest = sync_fixture("push.json");
    assert!(push.check().is_ok());
    assert!(push.ops.iter().all(|op| op.check().is_ok()));
    let actions: Vec<Action> = push.ops.iter().map(|op| op.action).collect();
    assert_eq!(
        actions,
        [Action::Put, Action::Put, Action::Revoke, Action::Delete]
    );
    let reply: PushReply = sync_fixture("push-reply.json");
    assert!(matches!(
        reply.results[0],
        OpResult::Applied { revision: 1, .. }
    ));
    let OpResult::Conflict { current, .. } = &reply.results[1] else {
        panic!("a conflict carries the record as it is");
    };
    assert_eq!(current.revision, 4);
    assert!(
        matches!(&reply.results[3], OpResult::Refused { error, .. } if error.code == ErrorCode::NotFound)
    );
    let page: SyncPage = sync_fixture("page.json");
    assert!(page.records[0].revoked && page.records[0].body.is_some());
    assert!(page.records[1].deleted && page.records[1].body.is_none());
    assert_eq!(page.cursor, page.records[1].cursor);
    let snapshot: SnapshotPage = sync_fixture("snapshot.json");
    assert!(snapshot.next.is_some());
    let projects: SyncProjects = sync_fixture("projects.json");
    assert_eq!(projects.projects[0].record.kind, RecordKind::Project);

    // A newer server's outcome, a newer kind or action, read as ones to leave alone.
    let newer: OpResult = serde_json::from_str(r#"{"outcome":"merged","op":"x"}"#).unwrap();
    assert_eq!(newer, OpResult::Unknown);
    let mut op = push.ops[0].clone();
    op.kind = serde_json::from_str(r#""branch""#).unwrap();
    assert_eq!(op.kind, RecordKind::Unknown);
    assert!(op.check().is_err());
}

#[test]
fn every_sync_fixture_is_read_here() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/sync-v1/");
    let here = include_str!("tests.rs");
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            here.contains(&format!("sync_fixture(\"{name}\")")),
            "no test reads {name}"
        );
    }
}

#[test]
fn sync_ids_are_lowercase_uuid_v4s() {
    use crate::sync::{OpId, SyncId};
    assert!(SyncId::new("5f0c8a52-6a1e-4d3b-8c2f-1e9d7b6a5c40").is_ok());
    for bad in [
        "",
        "5F0C8A52-6A1E-4D3B-8C2F-1E9D7B6A5C40",
        "5f0c8a52-6a1e-1d3b-8c2f-1e9d7b6a5c40",
        "5f0c8a52-6a1e-4d3b-cc2f-1e9d7b6a5c40",
        "5f0c8a526a1e4d3b8c2f1e9d7b6a5c40",
        "../../5f0c8a52-6a1e-4d3b-8c2f-1e9d7b",
        "project-1",
    ] {
        assert!(SyncId::new(bad).is_err(), "{bad}");
        assert!(OpId::new(bad).is_err(), "{bad}");
    }
}

#[test]
fn sync_ops_are_checked_on_their_own() {
    use crate::sync::*;
    let push: PushRequest = sync_fixture("push.json");
    let [project, note, revoke, delete] = [0, 1, 2, 3].map(|at| push.ops[at].clone());
    let fails = |op: SyncOp| op.check().is_err();

    assert!(!fails(project.clone()));
    assert!(
        fails(SyncOp {
            id: note.id.clone(),
            ..project.clone()
        }),
        "a project is its own"
    );
    assert!(
        fails(SyncOp {
            project: note.id.clone(),
            ..note.clone()
        }),
        "a note is not"
    );
    assert!(fails(SyncOp {
        body: None,
        ..note.clone()
    }));
    assert!(fails(SyncOp {
        schema: Some(0),
        ..note.clone()
    }));
    assert!(fails(SyncOp {
        body: Some(serde_json::json!("text")),
        ..note.clone()
    }));
    let large = serde_json::json!({ "text": "x".repeat(MAX_RECORD_BODY) });
    assert!(fails(SyncOp {
        body: Some(large),
        ..note.clone()
    }));
    assert!(
        fails(SyncOp {
            kind: RecordKind::Workspace,
            ..revoke.clone()
        }),
        "only notes"
    );
    assert!(fails(SyncOp {
        body: note.body.clone(),
        ..revoke.clone()
    }));
    assert!(fails(SyncOp {
        base: None,
        ..delete.clone()
    }));
    assert!(fails(SyncOp {
        action: Action::Unknown,
        ..delete.clone()
    }));

    let many = PushRequest {
        ops: (0..=MAX_PUSH_OPS).map(|_| note.clone()).collect(),
    };
    assert!(many.check().is_err());
    assert!(PushRequest { ops: vec![] }.check().is_err());
    assert!(
        PushRequest {
            ops: vec![note.clone(), note]
        }
        .check()
        .is_err(),
        "an op id twice"
    );
}

#[test]
fn starting_and_uploading_are_bounded() {
    let plain =
        |prompt: &str| WorkspaceStartParams::plain(ProjectId::new("p").unwrap(), prompt, None);
    let start = |prompt: &str| Operation::WorkspaceStart(plain(prompt)).validate();
    assert!(start("Fix it").is_ok());
    assert!(start("  \n").is_err());
    assert!(start(&"x".repeat(MAX_PROMPT + 1)).is_err());
    // What a start may say since minor 10, each within its bounds.
    assert!(!plain("Fix it").says_more());
    let saying = |change: &dyn Fn(&mut WorkspaceStartParams)| {
        let mut params = plain("Fix it");
        change(&mut params);
        assert!(params.says_more());
        Operation::WorkspaceStart(params).validate()
    };
    assert!(saying(&|p| p.name = Some("fix-login".into())).is_ok());
    assert!(saying(&|p| p.name = Some("  ".into())).is_err());
    assert!(saying(&|p| p.name = Some("n".repeat(MAX_WORKSPACE_NAME + 1))).is_err());
    assert!(saying(&|p| p.model = Some("opus".into())).is_ok());
    assert!(saying(&|p| p.model = Some("--dangerously".into())).is_err());
    assert!(saying(&|p| p.effort = Some("e".repeat(MAX_LABEL + 1))).is_err());
    assert!(saying(&|p| p.base_branch = Some("develop".into())).is_ok());
    assert!(saying(&|p| p.base_branch = Some("--upload-pack=x".into())).is_err());
    assert!(saying(&|p| p.base_branch = Some("b".repeat(MAX_BRANCH + 1))).is_err());
    assert!(saying(&|p| p.existing_branch = Some("ys/kept".into())).is_ok());
    assert!(
        saying(&|p| {
            p.existing_branch = Some("ys/kept".into());
            p.base_branch = Some("main".into());
        })
        .is_err(),
        "an existing branch has no base"
    );
    assert!(
        saying(&|p| {
            p.existing_branch = Some("ys/kept".into());
            p.name = Some("kept".into());
        })
        .is_err(),
        "an existing branch names itself"
    );
    let session = |harness: Option<&str>| {
        Operation::SessionStart(SessionStartParams {
            workspace: WorkspaceId::new("w").unwrap(),
            harness: harness.map(str::to_owned),
            prompt: None,
        })
        .validate()
    };
    assert!(session(None).is_ok());
    assert!(session(Some("")).is_err());
    assert!(session(Some(&"h".repeat(MAX_LABEL + 1))).is_err());
    let resume = |prompt: Option<String>| {
        Operation::SessionResume(SessionResumeParams {
            session: SessionRecordId::new("s").unwrap(),
            prompt,
        })
        .validate()
    };
    assert!(resume(None).is_ok());
    assert!(resume(Some(String::new())).is_ok());
    assert!(resume(Some("x".repeat(MAX_PROMPT + 1))).is_err());

    let begin = |name: &str, size: u64| {
        Operation::UploadBegin(UploadBeginParams {
            workspace: WorkspaceId::new("w").unwrap(),
            name: name.into(),
            size,
        })
        .validate()
    };
    assert!(begin("photo.jpg", 1).is_ok());
    assert!(begin("photo.jpg", MAX_UPLOAD).is_ok());
    assert!(begin("photo.jpg", 0).is_err());
    assert!(begin("photo.jpg", MAX_UPLOAD + 1).is_err());
    assert!(begin("", 10).is_err());

    let chunk = |data: &str| {
        Operation::UploadChunk(UploadChunkParams {
            upload: 1,
            offset: 0,
            data: data.into(),
        })
        .validate()
    };
    assert!(chunk("aGk=").is_ok());
    assert!(chunk("aGk").is_err(), "unpadded");
    assert!(chunk("a=Gk").is_err(), "padding inside");
    assert!(chunk("").is_err());
    let largest = "A".repeat(MAX_UPLOAD_CHUNK.div_ceil(3) * 4);
    assert!(chunk(&largest).is_ok());
    assert!(chunk(&format!("{largest}AAAA")).is_err());
    // The largest piece's request is well within a frame.
    let request = Request {
        request_id: RequestId::new("r-1").unwrap(),
        protocol: ProtocolVersion::CURRENT,
        operation: Operation::UploadChunk(UploadChunkParams {
            upload: u32::MAX,
            offset: MAX_UPLOAD,
            data: largest,
        }),
    };
    assert!(serde_json::to_vec(&request).unwrap().len() < MAX_CONTROL_FRAME);
}

#[test]
fn looking_after_projects_and_workspaces_is_bounded() {
    let clone = |remote: &str, name: Option<&str>| {
        Operation::ProjectClone(ProjectCloneParams {
            remote: remote.into(),
            name: name.map(str::to_owned),
        })
        .validate()
    };
    assert!(clone("https://github.com/example/app.git", None).is_ok());
    assert!(clone("git@github.com:example/app.git", Some("app")).is_ok());
    assert!(clone("", None).is_err());
    assert!(clone("--upload-pack=touch /tmp/x", None).is_err());
    assert!(clone("https://example.com/a b", None).is_err());
    assert!(clone(
        &format!("https://example.com/{}", "a".repeat(MAX_REMOTE)),
        None
    )
    .is_err());
    assert!(clone("https://example.com/a", Some("../up")).is_err());
    assert!(clone(
        "https://example.com/a",
        Some(&"n".repeat(MAX_PROJECT_NAME + 1))
    )
    .is_err());

    let rename = |name: &str| {
        Operation::WorkspaceRename(WorkspaceRenameParams {
            workspace: WorkspaceId::new("w").unwrap(),
            name: name.into(),
        })
        .validate()
    };
    assert!(rename("Login spacing").is_ok());
    assert!(rename(" ").is_err());
    assert!(rename(&"n".repeat(MAX_WORKSPACE_NAME + 1)).is_err());

    // A long list of long names still fits a frame: the default first, the rest counted out.
    let long = |n: usize| format!("{}{n:03}", "b".repeat(MAX_BRANCH - 3));
    let many: Vec<String> = (0..MAX_BRANCHES + 20).map(long).collect();
    let listed = manage::bounded_branches(many.clone(), Some(long(150)), many);
    assert_eq!(listed.branches.len(), MAX_BRANCHES);
    assert_eq!(listed.branches[0], long(150));
    assert!(listed.more);
    assert!(listed.checked_out.len() <= MAX_BRANCHES);
    let response = Response {
        request_id: Some(RequestId::new("r-1").unwrap()),
        outcome: Outcome::Result(Reply::Branches(listed)),
    };
    assert!(encode_response(&response).len() < MAX_CONTROL_FRAME);
    // A name no request could give back is not offered.
    let odd = manage::bounded_branches(vec!["main".into(), "-x".into()], None, vec![]);
    assert_eq!(odd.branches, ["main"]);
    assert!(odd.more);
}

#[test]
fn reading_changes_and_files_is_bounded_and_never_names_a_path_outside() {
    let read = |path: &str, old: Option<&str>, offset: u64| {
        Operation::FileRead(FileReadParams {
            workspace: WorkspaceId::new("w").unwrap(),
            path: path.into(),
            old_path: old.map(str::to_owned),
            of: FileOf::Working,
            offset,
        })
        .validate()
    };
    assert!(read("src/login.tsx", None, 0).is_ok());
    assert!(read(".env.example", Some("old name.txt"), MAX_FILE).is_ok());
    for outside in [
        "",
        "/etc/passwd",
        "../secrets",
        "src/../../secrets",
        "src//login.tsx",
        "src/",
        "C:\\Users\\me",
        "C:/Users/me",
        "\\\\server\\share",
        "-x",
        "a\u{0}b",
    ] {
        assert!(read(outside, None, 0).is_err(), "{outside:?}");
        assert!(read("ok", Some(outside), 0).is_err(), "{outside:?}");
    }
    assert!(read(&"p".repeat(MAX_PATH + 1), None, 0).is_err());
    assert!(read("a", None, MAX_FILE + 1).is_err());
    let files = |dir: &str| {
        Operation::WorkspaceFiles(WorkspaceFilesParams {
            workspace: WorkspaceId::new("w").unwrap(),
            dir: dir.into(),
        })
        .validate()
    };
    assert!(files("").is_ok(), "the root");
    assert!(files("src/auth").is_ok());
    assert!(files("../up").is_err());

    // A long list of long paths still fits a frame, the uncommitted first, and says it was cut.
    let file = |n: usize| ChangedFile {
        path: format!("{}/{n}", "d".repeat(MAX_PATH - 8)),
        old_path: None,
        kind: ChangeKind::Modified,
        additions: Some(u32::MAX),
        deletions: Some(u32::MAX),
    };
    let listed = look::bounded_changes(
        (0..100).map(file).collect(),
        (100..200).map(file).collect(),
        Some("main".into()),
    );
    assert!(listed.more);
    assert!(!listed.uncommitted.is_empty());
    assert!(listed.uncommitted.len() + listed.committed.len() < 200);
    let response = Response {
        request_id: Some(RequestId::new("r-1").unwrap()),
        outcome: Outcome::Result(Reply::Changes(listed)),
    };
    assert!(encode_response(&response).len() < MAX_CONTROL_FRAME);
    // A path no request could give back is left out, and counted.
    let odd = look::bounded_changes(
        vec![ChangedFile {
            path: "../outside".into(),
            old_path: None,
            kind: ChangeKind::Added,
            additions: None,
            deletions: None,
        }],
        vec![],
        None,
    );
    assert!(odd.uncommitted.is_empty() && odd.more);

    let entries = (0..2000)
        .map(|n| FileEntry {
            name: format!("{}{n:04}", "f".repeat(200)),
            dir: false,
        })
        .collect();
    let folder = look::bounded_files(entries);
    assert!(folder.more && !folder.entries.is_empty());
    let response = Response {
        request_id: Some(RequestId::new("r-1").unwrap()),
        outcome: Outcome::Result(Reply::Files(folder)),
    };
    assert!(encode_response(&response).len() < MAX_CONTROL_FRAME);

    // The largest piece of a file, as base64, is well within a frame.
    let piece = FileChunk {
        state: FileState::Text,
        size: MAX_FILE,
        offset: MAX_FILE,
        data: "A".repeat(MAX_FILE_CHUNK.div_ceil(3) * 4),
        done: false,
    };
    let response = Response {
        request_id: Some(RequestId::new("r-1").unwrap()),
        outcome: Outcome::Result(Reply::File(piece)),
    };
    assert!(encode_response(&response).len() < MAX_CONTROL_FRAME);
    assert_eq!(look::not_sent(FileState::Absent, 0).data, "");
}

#[test]
fn committing_and_pull_requests_are_bounded() {
    use publish::*;
    let workspace = || WorkspaceId::new("w").unwrap();
    let commit = |message: &str| {
        Operation::WorkspaceCommit(WorkspaceCommitParams {
            workspace: workspace(),
            message: message.into(),
        })
        .validate()
    };
    assert!(commit("Fix it\n\nWith a body.").is_ok());
    assert!(commit("  \n").is_err());
    assert!(commit("a\0b").is_err());
    assert!(commit(&"m".repeat(MAX_COMMIT_MESSAGE + 1)).is_err());
    let open = |title: &str, body: &str| {
        Operation::PullRequestOpen(PullRequestOpenParams {
            workspace: workspace(),
            title: title.into(),
            body: body.into(),
            draft: false,
        })
        .validate()
    };
    assert!(open("Fix the spacing", "").is_ok());
    assert!(open(" ", "").is_err());
    assert!(
        open("--web", "").is_err(),
        "nothing that reads as an option"
    );
    assert!(open("two\nlines", "").is_err());
    assert!(open(&"t".repeat(MAX_PULL_REQUEST_TITLE + 1), "").is_err());
    assert!(open("ok", &"b".repeat(MAX_PULL_REQUEST_BODY + 1)).is_err());
    let merge = |number: u32, oid: &str| {
        Operation::PullRequestMerge(PullRequestMergeParams {
            workspace: workspace(),
            number,
            head_oid: oid.into(),
            method: MergeMethod::Squash,
        })
        .validate()
    };
    assert!(merge(91, "3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a").is_ok());
    assert!(merge(0, "3f2a1b0").is_err());
    assert!(
        merge(91, "").is_err(),
        "the head the person saw must be named"
    );
    assert!(merge(91, "HEAD").is_err());
    assert!(merge(91, "--admin").is_err());
    assert!(merge(91, &"a".repeat(MAX_OID + 1)).is_err());
    // A state from a newer host reads as unknown, and a state with nothing known is small.
    let read: PullRequest = serde_json::from_value(serde_json::json!({
        "number": 1, "url": "u", "title": "t", "state": "queued", "checks": "skipped"
    }))
    .unwrap();
    assert_eq!(
        (read.state, read.checks),
        (PullRequestState::Unknown, Checks::Unknown)
    );
    assert_eq!(
        serde_json::to_value(PublishState::default()).unwrap(),
        serde_json::json!({"ahead": 0, "behind": 0})
    );
}

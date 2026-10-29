//! The relay over a real socket, on each store: a phone reaches its host and nothing else does,
//! revocation closes a live socket, a slow phone is let go without holding anyone up, and a host
//! that leaves or comes back takes its phones' sockets with it.
//!
//! The host here is the test itself, holding the host's link the way the app does: it sees each
//! phone as a channel, and answers on it.

mod common;

use std::time::{Duration, Instant};

use common::served::*;
use remote_client::{Client, ClientError, Incoming, LinkReader, RelayLink};
use remote_proto::account::{Credentials, DeviceKind};
use remote_proto::pairing::{GrantSummary, HostSummary, Offer};
use remote_proto::relay::{CloseReason, LinkFrame, TicketRequest, MAX_QUEUED};
use remote_proto::{ErrorCode, GrantId, HostId};
use serde::de::DeserializeOwned;
use serde_json::json;
use yardsort_server::records::{MemoryRecords, Records};

macro_rules! for_each_store {
    ($($name:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::MemoryRecords::default()).await;
                }
            )*
        }
        mod postgres {
            $(
                #[tokio::test]
                async fn $name() {
                    if let Some(db) = super::common::pg::database().await {
                        let records = yardsort_server::postgres::PgRecords::connect_with(
                            db.config.clone(),
                        )
                        .await
                        .unwrap();
                        super::$name(records).await;
                        db.drop().await;
                    }
                }
            )*
        }
    };
}

for_each_store!(
    a_phone_reaches_its_host_and_nothing_else_does,
    revoking_closes_a_live_socket,
);

#[tokio::test]
async fn a_slow_phone_is_let_go_and_nobody_else_waits_for_it() {
    a_slow_phone(MemoryRecords::default()).await;
}

#[tokio::test]
async fn a_host_that_leaves_or_comes_back_takes_its_phones_with_it() {
    host_comes_and_goes(MemoryRecords::default()).await;
}

#[tokio::test]
async fn a_socket_that_breaks_the_rules_is_closed() {
    rule_breakers(MemoryRecords::default()).await;
}

/// Long enough for anything the relay does at once; well under its five-second recheck, so a
/// close that arrives in time came from the nudge.
const SOON: Duration = Duration::from_secs(3);

async fn ok<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

/// A host on `subject`'s account and a phone it approved, to watch and type everywhere.
struct Pair {
    host: Credentials,
    host_id: HostId,
    phone: Credentials,
    grant: GrantId,
}

async fn host(s: &Served, subject: &str) -> (Credentials, HostId) {
    let host = s.sign_in(subject, "Workstation", DeviceKind::Host).await;
    let summary: HostSummary = ok(s
        .post_as(
            &host.access,
            "v1/hosts",
            Some(&json!({"name": "Workstation"})),
        )
        .await)
    .await;
    (host, summary.id)
}

async fn pair(s: &Served, subject: &str) -> Pair {
    let (host, host_id) = host(s, subject).await;
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let phone = s.sign_in(subject, "Phone", DeviceKind::Phone).await;
    let asked: GrantSummary = ok(s
        .post_as(
            &phone.access,
            "v1/pairings",
            Some(&json!({
                "offer": {"kind": "code", "value": offer.code.expose()},
                "scopes": ["workspace:read", "terminal:read", "terminal:control"],
            })),
        )
        .await)
    .await;
    let _: GrantSummary = ok(s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({
                "scopes": ["workspace:read", "terminal:read", "terminal:control"],
                "projects": {"kind": "all"},
            })),
        )
        .await)
    .await;
    Pair {
        host,
        host_id,
        phone,
        grant: asked.id,
    }
}

fn client(s: &Served) -> Client {
    Client::new(s.base.as_str()).unwrap()
}

async fn link(s: &Served, access: &Credentials, request: TicketRequest) -> RelayLink {
    client(s)
        .open_relay(&access.access, &request)
        .await
        .unwrap()
}

async fn host_link(s: &Served, pair: &Pair) -> RelayLink {
    link(
        s,
        &pair.host,
        TicketRequest::Host {
            host: pair.host_id.clone(),
        },
    )
    .await
}

async fn phone_link(s: &Served, pair: &Pair) -> RelayLink {
    link(
        s,
        &pair.phone,
        TicketRequest::Device {
            host: pair.host_id.clone(),
        },
    )
    .await
}

/// The next link frame a host receives, as owned parts.
#[derive(Debug, PartialEq, Eq)]
enum Heard {
    Open(u32, String, String),
    Close(u32, CloseReason),
    Text(u32, String),
    Binary(u32, Vec<u8>),
}

async fn heard(host: &mut LinkReader) -> Heard {
    let message = tokio::time::timeout(SOON, host.next())
        .await
        .expect("the host heard nothing")
        .expect("the host's link closed");
    let Incoming::Binary(bytes) = message else {
        panic!("a host's link carries only link frames: {message:?}");
    };
    match LinkFrame::decode(&bytes).unwrap() {
        LinkFrame::Open(c, open) => Heard::Open(c, open.device.to_string(), open.grant.to_string()),
        LinkFrame::Close(c, reason) => Heard::Close(c, reason),
        LinkFrame::Text(c, text) => Heard::Text(c, String::from_utf8(text.to_vec()).unwrap()),
        LinkFrame::Binary(c, bytes) => Heard::Binary(c, bytes.to_vec()),
    }
}

async fn closed(reader: &mut LinkReader) -> CloseReason {
    tokio::time::timeout(SOON, async {
        loop {
            if let Err(reason) = reader.next().await {
                return reason;
            }
        }
    })
    .await
    .expect("the socket stayed open")
}

fn refused(result: Result<RelayLink, ClientError>) -> ErrorCode {
    match result {
        Err(error) => error.code().unwrap_or_else(|| panic!("{error}")),
        Ok(_) => panic!("let through"),
    }
}

async fn a_phone_reaches_its_host_and_nothing_else_does(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let p = pair(&s, "alice").await;
    let mut host = host_link(&s, &p).await;
    assert_eq!(host.host, p.host_id);
    let mut phone = phone_link(&s, &p).await;
    assert_eq!(phone.host, p.host_id);

    // The host is told who connected, under which grant, and they talk.
    let Heard::Open(channel, device, grant) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };
    assert_eq!(device, p.phone.device.to_string());
    assert_eq!(grant, p.grant.to_string());
    phone.writer.text(r#"{"hello":1}"#.into()).await.unwrap();
    assert_eq!(
        heard(&mut host.reader).await,
        Heard::Text(channel, r#"{"hello":1}"#.into())
    );
    host.writer
        .binary(LinkFrame::Text(channel, br#"{"reply":1}"#).encode())
        .await
        .unwrap();
    host.writer
        .binary(LinkFrame::Binary(channel, b"\x01\x01raw").encode())
        .await
        .unwrap();
    assert_eq!(
        phone.reader.next().await,
        Ok(Incoming::Text(r#"{"reply":1}"#.into()))
    );
    assert_eq!(
        phone.reader.next().await,
        Ok(Incoming::Binary(b"\x01\x01raw".to_vec()))
    );

    // A second phone of the account, never approved, gets no ticket; nor does another
    // account's, which cannot tell the host exists; nor a phone asking for a host's own link.
    let c = client(&s);
    let device = TicketRequest::Device {
        host: p.host_id.clone(),
    };
    let unpaired = s.sign_in("alice", "Tablet", DeviceKind::Phone).await;
    assert_eq!(
        refused(c.open_relay(&unpaired.access, &device).await),
        ErrorCode::Forbidden
    );
    let stranger = s.sign_in("mallory", "Phone", DeviceKind::Phone).await;
    assert_eq!(
        refused(c.open_relay(&stranger.access, &device).await),
        ErrorCode::NotFound
    );
    let (stranger_host, _) = self::host(&s, "mallory").await;
    for asking in [&stranger_host, &p.phone] {
        let as_host = TicketRequest::Host {
            host: p.host_id.clone(),
        };
        assert_eq!(
            refused(c.open_relay(&asking.access, &as_host).await),
            ErrorCode::NotFound
        );
    }

    // A ticket works once, and only as a ticket.
    let ticket = c.ticket(&p.phone.access, &device).await.unwrap();
    let again = ticket.ticket.clone();
    let mut second = c.open_relay_with(ticket.ticket).await.unwrap();
    assert!(matches!(heard(&mut host.reader).await, Heard::Open(..)));
    assert_eq!(
        refused(c.open_relay_with(again).await),
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        refused(c.open_relay_with(p.phone.access.clone()).await),
        ErrorCode::Unauthenticated
    );

    // Nothing sent on one phone's channel reaches another.
    host.writer
        .binary(LinkFrame::Text(channel, br#"{"only":"first"}"#).encode())
        .await
        .unwrap();
    assert_eq!(
        phone.reader.next().await,
        Ok(Incoming::Text(r#"{"only":"first"}"#.into()))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), second.reader.next())
            .await
            .is_err(),
        "the second phone heard the first one's traffic"
    );
}

async fn revoking_closes_a_live_socket(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let p = pair(&s, "alice").await;
    let mut host = host_link(&s, &p).await;
    let mut phone = phone_link(&s, &p).await;
    let Heard::Open(channel, ..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };

    // The host's person takes the grant back: the phone's socket closes at once, and the host
    // is told to drop the channel.
    let revoked = Instant::now();
    let response = s
        .post_as(
            &p.host.access,
            &format!("v1/grants/{}/revoke", p.grant),
            None,
        )
        .await;
    assert!(response.status().is_success());
    assert_eq!(closed(&mut phone.reader).await, CloseReason::Revoked);
    assert!(revoked.elapsed() < SOON);
    assert_eq!(
        heard(&mut host.reader).await,
        Heard::Close(channel, CloseReason::Revoked)
    );
    // And it gets no new ticket.
    let device = TicketRequest::Device {
        host: p.host_id.clone(),
    };
    assert_eq!(
        refused(client(&s).open_relay(&p.phone.access, &device).await),
        ErrorCode::Forbidden
    );

    // Signing a device out closes its socket too.
    let p = pair(&s, "bob").await;
    let mut host = host_link(&s, &p).await;
    let mut phone = phone_link(&s, &p).await;
    let Heard::Open(..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };
    let response = s
        .post_as(
            &p.host.access,
            &format!("v1/devices/{}/revoke", p.phone.device),
            None,
        )
        .await;
    assert!(response.status().is_success());
    assert_eq!(closed(&mut phone.reader).await, CloseReason::Revoked);

    // A host that stops being one loses its link, and its phones theirs.
    let p = pair(&s, "carol").await;
    let mut host = host_link(&s, &p).await;
    let mut phone = phone_link(&s, &p).await;
    let Heard::Open(..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };
    let response = s
        .post_as(
            &p.host.access,
            &format!("v1/hosts/{}/remove", p.host_id),
            None,
        )
        .await;
    assert!(response.status().is_success());
    assert_eq!(closed(&mut host.reader).await, CloseReason::Revoked);
    assert_eq!(closed(&mut phone.reader).await, CloseReason::Revoked);
}

async fn a_slow_phone(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let p = pair(&s, "alice").await;
    let mut host = host_link(&s, &p).await;
    // The slow one never reads.
    let slow = phone_link(&s, &p).await;
    let Heard::Open(slow_channel, ..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };
    let mut quick = phone_link(&s, &p).await;
    let Heard::Open(quick_channel, ..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };

    // Far more than the slow phone's queue and its socket's buffers can hold.
    let chunk = vec![b'x'; 60 * 1024];
    let mut sent = 0;
    let let_go = loop {
        host.writer
            .binary(LinkFrame::Binary(slow_channel, &chunk).encode())
            .await
            .unwrap();
        sent += chunk.len();
        assert!(sent < 64 * MAX_QUEUED, "the slow phone was never let go");
        if let Ok(Ok(Incoming::Binary(bytes))) =
            tokio::time::timeout(Duration::from_millis(1), host.reader.next()).await
        {
            break LinkFrame::decode(&bytes).unwrap().encode();
        }
    };
    assert_eq!(
        LinkFrame::decode(&let_go).unwrap(),
        LinkFrame::Close(slow_channel, CloseReason::Overloaded)
    );
    drop(slow);

    // The host and the other phone carried on.
    host.writer
        .binary(LinkFrame::Text(quick_channel, br#"{"still":"here"}"#).encode())
        .await
        .unwrap();
    assert_eq!(
        quick.reader.next().await,
        Ok(Incoming::Text(r#"{"still":"here"}"#.into()))
    );
}

async fn host_comes_and_goes(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let p = pair(&s, "alice").await;
    let c = client(&s);
    let device = TicketRequest::Device {
        host: p.host_id.clone(),
    };
    // Not connected yet.
    assert_eq!(
        refused(c.open_relay(&p.phone.access, &device).await),
        ErrorCode::HostOffline
    );

    let mut first = host_link(&s, &p).await;
    let mut phone = phone_link(&s, &p).await;
    assert!(matches!(heard(&mut first.reader).await, Heard::Open(..)));
    // The host connects again, as after its network changed: the old link is told it was
    // replaced, and the phone that came through it reconnects to the new one.
    let mut second = host_link(&s, &p).await;
    assert_eq!(closed(&mut first.reader).await, CloseReason::Replaced);
    assert_eq!(closed(&mut phone.reader).await, CloseReason::HostOffline);
    let mut phone = phone_link(&s, &p).await;
    assert!(matches!(heard(&mut second.reader).await, Heard::Open(..)));

    // The host goes away: its phone is told.
    second.writer.close().await;
    assert_eq!(closed(&mut phone.reader).await, CloseReason::HostOffline);
}

async fn rule_breakers(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let p = pair(&s, "alice").await;
    let mut host = host_link(&s, &p).await;

    // A phone may send only text.
    let mut phone = phone_link(&s, &p).await;
    let Heard::Open(channel, ..) = heard(&mut host.reader).await else {
        panic!("no channel opened");
    };
    phone.writer.binary(b"\x01".to_vec()).await.unwrap();
    assert_eq!(closed(&mut phone.reader).await, CloseReason::Invalid);
    assert_eq!(
        heard(&mut host.reader).await,
        Heard::Close(channel, CloseReason::Gone)
    );

    // A host may send only link frames, and never opens a channel itself.
    host.writer
        .binary(
            LinkFrame::Open(
                9,
                remote_proto::relay::ChannelOpen {
                    device: p.phone.device.clone(),
                    grant: p.grant.clone(),
                },
            )
            .encode(),
        )
        .await
        .unwrap();
    assert_eq!(closed(&mut host.reader).await, CloseReason::Invalid);
}

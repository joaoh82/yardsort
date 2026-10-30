//! The relay on two instances of the server sharing their records, behind a stand-in for Fly.io's
//! proxy (`common/replay.rs`), on each store: a phone reaches its host whichever instance its
//! requests land on, a host that connects again on the other instance takes its link with it, a
//! revocation through one instance closes a socket held by the other, a place nobody keeps is
//! not believed, and a host hears of a device asking to pair through the other instance: at the
//! other's next round, and at once through the records' signals, as a host's old link is closed.
//! The two also share their rate limits.
//!
//! Everyone signs in and pairs at instance `a` itself; only the relay goes through the proxy. Each
//! test also holds the records the two share, to look at them as a third instance would.

mod common;

use std::time::Duration;

use common::replay::ReplayProxy;
use common::served::*;
use remote_client::{Client, Incoming, LinkReader, RelayLink};
use remote_proto::account::{Credentials, DeviceKind};
use remote_proto::pairing::{GrantSummary, HostSummary, Offer};
use remote_proto::relay::{CloseReason, HostNotice, LinkFrame, TicketRequest, NOTICE_CHANNEL};
use remote_proto::{ErrorCode, GrantId, HostId};
use serde::de::DeserializeOwned;
use serde_json::json;
use yardsort_server::http;
use yardsort_server::records::{LinkPlace, MemoryRecords, Records};
use yardsort_server::relay::{Instance, Relay, Routing, PLACE_LAPSES_AFTER_MS, RECHECK_EVERY};

macro_rules! for_each_store {
    ($($name:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test]
                async fn $name() {
                    let records = super::MemoryRecords::default();
                    super::$name(records.clone(), records.clone(), records).await;
                }
            )*
        }
        mod postgres {
            $(
                #[tokio::test]
                async fn $name() {
                    if let Some(db) = super::common::pg::database().await {
                        let connect = || yardsort_server::postgres::PgRecords::connect_with(
                            db.config.clone(),
                        );
                        super::$name(
                            connect().await.unwrap(),
                            connect().await.unwrap(),
                            connect().await.unwrap(),
                        )
                        .await;
                        db.drop().await;
                    }
                }
            )*
        }
    };
}

for_each_store!(
    a_phone_reaches_its_host_whichever_instance_it_lands_on,
    a_host_that_connects_again_elsewhere_takes_its_link_with_it,
    a_place_nobody_keeps_is_not_believed,
    a_host_hears_of_a_device_asking_through_the_other_instance,
    what_happens_through_one_instance_reaches_the_other_at_once,
    a_host_link_taken_elsewhere_closes_at_once,
    two_instances_share_one_rate_limit,
    a_ticket_request_sent_on_counts_once,
);

/// Long enough for anything done at once.
const SOON: Duration = Duration::from_secs(3);

/// Long enough for the other instance's next round of checks.
fn next_round() -> Duration {
    RECHECK_EVERY + SOON
}

struct Instances {
    a: Served,
    b: Served,
    proxy: ReplayProxy,
}

async fn instances(a: impl Records + 'static, b: impl Records + 'static) -> Instances {
    instances_with_rounds(a, b, RECHECK_EVERY).await
}

/// With rounds of checks `rounds` apart.
async fn instances_with_rounds(
    a: impl Records + 'static,
    b: impl Records + 'static,
    rounds: Duration,
) -> Instances {
    instances_with(a, b, rounds, http::Options::default).await
}

/// With rounds of checks `rounds` apart, each instance with `options`.
async fn instances_with(
    a: impl Records + 'static,
    b: impl Records + 'static,
    rounds: Duration,
    options: impl Fn() -> http::Options,
) -> Instances {
    let relay =
        |name: &str| Relay::with_rounds(Instance::new(name).unwrap(), Routing::FlyReplay, rounds);
    let a = Served::start_as(a, options(), relay("a")).await;
    let b = Served::start_as(b, options(), relay("b")).await;
    let addr = |s: &Served| s.base.socket_addrs(|| None).unwrap()[0];
    let proxy = ReplayProxy::start(vec![("a".into(), addr(&a)), ("b".into(), addr(&b))]).await;
    Instances { a, b, proxy }
}

impl Instances {
    /// The relay's way in, through the proxy.
    fn client(&self) -> Client {
        Client::new(&self.proxy.base()).unwrap()
    }

    async fn host_link(&self, pair: &Pair) -> RelayLink {
        let request = TicketRequest::Host {
            host: pair.host_id.clone(),
        };
        self.client()
            .open_relay(&pair.host.access, &request)
            .await
            .unwrap()
    }

    async fn phone_link(&self, pair: &Pair) -> RelayLink {
        self.client()
            .open_relay(&pair.phone.access, &device(pair))
            .await
            .unwrap()
    }
}

fn device(pair: &Pair) -> TicketRequest {
    TicketRequest::Device {
        host: pair.host_id.clone(),
    }
}

async fn ok<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

struct Pair {
    host: Credentials,
    host_id: HostId,
    phone: Credentials,
    grant: GrantId,
}

/// A host and a phone it approved, signed in and paired at `s`.
async fn pair(s: &Served) -> Pair {
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let summary: HostSummary = ok(s
        .post_as(
            &host.access,
            "v1/hosts",
            Some(&json!({"name": "Workstation"})),
        )
        .await)
    .await;
    let host_id = summary.id;
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;
    let scopes = ["workspace:read", "terminal:read"];
    let asked: GrantSummary = ok(s
        .post_as(
            &phone.access,
            "v1/pairings",
            Some(&json!({
                "offer": {"kind": "code", "value": offer.code.expose()},
                "scopes": scopes,
            })),
        )
        .await)
    .await;
    let _: GrantSummary = ok(s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({"scopes": scopes, "projects": {"kind": "all"}})),
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

/// The channel a host was just told of.
async fn opened(host: &mut LinkReader) -> u32 {
    let message = tokio::time::timeout(SOON, host.next())
        .await
        .expect("the host heard nothing")
        .expect("the host's link closed");
    let Incoming::Binary(bytes) = message else {
        panic!("a host's link carries only link frames: {message:?}");
    };
    match LinkFrame::decode(&bytes).unwrap() {
        LinkFrame::Open(channel, _) => channel,
        other => panic!("not an open: {other:?}"),
    }
}

/// The phone says something and the host hears it, then the other way round.
async fn they_talk(host: &mut RelayLink, phone: &mut RelayLink) {
    let channel = opened(&mut host.reader).await;
    phone.writer.text(r#"{"hello":1}"#.into()).await.unwrap();
    let heard = tokio::time::timeout(SOON, host.reader.next())
        .await
        .unwrap()
        .unwrap();
    let Incoming::Binary(bytes) = heard else {
        panic!("{heard:?}");
    };
    assert_eq!(
        LinkFrame::decode(&bytes).unwrap(),
        LinkFrame::Text(channel, br#"{"hello":1}"#)
    );
    host.writer
        .binary(LinkFrame::Text(channel, br#"{"reply":1}"#).encode())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(SOON, phone.reader.next())
            .await
            .unwrap(),
        Ok(Incoming::Text(r#"{"reply":1}"#.into()))
    );
}

async fn closed_within(wait: Duration, reader: &mut LinkReader) -> CloseReason {
    tokio::time::timeout(wait, async {
        loop {
            if let Err(reason) = reader.next().await {
                return reason;
            }
        }
    })
    .await
    .expect("the socket stayed open")
}

async fn a_phone_reaches_its_host_whichever_instance_it_lands_on(
    a: impl Records + 'static,
    b: impl Records + 'static,
    records: impl Records,
) {
    let i = instances(a, b).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("a"));
    let mut host = i.host_link(&p).await;
    assert!(i.a.relay.connected(&p.host_id));
    assert_eq!(i.proxy.replays(), 0, "a host's link is wherever it lands");
    let place = records.link_place(&p.host_id).await.unwrap().unwrap();
    assert_eq!(place.instance, "a");

    // Everything the phone sends lands on b: the ticket request is sent on to a, whose ticket
    // names it, and so is the socket.
    i.proxy.land_on(Some("b"));
    let ticket = i
        .client()
        .ticket(&p.phone.access, &device(&p))
        .await
        .unwrap();
    assert_eq!(ticket.route.as_deref(), Some("a"));
    assert_eq!(i.proxy.replays(), 1);
    let mut phone = i.client().open_relay_with(ticket).await.unwrap();
    assert_eq!(i.proxy.replays(), 2);
    assert_eq!(phone.host, p.host_id);
    they_talk(&mut host, &mut phone).await;

    // Revoked through b, which holds nothing of it: a closes the socket.
    let revoked =
        i.b.post_as(
            &p.host.access,
            &format!("v1/grants/{}/revoke", p.grant),
            None,
        )
        .await;
    assert!(revoked.status().is_success(), "{}", revoked.status());
    assert_eq!(
        closed_within(next_round(), &mut phone.reader).await,
        CloseReason::Revoked
    );
}

async fn a_host_that_connects_again_elsewhere_takes_its_link_with_it(
    a: impl Records + 'static,
    b: impl Records + 'static,
    records: impl Records,
) {
    let i = instances(a, b).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("a"));
    let mut first = i.host_link(&p).await;
    i.proxy.land_on(Some("b"));
    let mut phone = i.phone_link(&p).await;
    opened(&mut first.reader).await;

    // The host connects again, landing on b. a lets its old link go at its next round, and the
    // phone on it with it.
    let mut second = i.host_link(&p).await;
    assert!(i.b.relay.connected(&p.host_id));
    assert_eq!(
        closed_within(next_round(), &mut first.reader).await,
        CloseReason::Replaced
    );
    assert_eq!(
        closed_within(SOON, &mut phone.reader).await,
        CloseReason::HostOffline
    );
    assert!(!i.a.relay.connected(&p.host_id));

    // The phone comes back, landing on a this time, and is sent to b.
    i.proxy.land_on(Some("a"));
    let mut phone = i.phone_link(&p).await;
    they_talk(&mut second, &mut phone).await;

    // The old link's closing forgot nothing of the new one's place.
    let place = records.link_place(&p.host_id).await.unwrap();
    assert_eq!(place.map(|p| p.instance), Some("b".into()));
}

async fn a_place_nobody_keeps_is_not_believed(
    a: impl Records + 'static,
    b: impl Records + 'static,
    records: impl Records,
) {
    let i = instances(a, b).await;
    let p = pair(&i.a).await;
    let now = http::now_ms();
    let place = |instance: &str, seen_at| LinkPlace {
        host: p.host_id.clone(),
        instance: instance.into(),
        link: "an-old-link".into(),
        seen_at,
    };
    i.proxy.land_on(Some("b"));

    // An instance that is gone: the proxy cannot reach it and gives the request to another, which
    // answers it there and then, instead of sending it round again.
    records.put_link(place("gone", now)).await.unwrap();
    let ticket = i
        .client()
        .ticket(&p.phone.access, &device(&p))
        .await
        .unwrap();
    assert_eq!(ticket.route.as_deref(), Some("a"), "the proxy chose a");
    assert_eq!(i.proxy.replays(), 1);

    // A place its instance stopped keeping is not believed: b issues the ticket itself.
    records
        .put_link(place("a", now - PLACE_LAPSES_AFTER_MS - 1))
        .await
        .unwrap();
    let ticket = i
        .client()
        .ticket(&p.phone.access, &device(&p))
        .await
        .unwrap();
    assert_eq!(ticket.route.as_deref(), Some("b"));
    assert_eq!(i.proxy.replays(), 1, "not sent on");

    // Either way the host is not connected, which is what the phone is told.
    let refused = i.client().open_relay_with(ticket).await.err().unwrap();
    assert_eq!(refused.code(), Some(ErrorCode::HostOffline));
}

async fn a_host_hears_of_a_device_asking_through_the_other_instance(
    a: impl Records + 'static,
    b: impl Records + 'static,
    records: impl Records,
) {
    let i = instances(a, b).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("a"));
    let mut host = i.host_link(&p).await;

    // A tablet redeems a code through b, which holds nothing of the host's link.
    let offer: Offer = ok(i
        .a
        .post_as(
            &p.host.access,
            &format!("v1/hosts/{}/offers", p.host_id),
            None,
        )
        .await)
    .await;
    let tablet = i.a.sign_in("alice", "Tablet", DeviceKind::Phone).await;
    let asked: GrantSummary = ok(i
        .b
        .post_as(
            &tablet.access,
            "v1/pairings",
            Some(&json!({
                "offer": {"kind": "code", "value": offer.code.expose()},
                "scopes": ["workspace:read"],
            })),
        )
        .await)
    .await;
    let pending = records.pending_on_links("a", http::now_ms()).await.unwrap();
    assert_eq!(pending, [(p.host_id.clone(), asked.id.clone())]);

    // a tells its host at its next round.
    let heard = tokio::time::timeout(next_round(), host.reader.next())
        .await
        .expect("the host was not told")
        .unwrap();
    let Incoming::Binary(bytes) = heard else {
        panic!("{heard:?}");
    };
    let notice = serde_json::to_vec(&HostNotice::PairingRequested { grant: asked.id }).unwrap();
    assert_eq!(
        LinkFrame::decode(&bytes).unwrap(),
        LinkFrame::Text(NOTICE_CHANNEL, &notice)
    );
}

async fn what_happens_through_one_instance_reaches_the_other_at_once(
    a: impl Records + 'static,
    b: impl Records + 'static,
    _records: impl Records,
) {
    // A round a minute apart: only a signal gets anything there in time.
    let i = instances_with_rounds(a, b, Duration::from_secs(60)).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("a"));
    let mut host = i.host_link(&p).await;
    let mut phone = i.phone_link(&p).await;
    opened(&mut host.reader).await;

    // A tablet asks through b: the host on a hears of it at once.
    let offer: Offer = ok(i
        .a
        .post_as(
            &p.host.access,
            &format!("v1/hosts/{}/offers", p.host_id),
            None,
        )
        .await)
    .await;
    let tablet = i.a.sign_in("alice", "Tablet", DeviceKind::Phone).await;
    let asked: GrantSummary = ok(i
        .b
        .post_as(
            &tablet.access,
            "v1/pairings",
            Some(&json!({
                "offer": {"kind": "code", "value": offer.code.expose()},
                "scopes": ["workspace:read"],
            })),
        )
        .await)
    .await;
    let heard = tokio::time::timeout(SOON, host.reader.next())
        .await
        .expect("the host was not told at once")
        .unwrap();
    let notice = serde_json::to_vec(&HostNotice::PairingRequested { grant: asked.id }).unwrap();
    assert_eq!(
        heard,
        Incoming::Binary(LinkFrame::Text(NOTICE_CHANNEL, &notice).encode())
    );

    // The phone's grant revoked through b: its socket on a closes at once.
    let revoked =
        i.b.post_as(
            &p.host.access,
            &format!("v1/grants/{}/revoke", p.grant),
            None,
        )
        .await;
    assert!(revoked.status().is_success(), "{}", revoked.status());
    assert_eq!(
        closed_within(SOON, &mut phone.reader).await,
        CloseReason::Revoked
    );

    // A copy of the host's refresh credential presented through b after the host traded it in:
    // the host is signed out, and its link on a closes at once.
    let refresh = |credential: &remote_proto::account::Secret| json!({"grant": "refresh", "refresh": credential.expose()});
    let rotated =
        i.b.client
            .post(i.b.url("v1/token"))
            .json(&refresh(&p.host.refresh))
            .send()
            .await
            .unwrap();
    assert!(rotated.status().is_success(), "{}", rotated.status());
    // The host has its new credentials, and uses them: what comes next is not a lost answer.
    let rotated: remote_proto::account::Credentials = rotated.json().await.unwrap();
    let used = i.b.get_as(&rotated.access, "v1/devices").await;
    assert!(used.status().is_success(), "{}", used.status());
    let copy =
        i.b.client
            .post(i.b.url("v1/token"))
            .json(&refresh(&p.host.refresh))
            .send()
            .await
            .unwrap();
    assert_eq!(copy.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        closed_within(SOON, &mut host.reader).await,
        CloseReason::Revoked
    );
}

async fn a_host_link_taken_elsewhere_closes_at_once(
    a: impl Records + 'static,
    b: impl Records + 'static,
    _records: impl Records,
) {
    // A round a minute apart: only a signal gets anything there in time.
    let i = instances_with_rounds(a, b, Duration::from_secs(60)).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("a"));
    let mut first = i.host_link(&p).await;
    let mut phone = i.phone_link(&p).await;
    opened(&mut first.reader).await;

    // The host connects again, landing on b: a lets its old link go at once, and the phone on it.
    i.proxy.land_on(Some("b"));
    let mut second = i.host_link(&p).await;
    assert_eq!(
        closed_within(SOON, &mut first.reader).await,
        CloseReason::Replaced
    );
    assert_eq!(
        closed_within(SOON, &mut phone.reader).await,
        CloseReason::HostOffline
    );

    // The phone comes back, landing on a, and is sent to the new link on b.
    i.proxy.land_on(Some("a"));
    let mut phone = i.phone_link(&p).await;
    they_talk(&mut second, &mut phone).await;
}

async fn two_instances_share_one_rate_limit(
    a: impl Records + 'static,
    b: impl Records + 'static,
    _records: impl Records,
) {
    let options = || http::Options {
        client_ip_header: Some("fly-client-ip".parse().unwrap()),
        tokens_per_minute: 3,
        ..http::Options::default()
    };
    let a = Served::start_with(a, options()).await;
    let b = Served::start_with(b, options()).await;
    let forged = json!({"grant": "refresh", "refresh": "ys1.rt.not-one-the-server-issued"});
    let token = |s: &Served, from: &'static str| {
        s.client
            .post(s.url("v1/token"))
            .header("fly-client-ip", from)
            .json(&forged)
            .send()
    };
    let status = |r: reqwest::Result<reqwest::Response>| r.unwrap().status();
    // Three a minute from one address, wherever its requests land.
    assert_eq!(status(token(&a, "192.0.2.1").await), 401);
    assert_eq!(status(token(&a, "192.0.2.1").await), 401);
    assert_eq!(status(token(&b, "192.0.2.1").await), 401);
    let refused = token(&b, "192.0.2.1").await.unwrap();
    assert_eq!(refused.status(), 429);
    assert_eq!(refused.headers()["retry-after"], "20");
    // a has counted only two of its own, and still refuses: the third was b's.
    assert_eq!(status(token(&a, "192.0.2.1").await), 429);
    // Another address is counted on its own.
    assert_eq!(status(token(&b, "192.0.2.2").await), 401);
}

async fn a_ticket_request_sent_on_counts_once(
    a: impl Records + 'static,
    b: impl Records + 'static,
    _records: impl Records,
) {
    // Everyone's requests come from the proxy's address, so all count against one limit.
    let options = || http::Options {
        tickets_per_minute: 3,
        ..http::Options::default()
    };
    let i = instances_with(a, b, RECHECK_EVERY, options).await;
    let p = pair(&i.a).await;
    i.proxy.land_on(Some("b"));
    let _host = i.host_link(&p).await;

    // The phone's requests land on a, which sends them on to b: each counts once, at b.
    i.proxy.land_on(Some("a"));
    let replays = i.proxy.replays();
    for _ in 0..2 {
        i.client()
            .ticket(&p.phone.access, &device(&p))
            .await
            .expect("a ticket within the limit");
    }
    assert_eq!(i.proxy.replays(), replays + 2, "sent on from a");
    let refused = i.client().ticket(&p.phone.access, &device(&p)).await;
    assert!(
        matches!(&refused, Err(remote_client::ClientError::Remote(e)) if e.code == ErrorCode::RateLimited),
        "{refused:?}"
    );
}

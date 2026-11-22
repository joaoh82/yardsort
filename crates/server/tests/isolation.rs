//! Isolation, on each store: every route the server serves, asked by another account's devices
//! with one account's ids, answers nothing of that account and changes nothing of it (46 §13).
//!
//! The routes are read from `src/http.rs` itself, so a route added there without a probe here, or
//! a reason it needs none, fails this file.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use common::served::*;
use remote_proto::account::{Credentials, DeviceKind, Secret};
use remote_proto::pairing::{GrantSummary, HostSummary, Offer};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use yardsort_server::push::{PushOutcome, PushProvider, RecordingProvider};
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

for_each_store!(another_account_gets_nothing_of_one_from_any_route);

/// Routes no probe asks with another account's ids, and why none is needed.
const UNPROBED: &[(&str, &str)] = &[
    ("/health", "says ok, and nothing else"),
    ("/v1/server", "describes the server, the same to everyone"),
    (
        "/v1/login",
        "starts a sign-in; whose account it is, the provider says",
    ),
    (
        "/v1/login/callback",
        "the provider's answer, bound to the sign-in that started it",
    ),
    (
        "/v1/token",
        "takes secrets, never an id: each opens only its own device",
    ),
    (
        "/v1/relay",
        "opens only with a ticket, and tickets are probed here; tests/relay.rs proves the socket",
    ),
    (
        "/account/delete",
        "the page deletes whoever signs in at the provider: tests/delete.rs",
    ),
    (
        "/account/delete/start",
        "the page deletes whoever signs in at the provider: tests/delete.rs",
    ),
    (
        "/account/delete/confirm",
        "the page deletes whoever signs in at the provider: tests/delete.rs",
    ),
];

/// Every route `src/http.rs` serves, as it names them.
fn served_routes() -> BTreeSet<String> {
    let source = include_str!("../src/http.rs");
    let mut routes = BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find(".route(") {
        rest = &rest[at + ".route(".len()..];
        let start = rest.find('"').unwrap() + 1;
        let end = start + rest[start..].find('"').unwrap();
        routes.insert(rest[start..end].to_string());
        rest = &rest[end..];
    }
    routes
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

fn sid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Everything of Alice's that must never reach Bob: ids, names and words.
struct Alice {
    host: Credentials,
    phone: Credentials,
    host_id: String,
    project: String,
    note: String,
    grant: String,
    notice: String,
    offer: Offer,
    /// Every string of hers whose appearance in an answer to Bob is a leak.
    marks: Vec<String>,
    /// The ones Bob could not have made up: her names and words, not her ids, which a probe
    /// hands him.
    words: Vec<String>,
}

const ALICE: &str = "alice-7f3a";
const BOB: &str = "bob-91c2";

async fn alice(s: &Served) -> Alice {
    let host = s
        .sign_in(ALICE, "Alice's workstation", DeviceKind::Host)
        .await;
    let phone = s.sign_in(ALICE, "Alice's phone", DeviceKind::Phone).await;
    let registered: HostSummary = ok(s
        .post_as(
            &host.access,
            "v1/hosts",
            Some(&json!({ "name": "Alice's workstation" })),
        )
        .await)
    .await;
    let host_id = registered.id.to_string();

    // A grant her phone asked for and her host approved, and an offer left open.
    let asked_with: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let grant: GrantSummary = ok(s
        .post_as(
            &phone.access,
            "v1/pairings",
            Some(&json!({
                "offer": { "kind": "secret", "value": asked_with.secret.expose() },
                "scopes": ["workspace:read"],
            })),
        )
        .await)
    .await;
    let _: GrantSummary = ok(s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/approve", grant.id),
            Some(&json!({ "scopes": ["workspace:read"], "projects": { "kind": "all" } })),
        )
        .await)
    .await;
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;

    // A synced project with a note, linked on her host.
    let project = sid();
    let note = sid();
    let pushed = s
        .post_as(
            &host.access,
            "v1/sync/push",
            Some(&json!({ "ops": [
                {
                    "op": sid(), "action": "put", "kind": "project", "id": project,
                    "project": project, "schema": 1, "body": { "text": "alices-project-words" },
                },
                {
                    "op": sid(), "action": "put", "kind": "note", "id": note,
                    "project": project, "schema": 1, "body": { "text": "alices-note-words" },
                },
            ] })),
        )
        .await;
    assert_eq!(pushed.status(), StatusCode::OK);
    let linked = s
        .post_as(
            &host.access,
            &format!("v1/sync/projects/{project}/link"),
            None,
        )
        .await;
    assert!(linked.status().is_success(), "{}", linked.status());

    // A notice from her host, a push route for her phone, and her preferences.
    let receipt: Value = ok(s
        .post_as(
            &host.access,
            "v1/notices",
            Some(&json!({ "eventKey": "agent:row-1:finished:13", "category": "agent", "kind": "finished" })),
        )
        .await)
    .await;
    let notice = receipt["id"].as_str().unwrap().to_string();
    let registered = s
        .post_as(
            &phone.access,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "alices-push-token" })),
        )
        .await;
    assert_eq!(registered.status(), StatusCode::NO_CONTENT);

    let words: Vec<String> = [
        ALICE,
        "Alice's workstation",
        "Alice's phone",
        "alices-project-words",
        "alices-note-words",
        "alices-push-token",
        offer.code.expose(),
        offer.secret.expose(),
        host.access.expose(),
        host.refresh.expose(),
        phone.access.expose(),
        phone.refresh.expose(),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut marks = words.clone();
    marks.extend([
        host_id.clone(),
        host.device.to_string(),
        phone.device.to_string(),
        project.clone(),
        note.clone(),
        grant.id.to_string(),
        notice.clone(),
    ]);
    Alice {
        host,
        phone,
        host_id,
        project,
        note,
        grant: grant.id.to_string(),
        notice,
        offer,
        marks,
        words,
    }
}

/// What the server holds of Alice, as her own phone exports it: everything but what moves by
/// itself (when a device was last seen, and the day's usage).
async fn holding(s: &Served, alice: &Alice) -> Value {
    let mut page: Value = ok(s.get_as(&alice.phone.access, "v1/account/export").await).await;
    fn still(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("lastSeenAt");
                map.remove("usage");
                map.values_mut().for_each(still);
            }
            Value::Array(items) => items.iter_mut().for_each(still),
            _ => {}
        }
    }
    still(&mut page);
    let grants: Value = ok(s
        .get_as(
            &alice.host.access,
            &format!("v1/hosts/{}/grants", alice.host_id),
        )
        .await)
    .await;
    let notice: Value = ok(s
        .get_as(
            &alice.phone.access,
            &format!("v1/notifications/{}", alice.notice),
        )
        .await)
    .await;
    json!({ "export": page, "grants": grants, "notice": notice })
}

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// Refused: the id is not Bob's, so for him it does not exist; or the route is not for his
    /// kind of device at all.
    Missing,
    /// Answered, with Bob's own and nothing of Alice's.
    Own,
}

struct Probe {
    route: &'static str,
    post: bool,
    path: String,
    who: Secret,
    body: Option<Value>,
    expect: Expect,
}

impl Probe {
    fn get(route: &'static str, path: String, who: &Secret, expect: Expect) -> Self {
        Self {
            route,
            post: false,
            path,
            who: who.clone(),
            body: None,
            expect,
        }
    }

    fn post(
        route: &'static str,
        path: String,
        who: &Secret,
        body: Option<Value>,
        expect: Expect,
    ) -> Self {
        Self {
            route,
            post: true,
            path,
            who: who.clone(),
            body,
            expect,
        }
    }
}

/// Ask each probe, and say every way an answer gave Alice away. `marks` are what may not appear
/// in an answer.
async fn ask(s: &Served, probes: &[Probe], marks: &[String], failures: &mut Vec<String>) {
    for probe in probes {
        let response = if probe.post {
            s.post_as(&probe.who, &probe.path, probe.body.as_ref())
                .await
        } else {
            s.get_as(&probe.who, &probe.path).await
        };
        let status = response.status();
        let text = response.text().await.unwrap();
        let method = if probe.post { "POST" } else { "GET" };
        let named = format!("{method} {} ({})", probe.path, probe.route);
        match probe.expect {
            Expect::Missing => {
                if status != StatusCode::NOT_FOUND && status != StatusCode::FORBIDDEN {
                    failures.push(format!("{named}: {status}, not refused: {text}"));
                }
            }
            Expect::Own => {
                if !status.is_success() {
                    failures.push(format!("{named}: {status}: {text}"));
                }
            }
        }
        for mark in marks {
            if text.contains(mark.as_str()) {
                failures.push(format!("{named}: the answer holds Alice's {mark:?}"));
            }
        }
    }
}

async fn another_account_gets_nothing_of_one_from_any_route(
    records: impl Records + Clone + 'static,
) {
    let push = Arc::new(RecordingProvider::new(PushOutcome::Sent));
    let s = Served::start_notifying(records, Arc::clone(&push) as Arc<dyn PushProvider>).await;
    let alice = alice(&s).await;
    let before = holding(&s, &alice).await;

    // Bob has a host and a phone of his own, paired, with a project of his own.
    let bob_host = s.sign_in(BOB, "Bob's host", DeviceKind::Host).await;
    let bob_phone = s.sign_in(BOB, "Bob's phone", DeviceKind::Phone).await;
    let _bobs: HostSummary = ok(s
        .post_as(
            &bob_host.access,
            "v1/hosts",
            Some(&json!({ "name": "Bob's host" })),
        )
        .await)
    .await;
    let bobs_project = sid();
    let pushed = s
        .post_as(
            &bob_host.access,
            "v1/sync/push",
            Some(&json!({ "ops": [{
                "op": sid(), "action": "put", "kind": "project", "id": bobs_project,
                "project": bobs_project, "schema": 1, "body": { "text": "bobs-project" },
            }] })),
        )
        .await;
    assert_eq!(pushed.status(), StatusCode::OK);
    let bobs_preferences: Value = ok(s
        .get_as(&bob_phone.access, "v1/notifications/preferences")
        .await)
    .await;

    let (bh, bp) = (&bob_host.access, &bob_phone.access);
    let (host, project, note, grant, notice) = (
        &alice.host_id,
        &alice.project,
        &alice.note,
        &alice.grant,
        &alice.notice,
    );
    let approval = json!({ "scopes": ["workspace:read"], "projects": { "kind": "all" } });
    use Expect::{Missing, Own};

    // First, what only reads, or names Alice's ids: every answer free of everything of hers.
    let reading = vec![
        Probe::get("/v1/devices", "v1/devices".into(), bp, Own),
        Probe::get("/v1/hosts", "v1/hosts".into(), bp, Own),
        Probe::get("/v1/hosts", "v1/hosts".into(), bh, Own),
        Probe::get(
            "/v1/hosts/{id}/grants",
            format!("v1/hosts/{host}/grants"),
            bh,
            Missing,
        ),
        Probe::get("/v1/grants", "v1/grants".into(), bp, Own),
        Probe::get("/v1/grants/{id}", format!("v1/grants/{grant}"), bp, Missing),
        Probe::get("/v1/grants/{id}", format!("v1/grants/{grant}"), bh, Missing),
        Probe::get(
            "/v1/notifications/preferences",
            "v1/notifications/preferences".into(),
            bp,
            Own,
        ),
        Probe::get(
            "/v1/notifications/{id}",
            format!("v1/notifications/{notice}"),
            bp,
            Missing,
        ),
        Probe::get("/v1/sync/pull", "v1/sync/pull?after=0".into(), bh, Own),
        Probe::get("/v1/sync/pull", "v1/sync/pull?after=0".into(), bp, Missing),
        Probe::get(
            "/v1/sync/snapshot",
            format!("v1/sync/snapshot?project={project}"),
            bh,
            Missing,
        ),
        Probe::get("/v1/sync/projects", "v1/sync/projects".into(), bh, Own),
        Probe::get("/v1/account/export", "v1/account/export".into(), bp, Own),
        Probe::get(
            "/v1/account/export",
            format!("v1/account/export?after={note}"),
            bp,
            Own,
        ),
        // Alice's ids handed to everything that acts on one: none of it is Bob's to act on.
        Probe::post(
            "/v1/devices/{id}/revoke",
            format!("v1/devices/{}/revoke", alice.phone.device),
            bp,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/devices/{id}/revoke",
            format!("v1/devices/{}/revoke", alice.host.device),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/hosts",
            "v1/hosts".into(),
            bh,
            Some(json!({ "host": host, "name": "Mine now" })),
            Missing,
        ),
        Probe::post(
            "/v1/hosts/{id}/offers",
            format!("v1/hosts/{host}/offers"),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/hosts/{id}/remove",
            format!("v1/hosts/{host}/remove"),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/pairings",
            "v1/pairings".into(),
            bp,
            Some(json!({
                "offer": { "kind": "secret", "value": alice.offer.secret.expose() },
                "scopes": ["workspace:read"],
            })),
            Missing,
        ),
        Probe::post(
            "/v1/pairings",
            "v1/pairings".into(),
            bp,
            Some(json!({
                "offer": { "kind": "code", "value": alice.offer.code.expose() },
                "scopes": ["workspace:read"],
            })),
            Missing,
        ),
        Probe::post(
            "/v1/grants/{id}/approve",
            format!("v1/grants/{grant}/approve"),
            bh,
            Some(approval),
            Missing,
        ),
        Probe::post(
            "/v1/grants/{id}/deny",
            format!("v1/grants/{grant}/deny"),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/grants/{id}/revoke",
            format!("v1/grants/{grant}/revoke"),
            bp,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/grants/{id}/revoke",
            format!("v1/grants/{grant}/revoke"),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/relay/tickets",
            "v1/relay/tickets".into(),
            bh,
            Some(json!({ "role": "host", "host": host })),
            Missing,
        ),
        Probe::post(
            "/v1/relay/tickets",
            "v1/relay/tickets".into(),
            bp,
            Some(json!({ "role": "device", "host": host })),
            Missing,
        ),
        // Bob's own, which must land on Bob alone.
        Probe::post(
            "/v1/notices",
            "v1/notices".into(),
            bh,
            Some(
                json!({ "eventKey": "agent:row-9:finished:2", "category": "agent", "kind": "finished" }),
            ),
            Own,
        ),
        Probe::post(
            "/v1/notifications/token",
            "v1/notifications/token".into(),
            bp,
            Some(json!({ "platform": "apns", "token": "bobs-push-token" })),
            Own,
        ),
        Probe::post(
            "/v1/notifications/preferences",
            "v1/notifications/preferences".into(),
            bp,
            Some(bobs_preferences),
            Own,
        ),
        Probe::post(
            "/v1/notifications/reachable",
            "v1/notifications/reachable".into(),
            bp,
            None,
            Own,
        ),
    ];
    let mut failures = Vec::new();
    ask(&s, &reading, &alice.marks, &mut failures).await;

    // Then sync handed Alice's own ids: whatever Bob makes of them is his, so his answers may
    // name the ids he gave, but never her words, and nothing of hers changes. A project he did
    // not make is not his to link, unlink or take a snapshot of, even once he has pushed to it.
    let writing = vec![
        Probe::post(
            "/v1/sync/push",
            "v1/sync/push".into(),
            bh,
            Some(json!({ "ops": [
                {
                    "op": sid(), "action": "put", "kind": "note", "id": note,
                    "project": project, "schema": 1, "body": { "text": "bob was here" },
                },
                {
                    "op": sid(), "action": "revoke", "kind": "project", "id": project,
                    "project": project, "schema": 1,
                },
            ] })),
            Own,
        ),
        Probe::post(
            "/v1/sync/projects/{id}/link",
            format!("v1/sync/projects/{project}/link"),
            bh,
            None,
            Missing,
        ),
        Probe::post(
            "/v1/sync/projects/{id}/unlink",
            format!("v1/sync/projects/{project}/unlink"),
            bh,
            None,
            Missing,
        ),
        Probe::get("/v1/sync/pull", "v1/sync/pull?after=0".into(), bp, Missing),
        Probe::get(
            "/v1/sync/snapshot",
            format!("v1/sync/snapshot?project={project}"),
            bh,
            Missing,
        ),
        Probe::get("/v1/account/export", "v1/account/export".into(), bp, Own),
    ];
    ask(&s, &writing, &alice.words, &mut failures).await;

    // A deletion confirmed by signing in as Alice is not Bob's to make.
    let (_, verifier, back) = s.confirm(ALICE).await;
    let response = s
        .post_as(
            bp,
            "v1/account/delete",
            Some(&json!({ "code": query(&back)["code"], "verifier": verifier })),
        )
        .await;
    if response.status() != StatusCode::FORBIDDEN {
        failures.push(format!(
            "POST v1/account/delete with Alice's confirmation: {}",
            response.status()
        ));
    }

    // Every route is asked, or has its reason not to be.
    let probed: BTreeSet<&str> = reading
        .iter()
        .chain(&writing)
        .map(|p| p.route)
        .chain(["/v1/account/delete"])
        .collect();
    let unprobed: BTreeMap<&str, &str> = UNPROBED.iter().copied().collect();
    for route in served_routes() {
        let route = route.as_str();
        if !probed.contains(route) && !unprobed.contains_key(route) {
            failures.push(format!("{route}: served, and neither probed nor excused"));
        }
        if probed.contains(route) && unprobed.contains_key(route) {
            failures.push(format!("{route}: probed, and excused as well"));
        }
    }
    for route in probed.iter().chain(unprobed.keys()) {
        if !served_routes().contains(*route) {
            failures.push(format!("{route}: named here, and not served"));
        }
    }

    // Bob deletes his own account; nothing of Alice's has moved through any of it.
    let (_, verifier, back) = s.confirm(BOB).await;
    let deleted = s
        .post_as(
            bp,
            "v1/account/delete",
            Some(&json!({ "code": query(&back)["code"], "verifier": verifier })),
        )
        .await;
    assert!(deleted.status().is_success(), "{}", deleted.status());
    let after = holding(&s, &alice).await;
    if before != after {
        failures.push(format!(
            "Alice's account changed:\nbefore {before:#}\nafter {after:#}"
        ));
    }
    // And her own devices still reach what is hers.
    let hers: Value = ok(s.get_as(&alice.host.access, "v1/sync/projects").await).await;
    assert!(hers.to_string().contains(&alice.project));

    assert!(
        failures.is_empty(),
        "{} ways Bob reached Alice:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

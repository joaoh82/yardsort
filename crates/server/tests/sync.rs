//! Sync over HTTP, on each store: only a registered host syncs, two hosts of one account push and
//! pull each other's records and the other one hears of it on its relay link, and a push, a pull
//! and a snapshot are bounded. What each op comes to is in `records.rs`.

mod common;

use std::time::Duration;

use common::served::*;
use remote_client::{Client, Incoming, LinkReader, RelayLink};
use remote_proto::account::{Credentials, DeviceKind};
use remote_proto::pairing::HostSummary;
use remote_proto::relay::{HostNotice, LinkFrame, TicketRequest, NOTICE_CHANNEL};
use remote_proto::sync::{
    OpResult, PushReply, SnapshotPage, SyncPage, SyncProjects, MAX_RECORD_BODY, MAX_SYNC_BODY,
};
use remote_proto::{ErrorCode, HostId};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
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
    only_a_registered_host_syncs,
    two_hosts_sync_a_project_and_the_other_hears_of_each_change,
    a_push_is_bounded_and_each_op_answered_on_its_own,
    a_cursor_the_server_cannot_serve_asks_for_a_snapshot,
    sync_is_limited_per_device,
);

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

fn sid() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn host(s: &Served, subject: &str, name: &str) -> (Credentials, HostId) {
    let host = s.sign_in(subject, name, DeviceKind::Host).await;
    let summary: HostSummary = ok(s
        .post_as(&host.access, "v1/hosts", Some(&json!({ "name": name })))
        .await)
    .await;
    (host, summary.id)
}

fn put(id: &str, kind: &str, project: &str, base: Option<u64>, body: Value) -> Value {
    let mut op = json!({
        "op": sid(), "action": "put", "kind": kind, "id": id, "project": project,
        "schema": 1, "body": body,
    });
    if let Some(base) = base {
        op["base"] = base.into();
    }
    op
}

async fn push(s: &Served, who: &Credentials, ops: Vec<Value>) -> reqwest::Response {
    s.post_as(&who.access, "v1/sync/push", Some(&json!({ "ops": ops })))
        .await
}

async fn pushed(s: &Served, who: &Credentials, ops: Vec<Value>) -> Vec<OpResult> {
    ok::<PushReply>(push(s, who, ops).await).await.results
}

async fn link(s: &Served, who: &Credentials, host: &HostId) -> RelayLink {
    Client::new(s.base.as_str())
        .unwrap()
        .open_relay(
            &who.access,
            &TicketRequest::Host {
                host: host.clone(),
                protocol: None,
            },
        )
        .await
        .unwrap()
}

/// The next notice on a host's link, or `None` when none comes soon.
async fn notice(reader: &mut LinkReader) -> Option<HostNotice> {
    let message = tokio::time::timeout(SOON, reader.next()).await.ok()?.ok()?;
    let Incoming::Binary(bytes) = message else {
        panic!("a host's link carries only link frames: {message:?}");
    };
    match LinkFrame::decode(&bytes).unwrap() {
        LinkFrame::Text(NOTICE_CHANNEL, text) => serde_json::from_slice(text).ok(),
        other => panic!("not a notice: {other:?}"),
    }
}

async fn only_a_registered_host_syncs(records: impl Records + Clone + 'static) {
    let s = Served::start(records).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let computer = s.sign_in("ada", "Laptop", DeviceKind::Computer).await;
    let unregistered = s.sign_in("ada", "Box", DeviceKind::Host).await;
    let project = sid();
    let create = vec![put(
        &project,
        "project",
        &project,
        None,
        json!({"name": "widgets"}),
    )];
    for who in [&phone, &computer] {
        let refused = push(&s, who, create.clone()).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(error(refused).await.code, ErrorCode::Forbidden);
        let pull = s.get_as(&who.access, "v1/sync/pull?after=0").await;
        assert_eq!(pull.status(), StatusCode::FORBIDDEN);
        let list = s.get_as(&who.access, "v1/sync/projects").await;
        assert_eq!(list.status(), StatusCode::FORBIDDEN);
    }
    let refused = push(&s, &unregistered, create.clone()).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let anonymous = s.get("v1/sync/pull?after=0").await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // Nothing the refused ones sent was kept.
    let (workstation, _) = host(&s, "ada", "Workstation").await;
    let offered: SyncProjects = ok(s.get_as(&workstation.access, "v1/sync/projects").await).await;
    assert!(offered.projects.is_empty());
}

async fn two_hosts_sync_a_project_and_the_other_hears_of_each_change(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let (here, here_id) = host(&s, "ada", "Workstation").await;
    let (there, there_id) = host(&s, "ada", "Build box").await;
    let (bobs, _) = host(&s, "bob", "Bob's").await;
    let mut here_link = link(&s, &here, &here_id).await;
    let mut there_link = link(&s, &there, &there_id).await;

    let project = sid();
    let note = sid();
    let made = pushed(
        &s,
        &here,
        vec![
            put(
                &project,
                "project",
                &project,
                None,
                json!({"name": "widgets", "remote": "https://example.com/widgets.git"}),
            ),
            put(
                &note,
                "note",
                &project,
                None,
                json!({"text": "Run just check."}),
            ),
        ],
    )
    .await;
    assert!(
        made.iter().all(|r| matches!(r, OpResult::Applied { .. })),
        "{made:?}"
    );
    // Nobody else is linked yet, so nobody is told.
    assert_eq!(notice(&mut there_link.reader).await, None);

    // The other host is offered it, with its hint, and links it.
    let offered: SyncProjects = ok(s.get_as(&there.access, "v1/sync/projects").await).await;
    assert_eq!(offered.projects.len(), 1);
    assert!(!offered.projects[0].linked);
    assert_eq!(
        offered.projects[0].record.body.as_ref().unwrap()["remote"],
        "https://example.com/widgets.git"
    );
    let linked = s
        .post_as(
            &there.access,
            &format!("v1/sync/projects/{project}/link"),
            None,
        )
        .await;
    assert_eq!(linked.status(), StatusCode::NO_CONTENT);
    let snapshot: SnapshotPage = ok(s
        .get_as(
            &there.access,
            &format!("v1/sync/snapshot?project={project}"),
        )
        .await)
    .await;
    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(snapshot.next, None);

    // An edit there: here hears of it, there does not, and here pulls it.
    let edit = put(
        &note,
        "note",
        &project,
        Some(1),
        json!({"text": "Run just check first."}),
    );
    let edited = pushed(&s, &there, vec![edit]).await;
    assert!(
        matches!(edited[0], OpResult::Applied { revision: 2, .. }),
        "{edited:?}"
    );
    assert_eq!(
        notice(&mut here_link.reader).await,
        Some(HostNotice::SyncChanged)
    );
    assert_eq!(
        notice(&mut there_link.reader).await,
        None,
        "not the host that pushed"
    );
    let page: SyncPage = ok(s
        .get_as(
            &here.access,
            &format!("v1/sync/pull?after={}", snapshot.cursor),
        )
        .await)
    .await;
    assert_eq!(page.records.len(), 1);
    assert_eq!(
        page.records[0].body.as_ref().unwrap()["text"],
        "Run just check first."
    );
    assert_eq!(page.records[0].host, there_id);
    assert!(!page.more);
    let caught_up: SyncPage = ok(s
        .get_as(&here.access, &format!("v1/sync/pull?after={}", page.cursor))
        .await)
    .await;
    assert!(caught_up.records.is_empty() && caught_up.cursor == page.cursor);

    // Another account knows the ids and gets nothing.
    let theirs: SyncPage = ok(s.get_as(&bobs.access, "v1/sync/pull?after=0").await).await;
    assert!(theirs.records.is_empty());
    let snooped = s
        .get_as(&bobs.access, &format!("v1/sync/snapshot?project={project}"))
        .await;
    assert_eq!(snooped.status(), StatusCode::NOT_FOUND);
    let joined = s
        .post_as(
            &bobs.access,
            &format!("v1/sync/projects/{project}/link"),
            None,
        )
        .await;
    assert_eq!(joined.status(), StatusCode::NOT_FOUND);

    // Unlinked: there hears nothing more of it, and pulls nothing of it.
    let unlinked = s
        .post_as(
            &there.access,
            &format!("v1/sync/projects/{project}/unlink"),
            None,
        )
        .await;
    assert_eq!(unlinked.status(), StatusCode::NO_CONTENT);
    let again = put(
        &note,
        "note",
        &project,
        Some(2),
        json!({"text": "Run just check, always."}),
    );
    pushed(&s, &here, vec![again]).await;
    assert_eq!(notice(&mut there_link.reader).await, None);
    let nothing: SyncPage = ok(s.get_as(&there.access, "v1/sync/pull?after=0").await).await;
    assert!(nothing.records.is_empty());
}

async fn a_push_is_bounded_and_each_op_answered_on_its_own(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let (here, _) = host(&s, "ada", "Workstation").await;
    let project = sid();
    pushed(
        &s,
        &here,
        vec![put(
            &project,
            "project",
            &project,
            None,
            json!({"name": "widgets"}),
        )],
    )
    .await;

    // Larger than any other route takes, within sync's own limit: accepted.
    let text = "x".repeat(MAX_RECORD_BODY - 100);
    let large: Vec<Value> = (0..3)
        .map(|_| put(&sid(), "note", &project, None, json!({ "text": text })))
        .collect();
    let results = pushed(&s, &here, large).await;
    assert!(results
        .iter()
        .all(|r| matches!(r, OpResult::Applied { .. })));

    // Over sync's limit: too large, before anything is read.
    let over: Vec<Value> = (0..(MAX_SYNC_BODY / (MAX_RECORD_BODY - 100) + 2))
        .map(|_| put(&sid(), "note", &project, None, json!({ "text": text })))
        .collect();
    let refused = push(&s, &here, over).await;
    assert_eq!(refused.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Too many ops, or none, or one id twice: the push is refused whole.
    let many: Vec<Value> = (0..101)
        .map(|_| put(&sid(), "note", &project, None, json!({"text": "a"})))
        .collect();
    assert_eq!(
        push(&s, &here, many).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        push(&s, &here, vec![]).await.status(),
        StatusCode::BAD_REQUEST
    );
    let twice = put(&sid(), "note", &project, None, json!({"text": "a"}));
    assert_eq!(
        push(&s, &here, vec![twice.clone(), twice]).await.status(),
        StatusCode::BAD_REQUEST
    );
    let malformed = s
        .post_as(
            &here.access,
            "v1/sync/push",
            Some(&json!({"ops": [{"op": "not-a-uuid"}]})),
        )
        .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

    // A wrong op among good ones is refused alone.
    let too_large = put(
        &sid(),
        "note",
        &project,
        None,
        json!({ "text": "x".repeat(MAX_RECORD_BODY) }),
    );
    let not_linked = put(&sid(), "note", &sid(), None, json!({"text": "a"}));
    let fine = put(&sid(), "note", &project, None, json!({"text": "a"}));
    let results = pushed(&s, &here, vec![too_large, not_linked, fine]).await;
    assert!(
        matches!(&results[0], OpResult::Refused { error, .. } if error.code == ErrorCode::InvalidRequest)
    );
    assert!(
        matches!(&results[1], OpResult::Refused { error, .. } if error.code == ErrorCode::NotFound)
    );
    assert!(matches!(results[2], OpResult::Applied { .. }));

    // Queries that are not ids are refused without being echoed.
    let bad = s
        .get_as(&here.access, "v1/sync/snapshot?project=..%2F..%2Fetc")
        .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert!(!bad.text().await.unwrap().contains("etc"));
    let bad = s.get_as(&here.access, "v1/sync/pull?after=-1").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

async fn a_cursor_the_server_cannot_serve_asks_for_a_snapshot(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let (here, _) = host(&s, "ada", "Workstation").await;
    let project = sid();
    pushed(
        &s,
        &here,
        vec![put(
            &project,
            "project",
            &project,
            None,
            json!({"name": "widgets"}),
        )],
    )
    .await;
    let ahead = s.get_as(&here.access, "v1/sync/pull?after=99").await;
    assert_eq!(ahead.status(), StatusCode::GONE);
    let error = error(ahead).await;
    assert_eq!(error.code, ErrorCode::CursorExpired);
    assert!(error.message.contains("snapshot"), "{}", error.message);
    // From the start is always served.
    let page: SyncPage = ok(s.get_as(&here.access, "v1/sync/pull").await).await;
    assert_eq!(page.records.len(), 1);
}

async fn sync_is_limited_per_device(records: impl Records + Clone + 'static) {
    let s = Served::start_with(
        records,
        yardsort_server::http::Options {
            sync_per_minute: 2,
            ..yardsort_server::http::Options::default()
        },
    )
    .await;
    let (here, _) = host(&s, "ada", "Workstation").await;
    let (there, _) = host(&s, "ada", "Build box").await;
    for _ in 0..2 {
        let page = s.get_as(&here.access, "v1/sync/pull?after=0").await;
        assert_eq!(page.status(), StatusCode::OK);
    }
    let limited = s.get_as(&here.access, "v1/sync/projects").await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().contains_key("retry-after"));
    // Another host of the same account, from the same address, has its own allowance.
    let other = s.get_as(&there.access, "v1/sync/pull?after=0").await;
    assert_eq!(other.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_server_of_one_instance_tells_a_linked_host_only_once_per_push() {
    let s = Served::start(MemoryRecords::default()).await;
    let (here, _) = host(&s, "ada", "Workstation").await;
    let (there, there_id) = host(&s, "ada", "Build box").await;
    let project = sid();
    pushed(
        &s,
        &here,
        vec![put(
            &project,
            "project",
            &project,
            None,
            json!({"name": "widgets"}),
        )],
    )
    .await;
    s.post_as(
        &there.access,
        &format!("v1/sync/projects/{project}/link"),
        None,
    )
    .await;
    let mut hears = link(&s, &there, &there_id).await;
    let ops: Vec<Value> = (0..5)
        .map(|_| put(&sid(), "note", &project, None, json!({"text": "a"})))
        .collect();
    pushed(&s, &here, ops).await;
    assert_eq!(
        notice(&mut hears.reader).await,
        Some(HostNotice::SyncChanged)
    );
    assert_eq!(notice(&mut hears.reader).await, None, "five ops, one hint");
}

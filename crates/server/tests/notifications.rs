//! Phone notifications over HTTP, on each store: preferences, deduplication, the offline queue,
//! account isolation, and payloads that carry no code and no event key.

mod common;

use std::sync::Arc;

use remote_proto::account::DeviceKind;
use remote_proto::notify::NOTICE_TTL_MS;
use remote_proto::ErrorCode;
use reqwest::StatusCode;
use serde_json::{json, Value};
use yardsort_server::push::{PushOutcome, PushProvider, RecordingProvider};
use yardsort_server::records::{Device, IncomingNotice, Issued, MemoryRecords, Records};
use yardsort_server::tokens;

use common::served::Served;

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
    each_category_is_a_fixed_sentence,
    preferences_filter_and_do_not_backfill,
    one_event_is_sent_once_across_retries_and_two_desktops,
    an_unreachable_phone_is_delivered_once_when_it_returns,
    a_reconnect_sends_the_cap_and_drops_the_rest,
    another_account_sees_nothing,
    a_payload_has_no_code_and_no_event_key,
    only_the_right_device_may_register_or_submit,
    an_expired_notice_is_not_delivered,
);

struct World {
    served: Served,
    push: Arc<RecordingProvider>,
}

async fn world(records: impl Records + 'static, outcome: PushOutcome) -> World {
    let push = Arc::new(RecordingProvider::new(outcome));
    let served = Served::start_notifying(records, Arc::clone(&push) as Arc<dyn PushProvider>).await;
    World { served, push }
}

async fn signed(
    world: &World,
    subject: &str,
    name: &str,
    kind: DeviceKind,
) -> remote_proto::account::Secret {
    world.served.sign_in(subject, name, kind).await.access
}

fn notice(key: &str, category: &str, kind: &str) -> Value {
    json!({ "eventKey": key, "category": category, "kind": kind })
}

async fn submit(
    world: &World,
    access: &remote_proto::account::Secret,
    body: &Value,
) -> reqwest::Response {
    world.served.post_as(access, "v1/notices", Some(body)).await
}

async fn each_category_is_a_fixed_sentence(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    let registered = world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token" })),
        )
        .await;
    assert_eq!(registered.status(), StatusCode::NO_CONTENT);
    let cases = [
        (
            "task:example.widgets:91:progress:stamp",
            "task",
            "progress",
            "A task moved forward",
        ),
        (
            "task:example.widgets:91:complete",
            "task",
            "complete",
            "A task finished",
        ),
        (
            "review:example.widgets:41:checks",
            "review",
            "attention",
            "A review needs attention",
        ),
        (
            "failure:workflow:run-1",
            "failure",
            "failed",
            "Something failed",
        ),
        (
            "desktop-update:1.2.3",
            "desktopUpdate",
            "available",
            "A desktop update is available",
        ),
        (
            "workflow:question:run-1:ask:10",
            "workflow",
            "question",
            "A workflow has a question",
        ),
        (
            "workflow:approval:run-1:approve:11",
            "workflow",
            "approval",
            "A workflow is waiting for approval",
        ),
    ];
    for (key, category, kind, title) in cases {
        let response = submit(&world, &host, &notice(key, category, kind)).await;
        assert!(
            response.status().is_success(),
            "{key} {}",
            response.status()
        );
        let receipt: Value = response.json().await.unwrap();
        let id = receipt["id"].as_str().unwrap();
        let view = world
            .served
            .get_as(&phone, &format!("v1/notifications/{id}"))
            .await;
        let view: Value = view.json().await.unwrap();
        assert_eq!(view["title"], title);
        assert!(view["deepLink"]
            .as_str()
            .unwrap()
            .starts_with("yardsort://notice/"));
        assert!(view.get("eventKey").is_none());
        assert!(view.get("code").is_none());
    }
    assert_eq!(world.push.sent().len(), cases.len());
}

async fn preferences_filter_and_do_not_backfill(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token" })),
        )
        .await;
    let muted = json!({
        "muted": true, "task": true, "review": true, "failure": true,
        "desktopUpdate": true, "workflow": true
    });
    assert_eq!(
        world
            .served
            .post_as(&phone, "v1/notifications/preferences", Some(&muted))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(submit(
        &world,
        &host,
        &notice("task:example.widgets:1:progress:a", "task", "progress")
    )
    .await
    .status()
    .is_success());
    assert!(world.push.sent().is_empty());
    let open = json!({
        "muted": false, "task": true, "review": true, "failure": true,
        "desktopUpdate": true, "workflow": false
    });
    world
        .served
        .post_as(&phone, "v1/notifications/preferences", Some(&open))
        .await;
    assert!(submit(
        &world,
        &host,
        &notice("task:example.widgets:1:progress:a", "task", "progress")
    )
    .await
    .status()
    .is_success());
    assert!(
        world.push.sent().is_empty(),
        "a skipped notice is not sent later"
    );
    assert!(submit(
        &world,
        &host,
        &notice("workflow:question:run:ask:1", "workflow", "question")
    )
    .await
    .status()
    .is_success());
    assert!(
        world.push.sent().is_empty(),
        "a category that is off sends nothing"
    );
    assert!(submit(
        &world,
        &host,
        &notice("task:example.widgets:2:progress:b", "task", "progress")
    )
    .await
    .status()
    .is_success());
    assert_eq!(world.push.sent().len(), 1);
    let saved = world
        .served
        .get_as(&phone, "v1/notifications/preferences")
        .await;
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["workflow"], false);
    assert_eq!(saved["muted"], false);
}

async fn one_event_is_sent_once_across_retries_and_two_desktops(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let first = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let second = signed(&world, "ada", "Laptop", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token" })),
        )
        .await;
    let body = notice("review:example.widgets:41:checks", "review", "attention");
    assert!(submit(&world, &first, &body).await.status().is_success());
    assert!(submit(&world, &first, &body).await.status().is_success());
    assert!(submit(&world, &second, &body).await.status().is_success());
    assert_eq!(world.push.sent().len(), 1);
    assert_eq!(world.push.sent()[0].collapse, world.push.sent()[0].collapse);
}

async fn an_unreachable_phone_is_delivered_once_when_it_returns(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Unreachable).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token" })),
        )
        .await;
    assert!(submit(
        &world,
        &host,
        &notice("failure:workflow:run-9", "failure", "failed")
    )
    .await
    .status()
    .is_success());
    assert!(world.push.sent().is_empty());
    world.push.set_outcome(PushOutcome::Sent);
    let once = world
        .served
        .post_as(&phone, "v1/notifications/reachable", None)
        .await;
    let once: Value = once.json().await.unwrap();
    assert_eq!(once["sent"], 1);
    assert_eq!(world.push.sent().len(), 1);
    let again = world
        .served
        .post_as(&phone, "v1/notifications/reachable", None)
        .await;
    let again: Value = again.json().await.unwrap();
    assert_eq!(again["sent"], 0);
    assert_eq!(world.push.sent().len(), 1);
}

async fn a_reconnect_sends_the_cap_and_drops_the_rest(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Unreachable).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "fcm", "token": "phone-token" })),
        )
        .await;
    for n in 1..=20 {
        let key = format!("task:example.widgets:{n}:progress:stamp");
        assert!(submit(&world, &host, &notice(&key, "task", "progress"))
            .await
            .status()
            .is_success());
    }
    assert!(world.push.sent().is_empty());
    world.push.set_outcome(PushOutcome::Sent);
    let flushed = world
        .served
        .post_as(&phone, "v1/notifications/reachable", None)
        .await;
    let flushed: Value = flushed.json().await.unwrap();
    assert_eq!(flushed["sent"], 8);
    let second = world
        .served
        .post_as(&phone, "v1/notifications/reachable", None)
        .await;
    let second: Value = second.json().await.unwrap();
    assert_eq!(second["sent"], 0);
    assert_eq!(world.push.sent().len(), 8);
}

async fn another_account_sees_nothing(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let ada = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    let bea = signed(&world, "bea", "Other phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &ada,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "ada-token" })),
        )
        .await;
    world
        .served
        .post_as(
            &bea,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "bea-token" })),
        )
        .await;
    let response = submit(
        &world,
        &host,
        &notice("task:example.widgets:3:complete", "task", "complete"),
    )
    .await;
    let receipt: Value = response.json().await.unwrap();
    let id = receipt["id"].as_str().unwrap();
    assert_eq!(world.push.sent().len(), 1);
    assert!(!world.push.sent()[0].json.contains("bea-token"));
    assert!(!world.push.sent()[0].json.contains("ada-token"));
    let missing = world
        .served
        .get_as(&bea, &format!("v1/notifications/{id}"))
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let own = world
        .served
        .get_as(&ada, &format!("v1/notifications/{id}"))
        .await;
    assert!(own.status().is_success());
}

async fn a_payload_has_no_code_and_no_event_key(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token-secret" })),
        )
        .await;
    let planted = notice(
        "task:example.widgets:91:progress:zzsecretmarkerzz",
        "task",
        "progress",
    );
    let mut with_code = planted.clone();
    with_code["code"] = json!("Q7K2M9NP");
    let rejected = submit(&world, &host, &with_code).await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let error: remote_proto::RemoteError = rejected.json().await.unwrap();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(world.push.sent().is_empty());
    assert!(submit(&world, &host, &planted).await.status().is_success());
    let sent = &world.push.sent()[0];
    assert!(!sent.json.contains("zzsecretmarkerzz"));
    assert!(!sent.json.contains("Q7K2M9NP"));
    assert!(!sent.json.contains("phone-token-secret"));
    assert!(!sent.json.contains("code"));
    assert!(!sent.collapse.contains("zzsecretmarkerzz"));
}

async fn only_the_right_device_may_register_or_submit(records: impl Records + 'static) {
    let world = world(records, PushOutcome::Sent).await;
    let host = signed(&world, "ada", "Workstation", DeviceKind::Host).await;
    let phone = signed(&world, "ada", "Phone", DeviceKind::Phone).await;
    let anonymous = world.served.post_as_public("v1/notifications/token").await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let phone_submits = submit(
        &world,
        &phone,
        &notice("task:example.widgets:1:progress:a", "task", "progress"),
    )
    .await;
    assert_eq!(phone_submits.status(), StatusCode::FORBIDDEN);
    let host_registers = world
        .served
        .post_as(
            &host,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "nope" })),
        )
        .await;
    assert_eq!(host_registers.status(), StatusCode::FORBIDDEN);
    assert!(world.push.sent().is_empty());
}

async fn an_expired_notice_is_not_delivered(records: impl Records + 'static) {
    let account = records
        .account("https://id.example", "ada", 1)
        .await
        .unwrap();
    let phone = Device {
        id: remote_proto::DeviceId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        account: account.clone(),
        name: "Phone".into(),
        kind: DeviceKind::Phone,
        platform: remote_proto::Platform::Ios,
        created_at: 1_000,
        last_seen_at: None,
        revoked: None,
    };
    records
        .add_device(
            phone.clone(),
            Issued {
                access: tokens::hash("access"),
                access_expires_at: 9_000,
                refresh: tokens::hash("refresh"),
                refresh_expires_at: 9_000,
            },
        )
        .await
        .unwrap();
    assert!(records
        .save_push_token(&account, &phone.id, "apns", "tok", 1_000)
        .await
        .unwrap());
    let id = uuid::Uuid::new_v4().to_string();
    records
        .accept_notice(
            &account,
            &IncomingNotice {
                id,
                event_key: "task:example.widgets:1:progress:stamp".into(),
                category: "task".into(),
                kind: "progress".into(),
                title: "A task moved forward".into(),
                body: "Open it on this phone. The work stays on the computer.".into(),
                expires_at: 1_000 + NOTICE_TTL_MS,
                suppressed: false,
            },
            1_000,
        )
        .await
        .unwrap();
    assert_eq!(
        records.prepare_flush(&account, 1_000).await.unwrap().len(),
        1
    );
    assert!(records
        .prepare_flush(&account, 1_000 + NOTICE_TTL_MS + 1)
        .await
        .unwrap()
        .is_empty());
    assert!(records
        .prepare_flush(&account, 1_000 + NOTICE_TTL_MS + 1)
        .await
        .unwrap()
        .is_empty());
}

impl Served {
    async fn post_as_public(&self, path: &str) -> reqwest::Response {
        self.client.post(self.url(path)).send().await.unwrap()
    }
}

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
use yardsort_server::records::{
    DeliveryEnd, Device, IncomingNotice, Issued, MemoryRecords, Records, RevokeReason, Revoked,
    CLAIM_LEASE_MS, OPEN_NOTICE_CAP,
};
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
    notice_routes_are_limited_per_device,
    a_live_claim_is_not_sent_twice,
    the_same_event_from_two_callers_is_one_row,
    sent_notices_do_not_fill_the_cap,
    a_revoked_phone_is_not_sent,
    a_gone_token_is_deleted,
    an_agent_notice_names_only_its_terminal,
    an_older_phone_keeps_the_agent_preference,
);

struct World {
    served: Served,
    push: Arc<RecordingProvider>,
}

async fn await_sent(push: &RecordingProvider, n: usize) {
    for _ in 0..100 {
        if push.sent().len() >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("sent {} of {n}", push.sent().len());
}

async fn await_attempts(push: &RecordingProvider, n: usize) {
    for _ in 0..100 {
        if push.attempts() >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("attempts {} of {n}", push.attempts());
}

async fn world(records: impl Records + Clone + 'static, outcome: PushOutcome) -> World {
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

async fn each_category_is_a_fixed_sentence(records: impl Records + Clone + 'static) {
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
        (
            "agent:run-1:approval:12",
            "agent",
            "approval",
            "An agent needs you",
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
    await_sent(&world.push, cases.len()).await;
    assert_eq!(world.push.sent().len(), cases.len());
}

async fn preferences_filter_and_do_not_backfill(records: impl Records + Clone + 'static) {
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
    await_sent(&world.push, 1).await;
    assert_eq!(world.push.sent().len(), 1);
    let saved = world
        .served
        .get_as(&phone, "v1/notifications/preferences")
        .await;
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["workflow"], false);
    assert_eq!(saved["muted"], false);
}

async fn one_event_is_sent_once_across_retries_and_two_desktops(
    records: impl Records + Clone + 'static,
) {
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
    await_sent(&world.push, 1).await;
    assert_eq!(world.push.sent().len(), 1);
    let ada = world.push.sent()[0].collapse.clone();
    assert_eq!(ada.len(), 64);
    assert!(ada.chars().all(|c| c.is_ascii_hexdigit()));
    let bea_host = signed(&world, "bea", "Workstation", DeviceKind::Host).await;
    let bea_phone = signed(&world, "bea", "Phone", DeviceKind::Phone).await;
    world
        .served
        .post_as(
            &bea_phone,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "bea-token" })),
        )
        .await;
    assert!(submit(&world, &bea_host, &body).await.status().is_success());
    await_sent(&world.push, 2).await;
    let bea = world.push.sent()[1].collapse.clone();
    assert_eq!(bea.len(), 64);
    assert!(bea.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(
        ada, bea,
        "the same event on another account is a different collapse id"
    );
}

async fn an_unreachable_phone_is_delivered_once_when_it_returns(
    records: impl Records + Clone + 'static,
) {
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
    // The computer returns before the send. Wait until that attempt has given the row back.
    await_attempts(&world.push, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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

async fn a_reconnect_sends_the_cap_and_drops_the_rest(records: impl Records + Clone + 'static) {
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
    let mut receipts = Vec::new();
    for n in 1..=20 {
        let key = format!("task:example.widgets:{n}:progress:stamp");
        let response = submit(&world, &host, &notice(&key, "task", "progress")).await;
        assert!(response.status().is_success());
        let receipt: Value = response.json().await.unwrap();
        receipts.push(receipt["id"].as_str().unwrap().to_owned());
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    await_attempts(&world.push, 20).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
    let sent_ids: Vec<String> = world
        .push
        .sent()
        .iter()
        .map(|alert| {
            let body: Value = serde_json::from_str(&alert.json).unwrap();
            body["deepLink"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    let newest = &receipts[12..];
    assert_eq!(sent_ids.len(), 8);
    for id in newest {
        assert!(
            sent_ids.contains(id),
            "key receipt {id} was not among the 8 newest"
        );
    }
}

async fn another_account_sees_nothing(records: impl Records + Clone + 'static) {
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
    await_sent(&world.push, 1).await;
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

async fn a_payload_has_no_code_and_no_event_key(records: impl Records + Clone + 'static) {
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
    await_sent(&world.push, 1).await;
    let sent = &world.push.sent()[0];
    assert!(!sent.json.contains("zzsecretmarkerzz"));
    assert!(!sent.json.contains("Q7K2M9NP"));
    assert!(!sent.json.contains("phone-token-secret"));
    assert!(!sent.json.contains("code"));
    assert!(!sent.collapse.contains("zzsecretmarkerzz"));
}

async fn only_the_right_device_may_register_or_submit(records: impl Records + Clone + 'static) {
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

async fn notice_routes_are_limited_per_device(records: impl Records + Clone + 'static) {
    let push = Arc::new(RecordingProvider::new(PushOutcome::Unreachable));
    let served = Served::start_notifying_with(
        records,
        Arc::clone(&push) as Arc<dyn PushProvider>,
        yardsort_server::http::Options {
            notices_per_minute: 2,
            ..yardsort_server::http::Options::default()
        },
    )
    .await;
    let host = served
        .sign_in("ada", "Workstation", DeviceKind::Host)
        .await
        .access;
    let other = served
        .sign_in("bea", "Workstation", DeviceKind::Host)
        .await
        .access;
    let body = notice("task:example.widgets:1:progress:a", "task", "progress");
    assert!(served
        .post_as(&host, "v1/notices", Some(&body))
        .await
        .status()
        .is_success());
    assert!(served
        .post_as(&host, "v1/notices", Some(&body))
        .await
        .status()
        .is_success());
    let limited = served.post_as(&host, "v1/notices", Some(&body)).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(served
        .post_as(
            &other,
            "v1/notices",
            Some(&notice(
                "task:example.widgets:2:progress:b",
                "task",
                "progress"
            )),
        )
        .await
        .status()
        .is_success());
    let phone = served
        .sign_in("ada", "Phone", DeviceKind::Phone)
        .await
        .access;
    assert_eq!(
        served
            .post_as(
                &phone,
                "v1/notifications/token",
                Some(&json!({ "platform": "apns", "token": "phone-token" })),
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(served
        .post_as(&phone, "v1/notifications/reachable", None)
        .await
        .status()
        .is_success());
    assert_eq!(
        served
            .post_as(&phone, "v1/notifications/reachable", None)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

async fn a_live_claim_is_not_sent_twice(records: impl Records + Clone + 'static) {
    let account = records
        .account("https://id.example", "ada", 1_000)
        .await
        .unwrap();
    let phone = enroll_phone(&records, &account).await;
    let notice = incoming("task:one:1:progress:a");
    records
        .accept_notice(&account, &notice, 1_000)
        .await
        .unwrap();
    let ready = records.prepare_flush(&account, 1_000).await.unwrap();
    assert_eq!(ready.len(), 1);
    let claimed = records
        .claim_delivery(&ready[0].notice_id, &ready[0].device, 1_000)
        .await
        .unwrap()
        .unwrap();
    assert!(records
        .prepare_flush(&account, 1_000)
        .await
        .unwrap()
        .is_empty());
    let later = 1_000 + CLAIM_LEASE_MS + 1;
    let again = records.prepare_flush(&account, later).await.unwrap();
    assert_eq!(again.len(), 1);
    let renewed = records
        .claim_delivery(&again[0].notice_id, &again[0].device, later)
        .await
        .unwrap()
        .unwrap();
    records
        .finish_delivery(&ready[0].notice_id, &phone.id, claimed, DeliveryEnd::Sent)
        .await
        .unwrap();
    assert!(
        records
            .prepare_flush(&account, later)
            .await
            .unwrap()
            .is_empty(),
        "finishing the expired claim does not complete the one that replaced it"
    );
    records
        .finish_delivery(&again[0].notice_id, &phone.id, renewed, DeliveryEnd::Sent)
        .await
        .unwrap();
    assert!(records
        .prepare_flush(&account, later)
        .await
        .unwrap()
        .is_empty());
}

async fn the_same_event_from_two_callers_is_one_row(records: impl Records + Clone + 'static) {
    let account = records
        .account("https://id.example", "ada", 1_000)
        .await
        .unwrap();
    enroll_phone(&records, &account).await;
    let first = incoming("task:one:1:progress:a");
    let mut second = incoming("task:one:1:progress:a");
    second.id = uuid::Uuid::new_v4().to_string();
    let left_store = records.clone();
    let right_store = records.clone();
    let (left, right) = tokio::join!(
        left_store.accept_notice(&account, &first, 1_000),
        right_store.accept_notice(&account, &second, 1_000),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert!(left.fresh ^ right.fresh);
    assert_eq!(
        records.prepare_flush(&account, 1_000).await.unwrap().len(),
        1
    );
}

async fn sent_notices_do_not_fill_the_cap(records: impl Records + Clone + 'static) {
    let account = records
        .account("https://id.example", "ada", 1_000)
        .await
        .unwrap();
    let phone = enroll_phone(&records, &account).await;
    let mut ids = Vec::new();
    for n in 0..OPEN_NOTICE_CAP {
        let notice = incoming(&format!("task:one:{n}:progress:a"));
        ids.push(notice.id.clone());
        let record = records
            .accept_notice(&account, &notice, 1_000)
            .await
            .unwrap();
        assert!(!record.suppressed, "{n}");
    }
    let blocked = records
        .accept_notice(&account, &incoming("task:one:overflow:progress:a"), 1_000)
        .await
        .unwrap();
    assert!(blocked.fresh);
    assert!(blocked.suppressed);
    for id in &ids {
        let claimed = records
            .claim_delivery(id, &phone.id, 1_000)
            .await
            .unwrap()
            .unwrap();
        records
            .finish_delivery(id, &phone.id, claimed, DeliveryEnd::Sent)
            .await
            .unwrap();
    }
    let next = records
        .accept_notice(&account, &incoming("task:one:after:progress:a"), 1_000)
        .await
        .unwrap();
    assert!(!next.suppressed);
}

async fn a_revoked_phone_is_not_sent(records: impl Records + Clone + 'static) {
    let account = records
        .account("https://id.example", "ada", 1_000)
        .await
        .unwrap();
    let phone = enroll_phone(&records, &account).await;
    records
        .accept_notice(&account, &incoming("task:one:1:progress:a"), 1_000)
        .await
        .unwrap();
    assert_eq!(
        records.prepare_flush(&account, 1_000).await.unwrap().len(),
        1
    );
    assert!(records
        .revoke(
            &account,
            &phone.id,
            Revoked {
                at: 2_000,
                reason: RevokeReason::ByOwner,
            },
        )
        .await
        .unwrap());
    assert!(records
        .prepare_flush(&account, 2_000)
        .await
        .unwrap()
        .is_empty());
    assert!(!records
        .save_push_token(&account, &phone.id, "apns", "again", 2_000)
        .await
        .unwrap());
}

async fn a_gone_token_is_deleted(records: impl Records + Clone + 'static) {
    let account = records
        .account("https://id.example", "ada", 1_000)
        .await
        .unwrap();
    let phone = enroll_phone(&records, &account).await;
    let notice = incoming("task:one:1:progress:a");
    records
        .accept_notice(&account, &notice, 1_000)
        .await
        .unwrap();
    let ready = records.prepare_flush(&account, 1_000).await.unwrap();
    let claimed = records
        .claim_delivery(&ready[0].notice_id, &ready[0].device, 1_000)
        .await
        .unwrap()
        .unwrap();
    records
        .finish_delivery(&ready[0].notice_id, &phone.id, claimed, DeliveryEnd::Gone)
        .await
        .unwrap();
    assert!(records
        .prepare_flush(&account, 1_000)
        .await
        .unwrap()
        .is_empty());
    assert!(records
        .save_push_token(&account, &phone.id, "apns", "replacement", 1_000)
        .await
        .unwrap());
    let again = records.prepare_flush(&account, 1_000).await.unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].token, "replacement");
}

fn incoming(key: &str) -> IncomingNotice {
    IncomingNotice {
        id: uuid::Uuid::new_v4().to_string(),
        event_key: key.into(),
        category: "task".into(),
        kind: "progress".into(),
        title: "A task moved forward".into(),
        body: "Open it on this phone. The work stays on the computer.".into(),
        expires_at: 1_000 + NOTICE_TTL_MS,
        suppressed: false,
        run_id: None,
        step_id: None,
        workspace_id: None,
        terminal_id: None,
        host_id: None,
    }
}

async fn enroll_phone(
    records: &impl Records,
    account: &yardsort_server::records::AccountId,
) -> Device {
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
                access: tokens::hash(&format!("{}-access", phone.id.as_str())),
                access_expires_at: 9_000,
                refresh: tokens::hash(&format!("{}-refresh", phone.id.as_str())),
                refresh_expires_at: 9_000,
            },
        )
        .await
        .unwrap();
    assert!(records
        .save_push_token(account, &phone.id, "apns", "tok", 1_000)
        .await
        .unwrap());
    phone
}

async fn an_expired_notice_is_not_delivered(records: impl Records + Clone + 'static) {
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
                run_id: None,
                step_id: None,
                workspace_id: None,
                terminal_id: None,
                host_id: None,
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

async fn an_agent_notice_names_only_its_terminal(records: impl Records + Clone + 'static) {
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
    let mut body = notice("agent:run-7:approval:1", "agent", "approval");
    body["workspaceId"] = json!("ws-1");
    body["terminalId"] = json!("pty-1");
    let receipt: Value = submit(&world, &host, &body).await.json().await.unwrap();
    let view: Value = world
        .served
        .get_as(
            &phone,
            &format!("v1/notifications/{}", receipt["id"].as_str().unwrap()),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(view["category"], "agent");
    assert_eq!(view["title"], "An agent needs you");
    assert_eq!(view["workspaceId"], "ws-1");
    assert_eq!(view["terminalId"], "pty-1");
    assert!(view.get("runId").is_none());

    // The push itself carries the fixed words and the link, and neither id.
    await_sent(&world.push, 1).await;
    let alert = &world.push.sent()[0].json;
    assert!(alert.contains("An agent needs you"), "{alert}");
    assert!(
        !alert.contains("ws-1") && !alert.contains("pty-1"),
        "{alert}"
    );

    // Another category may not name a terminal, and what looks like a one-time code is no id.
    let mut task = notice("task:example.widgets:3:complete", "task", "complete");
    task["workspaceId"] = json!("ws-1");
    task["terminalId"] = json!("pty-1");
    let mut coded = notice("agent:run-7:approval:2", "agent", "approval");
    coded["workspaceId"] = json!("ws-1");
    coded["terminalId"] = json!("ABCDEFGH");
    for body in [task, coded] {
        let receipt: Value = submit(&world, &host, &body).await.json().await.unwrap();
        let view: Value = world
            .served
            .get_as(
                &phone,
                &format!("v1/notifications/{}", receipt["id"].as_str().unwrap()),
            )
            .await
            .json()
            .await
            .unwrap();
        assert!(view.get("workspaceId").is_none(), "{view}");
        assert!(view.get("terminalId").is_none(), "{view}");
    }
}

async fn an_older_phone_keeps_the_agent_preference(records: impl Records + Clone + 'static) {
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
    let read = || async {
        world
            .served
            .get_as(&phone, "v1/notifications/preferences")
            .await
            .json::<Value>()
            .await
            .unwrap()
    };
    assert_eq!(read().await["agent"], true, "on until turned off");
    let off = json!({
        "muted": false, "task": true, "review": true, "failure": true,
        "desktopUpdate": true, "workflow": true, "agent": false
    });
    let saved = world
        .served
        .post_as(&phone, "v1/notifications/preferences", Some(&off))
        .await;
    assert_eq!(saved.status(), StatusCode::NO_CONTENT);
    // A phone from before agent notices saves what it knows, and the switch it never showed
    // stays where it was.
    let older = json!({
        "muted": false, "task": false, "review": true, "failure": true,
        "desktopUpdate": true, "workflow": true
    });
    world
        .served
        .post_as(&phone, "v1/notifications/preferences", Some(&older))
        .await;
    let now = read().await;
    assert_eq!(now["task"], false);
    assert_eq!(now["agent"], false);
    assert!(submit(
        &world,
        &host,
        &notice("agent:run-8:approval:1", "agent", "approval")
    )
    .await
    .status()
    .is_success());
    assert!(
        world.push.sent().is_empty(),
        "agents that are off send nothing"
    );
}

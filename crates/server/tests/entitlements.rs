//! Asking an entitlements service (`docs/design/46-remote-r7.md` §11), against a stand-in served
//! over real HTTP: what it says is done and kept for a minute, what an account was allowed stays
//! allowed for a day while it cannot be reached, and a refusal never stops reading, revoking,
//! exporting or deleting. Over HTTP, on each store; the timing directly, with the clock in hand.

mod common;

use common::plans::{Plan, Plans, TOKEN};
use common::served::*;
use remote_proto::account::{Credentials, DeviceKind, Secret};
use remote_proto::pairing::HostSummary;
use remote_proto::sync::{OpResult, PushReply};
use remote_proto::{ErrorCode, ErrorReason, HostId};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use url::Url;
use yardsort_server::entitlements::{Action, Counts, Entitlements, GRACE_MS, KEEP_ANSWER_MS};
use yardsort_server::records::{AccountId, MemoryRecords, Records};

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
    a_device_the_plan_has_no_room_for_is_not_signed_in,
    more_synced_data_waits_on_the_plan_but_a_revoke_or_delete_never_does,
    a_refusal_never_stops_reading_revoking_exporting_or_deleting,
);

const T: i64 = 1_800_000_000_000;

async fn ok<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

fn account(id: &str) -> AccountId {
    AccountId(id.into())
}

fn devices(n: u64) -> Counts {
    Counts {
        devices: Some(n),
        ..Counts::default()
    }
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

fn sid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn put(id: &str, kind: &str, project: &str, body: Value) -> Value {
    json!({
        "op": sid(), "action": "put", "kind": kind, "id": id, "project": project,
        "schema": 1, "body": body,
    })
}

async fn push(s: &Served, who: &Credentials, ops: Vec<Value>) -> reqwest::Response {
    s.post_as(&who.access, "v1/sync/push", Some(&json!({ "ops": ops })))
        .await
}

fn revision(result: &OpResult) -> u64 {
    match result {
        OpResult::Applied { revision, .. } => *revision,
        other => panic!("not applied: {other:?}"),
    }
}

async fn refusal(response: reqwest::Response) -> remote_proto::RemoteError {
    // Only a sign-in is refused outright; a push is held back instead.
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    error(response).await
}

#[tokio::test]
async fn an_answer_is_kept_for_a_minute_for_its_account_and_action() {
    let plans = Plans::start().await;
    let entitlements = plans.entitlements();
    let (ada, bob) = (account("acct-ada"), account("acct-bob"));
    entitlements
        .may(&ada, Action::Device, devices(1), T)
        .await
        .unwrap();
    entitlements
        .may(&ada, Action::Device, devices(2), T + 1)
        .await
        .unwrap();
    assert_eq!(plans.asked_about("device"), 1, "kept");
    entitlements
        .may(&bob, Action::Device, devices(0), T + 2)
        .await
        .unwrap();
    entitlements
        .may(&ada, Action::Sync, Counts::default(), T + 3)
        .await
        .unwrap();
    assert_eq!(plans.asked().len(), 3, "another account, another action");

    // Refused from now on: the kept yes holds for its minute, then the no comes.
    plans.set("device", Plan::Refuse(Some("Your plan has two devices.")));
    entitlements
        .may(&ada, Action::Device, devices(2), T + KEEP_ANSWER_MS - 1)
        .await
        .unwrap();
    let no = entitlements
        .may(&ada, Action::Device, devices(2), T + KEEP_ANSWER_MS)
        .await
        .unwrap_err();
    assert_eq!(no.code, ErrorCode::Forbidden);
    assert_eq!(no.reason, Some(ErrorReason::PlanLimit));
    assert_eq!(no.message, "Your plan has two devices.");
    assert!(!no.retryable);

    // What it was asked: the server's account id, the action and its counts, with the token.
    let (authorization, question) = plans.asked().last().cloned().unwrap();
    assert_eq!(authorization.as_deref(), Some(&*format!("Bearer {TOKEN}")));
    assert_eq!(
        question,
        json!({"account": "acct-ada", "action": "device", "counts": {"devices": 2}})
    );
}

#[tokio::test]
async fn while_the_service_is_down_what_was_allowed_stays_allowed_for_a_day() {
    let plans = Plans::start().await;
    let entitlements = plans.entitlements();
    let (ada, bob) = (account("acct-ada"), account("acct-bob"));
    entitlements
        .may(&ada, Action::Stream, Counts::default(), T)
        .await
        .unwrap();

    plans.set_all(Plan::Down);
    let later = T + KEEP_ANSWER_MS;
    entitlements
        .may(&ada, Action::Stream, Counts::default(), later)
        .await
        .unwrap();
    // Asked once, then that is kept too: a service that is down is not asked at every request.
    entitlements
        .may(&ada, Action::Stream, Counts::default(), later + 1)
        .await
        .unwrap();
    assert_eq!(plans.asked_about("stream"), 2);

    // Something new waits: another account, or another action of this one.
    for (who, action) in [(&bob, Action::Stream), (&ada, Action::Device)] {
        let waits = entitlements
            .may(who, action, Counts::default(), later)
            .await
            .unwrap_err();
        assert_eq!(waits.code, ErrorCode::Internal);
        assert!(waits.retryable);
        assert_eq!(waits.reason, None);
        assert!(waits.message.contains("cannot check"), "{}", waits.message);
    }

    // Still allowed when asked just within the day of the last yes, and that is kept for its
    // minute; asked after it, not.
    let last = T + GRACE_MS - 1;
    entitlements
        .may(&ada, Action::Stream, Counts::default(), last)
        .await
        .unwrap();
    let after = entitlements
        .may(
            &ada,
            Action::Stream,
            Counts::default(),
            last + KEEP_ANSWER_MS,
        )
        .await
        .unwrap_err();
    assert_eq!(after.code, ErrorCode::Internal);

    // Back up: asked again once what was kept lapses, and allowed.
    plans.set_all(Plan::Allow);
    entitlements
        .may(
            &ada,
            Action::Stream,
            Counts::default(),
            last + 2 * KEEP_ANSWER_MS,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_refusal_is_not_undone_by_the_service_going_down() {
    let plans = Plans::start().await;
    let entitlements = plans.entitlements();
    let ada = account("acct-ada");
    entitlements
        .may(&ada, Action::Sync, Counts::default(), T)
        .await
        .unwrap();
    plans.set("sync", Plan::Refuse(None));
    let refused = entitlements
        .may(&ada, Action::Sync, Counts::default(), T + KEEP_ANSWER_MS)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, Some(ErrorReason::PlanLimit));
    assert!(
        refused.message.contains("synced data"),
        "{}",
        refused.message
    );
    plans.set("sync", Plan::Down);
    let still = entitlements
        .may(
            &ada,
            Action::Sync,
            Counts::default(),
            T + 2 * KEEP_ANSWER_MS,
        )
        .await
        .unwrap_err();
    assert_eq!(
        still.code,
        ErrorCode::Internal,
        "not let through by an earlier yes"
    );
}

#[tokio::test]
async fn a_service_nothing_answers_at_is_down() {
    // A port that was just free: nothing listens there.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/ask", free.local_addr().unwrap())).unwrap();
    drop(free);
    let entitlements = Entitlements::service(
        url,
        Some(Secret::new(TOKEN)),
        yardsort_server::oidc::http_client_builder()
            .no_proxy()
            .build()
            .unwrap(),
    );
    let waits = entitlements
        .may(&account("acct-ada"), Action::Device, devices(0), T)
        .await
        .unwrap_err();
    assert_eq!(waits.code, ErrorCode::Internal);
}

async fn a_device_the_plan_has_no_room_for_is_not_signed_in(
    records: impl Records + Clone + 'static,
) {
    let plans = Plans::start().await;
    let s = Served::start_entitled(records, plans.entitlements()).await;
    let first = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let (_, question) = plans.asked().last().cloned().unwrap();
    assert_eq!(question["action"], "device");
    assert_eq!(question["counts"], json!({"devices": 0}));
    let account = question["account"].as_str().unwrap().to_owned();
    assert!(!account.contains("ada"), "the server's id, not the subject");
    assert!(!question.to_string().contains("Phone"), "no device name");

    plans.set("device", Plan::Refuse(Some("Your plan has one device.")));
    // Bob's account was never asked about, so it hears the no at once.
    let refused = refusal(s.try_sign_in("bob", "Laptop", DeviceKind::Computer).await).await;
    assert_eq!(refused.reason, Some(ErrorReason::PlanLimit));
    assert_eq!(refused.message, "Your plan has one device.");
    // The device already signed in keeps working.
    let mine: Value = ok(s.devices(Some(&first.access)).await).await;
    assert_eq!(mine["devices"].as_array().unwrap().len(), 1);
}

async fn more_synced_data_waits_on_the_plan_but_a_revoke_or_delete_never_does(
    records: impl Records + Clone + 'static,
) {
    let plans = Plans::start().await;
    let s = Served::start_entitled(records, plans.entitlements()).await;
    let (here, _) = host(&s, "ada").await;
    let project = sid();
    let note = sid();
    let made: PushReply = ok(push(
        &s,
        &here,
        vec![
            put(&project, "project", &project, json!({"name": "widgets"})),
            put(&note, "note", &project, json!({"text": "Run just check."})),
        ],
    )
    .await)
    .await;
    let note_revision = revision(&made.results[1]);
    let (_, question) = plans.asked().last().cloned().unwrap();
    assert_eq!(question["action"], "sync");
    let asked_bytes = question["counts"]["syncBytes"].as_u64().unwrap();
    assert!(asked_bytes > 0, "{question}");

    plans.set("sync", Plan::Refuse(Some("Your plan holds 1 MB of notes.")));
    // Another account's computer, so nothing kept answers for it. Ops go in order: the push
    // stops at its first put and says why, and what came before it is applied.
    let (bobs, _) = host(&s, "bob").await;
    let bob_project = sid();
    let first: PushReply = ok(push(
        &s,
        &bobs,
        vec![put(
            &bob_project,
            "project",
            &bob_project,
            json!({"name": "b"}),
        )],
    )
    .await)
    .await;
    assert!(first.results.is_empty());
    let held = first.held.expect("held back");
    assert_eq!(held.code, ErrorCode::Forbidden);
    assert_eq!(held.reason, Some(ErrorReason::PlanLimit));
    assert_eq!(held.message, "Your plan holds 1 MB of notes.");
    let unknown = sid();
    let mixed: PushReply = ok(push(
        &s,
        &bobs,
        vec![
            json!({"op": sid(), "action": "delete", "kind": "note", "id": unknown,
                   "project": bob_project, "base": 1}),
            put(&bob_project, "project", &bob_project, json!({"name": "b"})),
            json!({"op": sid(), "action": "delete", "kind": "note", "id": unknown,
                   "project": bob_project, "base": 1}),
        ],
    )
    .await)
    .await;
    assert_eq!(
        mixed.results.len(),
        1,
        "the delete before the put: {mixed:?}"
    );
    assert!(mixed.held.is_some());

    // Ada's yes is kept for its minute; once it lapses she is refused too, but a revoke and a
    // delete still go through, and nothing is asked for them.
    plans.set_all(Plan::Refuse(None));
    let asked = plans.asked().len();
    let removed: PushReply = ok(push(
        &s,
        &here,
        vec![
            json!({"op": sid(), "action": "revoke", "kind": "note", "id": note,
                   "project": project, "base": note_revision}),
        ],
    )
    .await)
    .await;
    assert!(
        matches!(removed.results[0], OpResult::Applied { .. }),
        "{removed:?}"
    );
    assert!(removed.held.is_none());
    let gone: PushReply = ok(push(
        &s,
        &here,
        vec![
            json!({"op": sid(), "action": "delete", "kind": "project", "id": project,
                    "project": project, "base": revision(&made.results[0])}),
        ],
    )
    .await)
    .await;
    assert!(
        matches!(gone.results[0], OpResult::Applied { .. }),
        "{gone:?}"
    );
    assert_eq!(
        plans.asked().len(),
        asked,
        "nothing asked for a revoke or a delete"
    );
}

async fn a_refusal_never_stops_reading_revoking_exporting_or_deleting(
    records: impl Records + Clone + 'static,
) {
    let plans = Plans::start().await;
    let s = Served::start_entitled(records, plans.entitlements()).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let laptop = s.sign_in("ada", "Laptop", DeviceKind::Computer).await;
    let (here, _) = host(&s, "ada").await;
    plans.set_all(Plan::Refuse(None));
    let asked = plans.asked().len();

    let _: Value = ok(s.devices(Some(&phone.access)).await).await;
    let _: Value = ok(s.get_as(&here.access, "v1/sync/pull?after=0").await).await;
    let _: Value = ok(s.get_as(&phone.access, "v1/account/export").await).await;
    let revoked = s
        .post_as(
            &phone.access,
            &format!("v1/devices/{}/revoke", laptop.device),
            None,
        )
        .await;
    assert!(revoked.status().is_success(), "{}", revoked.status());
    let (_, verifier, back) = s.confirm("ada").await;
    let deleted = s
        .post_as(
            &phone.access,
            "v1/account/delete",
            Some(&json!({"code": query(&back)["code"], "verifier": verifier})),
        )
        .await;
    assert!(deleted.status().is_success(), "{}", deleted.status());
    // The plan has no room for anything new, and none of that asked it.
    assert_eq!(plans.asked().len(), asked);
}

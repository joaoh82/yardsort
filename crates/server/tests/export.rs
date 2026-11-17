//! Exporting an account over HTTP, on each store: any device of the account gets all of it, a
//! page at a time, and nothing of another account, no credential and no push token.

mod common;

use common::served::*;
use remote_proto::account::{Credentials, DeviceKind, ServerDescription};
use remote_proto::export::{ExportPage, EXPORT_CAPABILITY, EXPORT_PAGE, EXPORT_PAGE_BYTES};
use remote_proto::pairing::HostSummary;
use remote_proto::sync::{PushReply, MAX_PUSH_OPS};
use remote_proto::ErrorCode;
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
    an_account_is_exported_whole_a_page_at_a_time,
    an_export_is_its_own_accounts_and_asked_for_properly,
    a_page_stays_small_whatever_its_records_hold,
);

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

fn put(id: &str, kind: &str, project: &str, text: &str) -> Value {
    json!({
        "op": sid(), "action": "put", "kind": kind, "id": id, "project": project,
        "schema": 1, "body": { "text": text },
    })
}

/// A registered host of `subject`'s, and a project it made with `notes` notes in it.
async fn host_with_notes(s: &Served, subject: &str, notes: usize) -> (Credentials, String) {
    let host = s.sign_in(subject, "Workstation", DeviceKind::Host).await;
    let _: HostSummary = ok(s
        .post_as(
            &host.access,
            "v1/hosts",
            Some(&json!({ "name": "Workstation" })),
        )
        .await)
    .await;
    let project = sid();
    let mut ops = vec![put(&project, "project", &project, "widgets")];
    ops.extend((0..notes).map(|n| put(&sid(), "note", &project, &format!("note {n}"))));
    for batch in ops.chunks(MAX_PUSH_OPS) {
        let reply: PushReply = ok(s
            .post_as(&host.access, "v1/sync/push", Some(&json!({ "ops": batch })))
            .await)
        .await;
        assert_eq!(reply.results.len(), batch.len());
    }
    (host, project)
}

async fn page(s: &Served, who: &Credentials, after: Option<&str>) -> (ExportPage, String) {
    let path = match after {
        Some(after) => format!("v1/account/export?after={after}"),
        None => "v1/account/export".into(),
    };
    let response = s.get_as(&who.access, &path).await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.unwrap();
    (serde_json::from_str(&text).unwrap(), text)
}

async fn an_account_is_exported_whole_a_page_at_a_time(records: impl Records + Clone + 'static) {
    let s = Served::start(records).await;
    let (host, project) = host_with_notes(&s, "ada", EXPORT_PAGE + 50).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let registered = s
        .post_as(
            &phone.access,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "the-phones-push-token" })),
        )
        .await;
    assert_eq!(registered.status(), StatusCode::NO_CONTENT);

    let (first, first_text) = page(&s, &phone, None).await;
    let account = first
        .account
        .clone()
        .expect("the first page has the account");
    assert_eq!(account.subject, "ada");
    assert_eq!(account.issuer, s.idp.issuer());
    assert_eq!(account.devices.len(), 2);
    assert!(account
        .devices
        .iter()
        .any(|d| d.current && d.kind == DeviceKind::Phone));
    assert_eq!(account.hosts.len(), 1);
    assert_eq!(account.push_routes.len(), 1);
    assert_eq!(account.push_routes[0].platform, "apns");
    assert_eq!(first.records.len(), EXPORT_PAGE);
    let next = first.next.clone().expect("more than a page");

    let (rest, rest_text) = page(&s, &phone, Some(next.as_str())).await;
    assert!(
        rest.account.is_none(),
        "the account is on the first page only"
    );
    assert!(rest.next.is_none());
    assert_eq!(
        rest.records.len(),
        51,
        "the project and the rest of its notes"
    );
    let ids: Vec<&str> = first
        .records
        .iter()
        .chain(&rest.records)
        .map(|r| r.id.as_str())
        .collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "by id, once each");
    assert!(ids.contains(&project.as_str()));

    // Nothing that opens anything: no credential, no push token.
    for text in [&first_text, &rest_text] {
        for secret in [
            phone.access.expose(),
            phone.refresh.expose(),
            host.access.expose(),
            host.refresh.expose(),
            "the-phones-push-token",
        ] {
            assert!(!text.contains(secret), "an export holds a secret");
        }
    }

    // The host may export too; any device of the account may.
    let (from_host, _) = page(&s, &host, None).await;
    assert_eq!(from_host.account.unwrap().devices.len(), 2);
}

async fn an_export_is_its_own_accounts_and_asked_for_properly(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let (_, adas) = host_with_notes(&s, "ada", 3).await;
    let (bob, bobs) = host_with_notes(&s, "bob", 2).await;

    let (export, text) = page(&s, &bob, None).await;
    assert_eq!(export.account.unwrap().subject, "bob");
    assert_eq!(export.records.len(), 3);
    assert!(export.records.iter().all(|r| r.project.as_str() == bobs));
    assert!(!text.contains(&adas));

    let description: ServerDescription = ok(s.get("v1/server").await).await;
    assert!(description
        .capabilities
        .iter()
        .any(|c| c == EXPORT_CAPABILITY));

    let anonymous = s.get("v1/account/export").await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let not_an_id = s
        .get_as(&bob.access, "v1/account/export?after=../accounts")
        .await;
    assert_eq!(not_an_id.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error(not_an_id).await.code, ErrorCode::InvalidRequest);

    // A signed-out device exports nothing.
    let revoked = s
        .post_as(
            &bob.access,
            &format!("v1/devices/{}/revoke", bob.device),
            None,
        )
        .await;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let after = s.get_as(&bob.access, "v1/account/export").await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
}

async fn a_page_stays_small_whatever_its_records_hold(records: impl Records + Clone + 'static) {
    let s = Served::start(records).await;
    let (host, project) = host_with_notes(&s, "ada", 0).await;
    // Eighty notes of ten thousand characters: more than a page's worth of bytes, fewer than its
    // count of records.
    let long = "x".repeat(10_000);
    let notes: Vec<Value> = (0..80)
        .map(|_| put(&sid(), "note", &project, &long))
        .collect();
    for batch in notes.chunks(20) {
        let _: PushReply = ok(s
            .post_as(&host.access, "v1/sync/push", Some(&json!({ "ops": batch })))
            .await)
        .await;
    }
    let mut after: Option<String> = None;
    let mut pages = 0;
    let mut seen = 0;
    loop {
        let (page, text) = page(&s, &host, after.as_deref()).await;
        assert!(
            text.len() < EXPORT_PAGE_BYTES + 64 * 1024,
            "{} bytes",
            text.len()
        );
        assert!(!page.records.is_empty());
        seen += page.records.len();
        pages += 1;
        match page.next {
            Some(next) => after = Some(next.as_str().to_owned()),
            None => break,
        }
    }
    assert_eq!(seen, 81, "the project and its notes, each once");
    assert!(pages >= 2);
}

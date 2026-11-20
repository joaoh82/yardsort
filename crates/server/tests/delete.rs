//! Deleting an account over HTTP, on each store: only after its person signed in again as it,
//! from one of its devices or on the server's own page; then nothing of it is left, every device
//! of it is told why it is signed out, and another account is untouched.

mod common;

use common::idp::Tamper;
use common::served::*;
use remote_proto::account::{
    Credentials, DeviceKind, NewDevice, Secret, ServerDescription, TokenRequest,
};
use remote_proto::delete::{AccountDeleted, DELETE_CAPABILITY};
use remote_proto::pairing::HostSummary;
use remote_proto::sync::PushReply;
use remote_proto::{ErrorCode, ErrorReason, Platform};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use url::Url;
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
    a_device_deletes_its_account_and_every_device_of_it_is_told,
    only_a_fresh_sign_in_again_as_the_same_account_deletes_it,
    the_servers_page_deletes_the_account_signed_in_as_in_that_browser,
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

/// A registered host of `subject`'s, with a synced project and a note in it.
async fn host_with_a_note(s: &Served, subject: &str) -> Credentials {
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
    let op = |id: &str, kind: &str, text: &str| {
        json!({
            "op": sid(), "action": "put", "kind": kind, "id": id, "project": project,
            "schema": 1, "body": { "text": text },
        })
    };
    let reply: PushReply = ok(s
        .post_as(
            &host.access,
            "v1/sync/push",
            Some(&json!({ "ops": [op(&project, "project", "widgets"), op(&sid(), "note", "a note")] })),
        )
        .await)
    .await;
    assert_eq!(reply.results.len(), 2);
    host
}

/// What a device holding `access` is told now, if it is refused.
async fn refusal(s: &Served, access: &Secret) -> (StatusCode, Option<ErrorReason>) {
    let response = s.devices(Some(access)).await;
    let status = response.status();
    if status.is_success() {
        return (status, None);
    }
    (status, error(response).await.reason)
}

async fn refresh(s: &Served, refresh: &Secret) -> reqwest::Response {
    s.token(&TokenRequest::Refresh {
        refresh: refresh.clone(),
    })
    .await
}

async fn delete(s: &Served, access: &Secret, code: &str, verifier: &str) -> reqwest::Response {
    s.post_as(
        access,
        "v1/account/delete",
        Some(&json!({ "code": code, "verifier": verifier })),
    )
    .await
}

async fn device_count(s: &Served, access: &Secret) -> usize {
    let page: Value = ok(s.devices(Some(access)).await).await;
    page["devices"].as_array().unwrap().len()
}

async fn a_device_deletes_its_account_and_every_device_of_it_is_told(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let described: ServerDescription = ok(s.get("v1/server").await).await;
    assert!(described
        .capabilities
        .iter()
        .any(|c| c == DELETE_CAPABILITY));

    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let host = host_with_a_note(&s, "ada").await;
    let computer = s.sign_in("ada", "Laptop", DeviceKind::Computer).await;
    let bob = s.sign_in("bob", "Bob's phone", DeviceKind::Phone).await;
    let registered = s
        .post_as(
            &phone.access,
            "v1/notifications/token",
            Some(&json!({ "platform": "apns", "token": "phone-token" })),
        )
        .await;
    assert_eq!(registered.status(), StatusCode::NO_CONTENT);

    // The provider is asked to have the person sign in again, whatever session it has.
    let (to_provider, verifier, back) = s.confirm("ada").await;
    let asked = query(&to_provider);
    assert_eq!(asked["prompt"], "login");
    assert_eq!(asked["max_age"], "0");
    let code = query(&back)["code"].clone();

    let deleted: AccountDeleted = ok(delete(&s, &phone.access, &code, &verifier).await).await;
    assert_eq!(
        deleted,
        AccountDeleted {
            devices: 3,
            hosts: 1,
            records: 2
        }
    );

    // Every device of the account is told why, by access and by refresh, whichever asked.
    for device in [&phone, &host, &computer] {
        assert_eq!(
            refusal(&s, &device.access).await,
            (StatusCode::UNAUTHORIZED, Some(ErrorReason::AccountDeleted))
        );
        let refused = refresh(&s, &device.refresh).await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
        let refused = error(refused).await;
        assert_eq!(refused.reason, Some(ErrorReason::AccountDeleted));
        assert!(
            refused
                .message
                .contains("Nothing on the device was deleted"),
            "{}",
            refused.message
        );
    }
    let export = s.get_as(&host.access, "v1/account/export").await;
    assert_eq!(export.status(), StatusCode::UNAUTHORIZED);

    // Another account is untouched; a credential that was never issued is only signed out.
    assert_eq!(device_count(&s, &bob.access).await, 1);
    assert_eq!(
        refusal(&s, &Secret::new("yat_never-issued")).await,
        (StatusCode::UNAUTHORIZED, None)
    );

    // Signing in again as the same person starts an empty account.
    let again = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    assert_eq!(device_count(&s, &again.access).await, 1);
    let hosts: Value = ok(s.get_as(&again.access, "v1/hosts").await).await;
    assert!(hosts["hosts"].as_array().unwrap().is_empty());
    let export: Value = ok(s.get_as(&again.access, "v1/account/export").await).await;
    assert!(export["records"].as_array().unwrap().is_empty());
}

async fn only_a_fresh_sign_in_again_as_the_same_account_deletes_it(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let bob = s.sign_in("bob", "Bob's phone", DeviceKind::Phone).await;
    let still_there = || async { device_count(&s, &phone.access).await == 1 };

    // A plain sign-in's code confirms nothing.
    let verifier = a_verifier();
    let mut start = s.url("v1/login");
    start
        .query_pairs_mut()
        .append_pair("challenge", &yardsort_server::tokens::s256(&verifier))
        .append_pair("challengeMethod", "S256")
        .append_pair("redirect", common::idp::APP);
    let to_provider = location(&s.client.get(start).send().await.unwrap());
    assert!(!query(&to_provider).contains_key("prompt"));
    let (code, state) = s.idp.approve(&to_provider, "ada");
    let back = callback(&s, &code, &state).await;
    let refused = delete(&s, &phone.access, &query(&back)["code"], &verifier).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(still_there().await);

    // A confirming one's code enrolls nothing.
    let (_, verifier, back) = s.confirm("ada").await;
    let enrolled = s
        .token(&TokenRequest::SignIn {
            code: Secret::new(query(&back)["code"].clone()),
            verifier: Secret::new(verifier),
            device: NewDevice {
                name: "Sneaky".into(),
                kind: DeviceKind::Phone,
                platform: Platform::Linux,
            },
        })
        .await;
    assert_eq!(enrolled.status(), StatusCode::UNAUTHORIZED);
    assert!(still_there().await);

    // Signed in again as someone else: refused, and the code is spent.
    let (_, verifier, back) = s.confirm("bob").await;
    let code = query(&back)["code"].clone();
    let refused = delete(&s, &phone.access, &code, &verifier).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(error(refused).await.code, ErrorCode::Forbidden);
    let spent = delete(&s, &bob.access, &code, &verifier).await;
    assert_eq!(spent.status(), StatusCode::UNAUTHORIZED);
    assert!(still_there().await);
    assert_eq!(device_count(&s, &bob.access).await, 1);

    // A wrong verifier spends the code too.
    let (_, verifier, back) = s.confirm("ada").await;
    let code = query(&back)["code"].clone();
    let wrong = delete(&s, &phone.access, &code, &a_verifier()).await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let late = delete(&s, &phone.access, &code, &verifier).await;
    assert_eq!(late.status(), StatusCode::UNAUTHORIZED);
    assert!(still_there().await);

    // A provider that kept its session, or did not say when the person signed in, confirms
    // nothing: the browser goes back with an error, and no code.
    for tamper in [Tamper::StaleAuthTime, Tamper::NoAuthTime] {
        s.idp.with(|i| i.tamper = tamper);
        let (_, _, back) = s.confirm("ada").await;
        assert_eq!(
            query(&back).get("error").map(String::as_str),
            Some("unauthenticated")
        );
        assert!(!query(&back).contains_key("code"));
    }
    s.idp.with(|i| i.tamper = Tamper::Nothing);

    // Signed in again as someone with no account here: no code, and no account made for them.
    let (_, _, back) = s.confirm("nobody").await;
    assert!(query(&back).contains_key("error"));

    // A malformed request, and one with no access, delete nothing.
    let malformed = s
        .post_as(
            &phone.access,
            "v1/account/delete",
            Some(&json!({ "code": "x" })),
        )
        .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    let anonymous = s
        .client
        .post(s.url("v1/account/delete"))
        .json(&json!({ "code": "x", "verifier": a_verifier() }))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert!(still_there().await);

    // And then it does.
    let (_, verifier, back) = s.confirm("ada").await;
    let deleted: AccountDeleted =
        ok(delete(&s, &phone.access, &query(&back)["code"], &verifier).await).await;
    assert_eq!(deleted.devices, 1);
}

fn a_verifier() -> String {
    yardsort_server::tokens::random()
}

async fn callback(s: &Served, code: &str, state: &str) -> Url {
    let mut callback = s.url("v1/login/callback");
    callback
        .query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", state);
    location(&s.client.get(callback).send().await.unwrap())
}

/// The browser's side of the server's page: what it was sent, and its cookie.
async fn page_until_typed(s: &Served, subject: &str) -> (String, String) {
    let intro = s.get("account/delete").await;
    assert_eq!(intro.status(), StatusCode::OK);
    assert!(intro.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let text = intro.text().await.unwrap();
    assert!(
        text.contains("Nothing on your computers or phones is deleted"),
        "{text}"
    );
    assert!(text.contains("Sign in again to delete"));

    let started = s.get("account/delete/start").await;
    assert_eq!(started.status(), StatusCode::SEE_OTHER);
    let set = started.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(set.starts_with("yardsort_delete="), "{set}");
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Lax"),
        "{set}"
    );
    assert!(set.contains("Path=/account/delete"), "{set}");
    let cookie = set.split(';').next().unwrap().to_owned();
    let to_provider = location(&started);
    assert_eq!(query(&to_provider)["prompt"], "login");
    let (code, state) = s.idp.approve(&to_provider, subject);
    let back = callback(s, &code, &state).await;
    assert_eq!(back.path(), "/account/delete/confirm");
    let form = s.client.get(back.clone()).send().await.unwrap();
    assert_eq!(form.status(), StatusCode::OK);
    let form = form.text().await.unwrap();
    let code = query(&back)["code"].clone();
    assert!(form.contains(&format!("value=\"{code}\"")), "{form}");
    (code, cookie)
}

async fn post_form(
    s: &Served,
    code: &str,
    typed: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> reqwest::Response {
    let mut request = s
        .client
        .post(s.url("account/delete/confirm"))
        .form(&[("code", code), ("confirm", typed)]);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    request.send().await.unwrap()
}

async fn the_servers_page_deletes_the_account_signed_in_as_in_that_browser(
    records: impl Records + Clone + 'static,
) {
    let s = Served::start(records).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let host = host_with_a_note(&s, "ada").await;
    let bob = s.sign_in("bob", "Bob's phone", DeviceKind::Phone).await;
    let ours = s.base.origin().ascii_serialization();
    let (code, cookie) = page_until_typed(&s, "ada").await;

    // Mistyped, from another browser, or from another site: nothing is deleted, the code kept.
    let mistyped = post_form(&s, &code, "delte", Some(&cookie), Some(&ours)).await;
    assert_eq!(mistyped.status(), StatusCode::BAD_REQUEST);
    assert!(mistyped
        .text()
        .await
        .unwrap()
        .contains(&format!("value=\"{code}\"")));
    let elsewhere = post_form(&s, &code, "delete", None, Some(&ours)).await;
    assert_eq!(elsewhere.status(), StatusCode::UNAUTHORIZED);
    let forged = post_form(
        &s,
        &code,
        "delete",
        Some(&cookie),
        Some("https://evil.example.invalid"),
    )
    .await;
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let someone_elses = post_form(
        &s,
        &code,
        "delete",
        Some("yardsort_delete=another-browsers-verifier-0123456789abcdefghijk"),
        Some(&ours),
    )
    .await;
    assert_eq!(someone_elses.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(device_count(&s, &phone.access).await, 2);

    // That last one spent the code: start again, and type it right.
    let (code, cookie) = page_until_typed(&s, "ada").await;
    let deleted = post_form(&s, &code, " Delete ", Some(&cookie), Some(&ours)).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let cleared = deleted.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    let text = deleted.text().await.unwrap();
    assert!(
        text.contains("2 devices, 1 computer and 2 synced records were deleted"),
        "{text}"
    );

    for device in [&phone, &host] {
        assert_eq!(
            refusal(&s, &device.access).await,
            (StatusCode::UNAUTHORIZED, Some(ErrorReason::AccountDeleted))
        );
    }
    assert_eq!(device_count(&s, &bob.access).await, 1);
    let again = post_form(&s, &code, "delete", Some(&cookie), Some(&ours)).await;
    assert_eq!(again.status(), StatusCode::UNAUTHORIZED);

    // Back from the provider with an error: nothing deleted, and a way to start over.
    let mut refused = s.url("account/delete/confirm");
    refused
        .query_pairs_mut()
        .append_pair("error", "unauthenticated");
    let refused = s.client.get(refused).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(refused
        .text()
        .await
        .unwrap()
        .contains("nothing was deleted"));
}

/// Every table that holds anything of an account reaches it through a cascading reference, and
/// after a deletion not one row of the account, its devices or its hosts is left in any of them.
/// A later migration that adds a table without the cascade fails here.
#[tokio::test]
async fn postgres_every_table_cascades_and_nothing_of_a_deleted_account_is_left() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let records = yardsort_server::postgres::PgRecords::connect_with(db.config.clone())
        .await
        .unwrap();
    let (sql, connection) = db.config.connect(tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);

    // Tables that hold nothing of any one account. The journal holds a deleted account's id, and
    // nothing else of it, on purpose: so that a restore cannot bring the account back.
    const NOT_AN_ACCOUNTS: [&str; 7] = [
        "accounts",
        "deleted_credentials",
        "journal",
        "pending_logins",
        "rate_limits",
        "schema_migrations",
        "server_identity",
    ];
    let tables: Vec<String> = sql
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'
             ORDER BY table_name",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    let cascading: Vec<String> = sql
        .query(
            "SELECT DISTINCT tc.table_name FROM information_schema.table_constraints tc
             JOIN information_schema.referential_constraints rc
               ON rc.constraint_name = tc.constraint_name
              AND rc.constraint_schema = tc.constraint_schema
             WHERE tc.table_schema = current_schema() AND tc.constraint_type = 'FOREIGN KEY'
               AND rc.delete_rule = 'CASCADE'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    let loose: Vec<&String> = tables
        .iter()
        .filter(|t| !NOT_AN_ACCOUNTS.contains(&t.as_str()) && !cascading.contains(t))
        .collect();
    assert!(loose.is_empty(), "no cascading reference: {loose:?}");
    // And every reference to an account, a device or a host cascades.
    let held: Vec<(String, String)> = sql
        .query(
            "SELECT kcu.table_name, kcu.column_name FROM information_schema.key_column_usage kcu
             JOIN information_schema.referential_constraints rc
               ON rc.constraint_name = kcu.constraint_name
              AND rc.constraint_schema = kcu.constraint_schema
             WHERE kcu.table_schema = current_schema() AND rc.delete_rule <> 'CASCADE'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert!(held.is_empty(), "references that do not cascade: {held:?}");

    let s = Served::start(records).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let host = host_with_a_note(&s, "ada").await;
    let bob = host_with_a_note(&s, "bob").await;
    for (device, token) in [(&phone, "ada-token"), (&bob, "bob-token")] {
        let _ = s
            .post_as(
                &device.access,
                "v1/notifications/token",
                Some(&json!({ "platform": "apns", "token": token })),
            )
            .await;
    }
    let notice = json!({ "eventKey": "review:example.widgets:41:checks", "category": "review", "kind": "attention" });
    for device in [&host, &bob] {
        let _ = s.post_as(&device.access, "v1/notices", Some(&notice)).await;
    }
    let host_id: String = ok::<Value>(s.get_as(&host.access, "v1/hosts").await).await["hosts"][0]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let _ = s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await;
    let account: String = sql
        .query_one("SELECT id FROM accounts WHERE subject = 'ada'", &[])
        .await
        .unwrap()
        .get(0);
    let devices: Vec<String> = sql
        .query("SELECT id FROM devices WHERE account = $1", &[&account])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    let hosts = vec![host_id];

    // What names ada, before: something in every table that can.
    let columns: Vec<(String, String)> = sql
        .query(
            "SELECT table_name, column_name FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND column_name IN ('account', 'device', 'host')
             ORDER BY table_name, column_name",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let left = |at_all: bool| {
        let sql = &sql;
        let columns = &columns;
        let (account, devices, hosts) = (&account, &devices, &hosts);
        async move {
            let mut found = Vec::new();
            for (table, column) in columns {
                let ids: Vec<String> = match column.as_str() {
                    "account" => vec![account.clone()],
                    "device" => devices.clone(),
                    _ => hosts.clone(),
                };
                let n: i64 = sql
                    .query_one(
                        &format!("SELECT count(*) FROM {table} WHERE {column} = ANY($1)"),
                        &[&ids],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if (n > 0) == at_all {
                    found.push(format!("{table}.{column}"));
                }
            }
            found
        }
    };
    let before = left(true).await;
    for kept in [
        "access_credentials.device",
        "devices.account",
        "host_offers.host",
        "hosts.account",
        "mobile_notice_deliveries.device",
        "mobile_notices.account",
        "push_tokens.account",
        "refresh_credentials.device",
        "sync_changes.account",
        "sync_links.host",
        "sync_records.account",
    ] {
        assert!(before.iter().any(|t| t == kept), "{kept} not in {before:?}");
    }
    let bobs_before: i64 = sql
        .query_one("SELECT count(*) FROM sync_records", &[])
        .await
        .unwrap()
        .get(0);

    let (_, verifier, back) = s.confirm("ada").await;
    let _: AccountDeleted =
        ok(delete(&s, &phone.access, &query(&back)["code"], &verifier).await).await;

    assert_eq!(left(true).await, Vec::<String>::new());
    let accounts: i64 = sql
        .query_one("SELECT count(*) FROM accounts WHERE subject = 'ada'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(accounts, 0);
    let bobs_after: i64 = sql
        .query_one("SELECT count(*) FROM sync_records", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(bobs_after, bobs_before - 2, "bob's two records stay");
    drop(s);
    db.drop().await;
}

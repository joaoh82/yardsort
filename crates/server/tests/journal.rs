//! The journal over HTTP, on each store: a server on records restored from a backup refuses
//! every request until `reconcile` has replayed what was closed since, and nothing closed comes
//! back after it (docs/design/46-remote-r7.md §12). And the journal in a bucket, against a
//! stand-in S3 that checks every request's signature.

mod common;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use common::served::*;
use remote_proto::account::DeviceKind;
use remote_proto::pairing::HostSummary;
use remote_proto::{ErrorCode, ErrorReason};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use yardsort_server::http::now_ms;
use yardsort_server::journal::{
    self, Closed, Dir, Entry, Journal, Reason, Signing, Standing, Store, S3,
};
use yardsort_server::records::{MemoryRecords, Records};

mod memory {
    #[tokio::test]
    async fn a_restored_backup_is_refused_until_reconciled_and_nothing_closed_comes_back() {
        let live = super::MemoryRecords::default();
        let backup = {
            let live = live.clone();
            || async move { live.backup().unwrap() }
        };
        super::a_restored_backup_is_refused_until_reconciled_and_nothing_closed_comes_back(
            live, backup,
        )
        .await;
    }
}

mod postgres {
    use yardsort_server::postgres::PgRecords;

    #[tokio::test]
    async fn a_restored_backup_is_refused_until_reconciled_and_nothing_closed_comes_back() {
        let Some(db) = super::common::pg::database().await else {
            return;
        };
        let live = PgRecords::connect_with(db.config.clone()).await.unwrap();
        let copies = std::sync::Mutex::new(Vec::new());
        let backup = || async {
            let copy = db.restored_copy().await;
            let records = PgRecords::connect_with(copy.config.clone()).await.unwrap();
            copies.lock().unwrap().push(copy);
            records
        };
        super::a_restored_backup_is_refused_until_reconciled_and_nothing_closed_comes_back(
            live, backup,
        )
        .await;
        for copy in copies.into_inner().unwrap() {
            copy.drop().await;
        }
        db.drop().await;
    }
}

async fn ok<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(status.is_success(), "{status}");
    response.json().await.unwrap()
}

/// Ask until `done`, for at most ten seconds.
async fn eventually<F: Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    for _ in 0..200 {
        if done().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never: {what}");
}

async fn health(s: &Served) -> StatusCode {
    s.get("health").await.status()
}

async fn a_restored_backup_is_refused_until_reconciled_and_nothing_closed_comes_back<R, F>(
    live: R,
    backup: impl FnOnce() -> F,
) where
    R: Records + Clone + 'static,
    F: Future<Output = R>,
{
    let dir = tempfile::tempdir().unwrap();
    let store = Store::Dir(Dir::new(dir.path().join("journal")));
    let s = Served::start_journaled(live.clone(), Journal::new(store.clone())).await;
    let phone = s.sign_in("ada", "Phone", DeviceKind::Phone).await;
    let tablet = s.sign_in("ada", "Tablet", DeviceKind::Phone).await;
    let workstation = s.sign_in("ada", "Workstation", DeviceKind::Host).await;
    let host: HostSummary = ok(s
        .post_as(
            &workstation.access,
            "v1/hosts",
            Some(&json!({ "name": "Workstation" })),
        )
        .await)
    .await;
    let carol = s.sign_in("carol", "Phone", DeviceKind::Phone).await;
    let bob = s.sign_in("bob", "Phone", DeviceKind::Phone).await;

    // The backup, taken now.
    let restored = backup().await;

    // Since: ada revokes her tablet, her computer stops being a host, and carol deletes her
    // account.
    let revoked = s
        .post_as(
            &phone.access,
            &format!("v1/devices/{}/revoke", tablet.device),
            None,
        )
        .await;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let removed = s
        .post_as(
            &workstation.access,
            &format!("v1/hosts/{}/remove", host.id),
            None,
        )
        .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let (_, verifier, back) = s.confirm("carol").await;
    let deleted = s
        .post_as(
            &carol.access,
            "v1/account/delete",
            Some(&json!({ "code": query(&back)["code"], "verifier": verifier })),
        )
        .await;
    assert!(deleted.status().is_success(), "{}", deleted.status());
    // Out to the journal, as the server sends them within seconds.
    Journal::new(store.clone()).flush(&live).await.unwrap();
    assert_eq!(store.list(None).await.unwrap().len(), 3);
    s.fail();

    // Restored without a journal, the backup lets the revoked tablet back in: what the journal
    // is for.
    let careless = Served::start(restored.clone()).await;
    assert!(careless
        .devices(Some(&tablet.access))
        .await
        .status()
        .is_success());
    careless.fail();

    // With it, every request is refused until the records are reconciled, health checks too.
    let r = Served::start_journaled(restored.clone(), Journal::new(store.clone())).await;
    eventually("the restored records found behind", || async {
        health(&r).await == StatusCode::SERVICE_UNAVAILABLE
    })
    .await;
    for device in [&tablet, &phone, &bob] {
        let refused = r.devices(Some(&device.access)).await;
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            refused.headers().get("retry-after").map(|v| v.as_bytes()),
            Some(&b"60"[..])
        );
        let error = error(refused).await;
        assert_eq!(error.code, ErrorCode::Internal);
        assert!(error.retryable);
        assert!(
            error.message.contains("after a restore"),
            "{}",
            error.message
        );
    }

    let done = journal::reconcile(&store, &restored, now_ms())
        .await
        .unwrap();
    assert_eq!(
        (
            done.replayed,
            done.changed,
            done.devices,
            done.hosts,
            done.accounts
        ),
        (3, 3, 1, 1, 1),
        "{done:?}"
    );
    assert!(done.unreadable.is_empty());
    // The running server hears that it was reconciled, asks again, and serves.
    eventually("the reconciled records served", || async {
        health(&r).await == StatusCode::OK
    })
    .await;

    // Nothing closed after the backup is open again.
    let tablet_now = r.devices(Some(&tablet.access)).await;
    assert_eq!(tablet_now.status(), StatusCode::UNAUTHORIZED);
    let carol_now = error(r.devices(Some(&carol.access)).await).await;
    assert_eq!(carol_now.code, ErrorCode::Unauthenticated);
    assert_eq!(carol_now.reason, Some(ErrorReason::AccountDeleted));
    let hosts: Value = ok(r.get_as(&phone.access, "v1/hosts").await).await;
    assert_eq!(hosts["hosts"], json!([]), "{hosts}");
    // And what was open still is.
    let devices: Value = ok(r.devices(Some(&phone.access)).await).await;
    let tablet_row = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == tablet.device.as_str())
        .cloned()
        .unwrap();
    assert!(!tablet_row["revokedAt"].is_null(), "{tablet_row}");
    assert!(r.devices(Some(&bob.access)).await.status().is_success());
    assert!(restored.journal_unwritten(10).await.unwrap().is_empty());

    // Once is enough: again, nothing is replayed, and the next check finds them current.
    let again = journal::reconcile(&store, &restored, now_ms())
        .await
        .unwrap();
    assert_eq!(again.replayed, 0, "{again:?}");
    assert_eq!(
        Journal::new(store.clone()).check(&restored).await,
        Standing::Current
    );
}

#[tokio::test]
async fn a_journal_that_cannot_be_asked_leaves_the_server_serving() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, "").unwrap();
    let journal = Journal::new(Store::Dir(Dir::new(&file)));
    let records = MemoryRecords::default();
    assert_eq!(journal.check(&records).await, Standing::Unknown);
    assert!(!journal.behind());
    let s = Served::start_journaled(records, journal).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(health(&s).await, StatusCode::OK);
}

#[tokio::test]
async fn an_entry_a_newer_server_wrote_is_left_and_said() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::Dir(Dir::new(dir.path()));
    store
        .put(
            "journal/0000000000000001-newer.json",
            r#"{"v":2,"at":1,"what":"somethingNew"}"#,
        )
        .await
        .unwrap();
    let records = MemoryRecords::default();
    let done = journal::reconcile(&store, &records, now_ms())
        .await
        .unwrap();
    assert_eq!(done.replayed, 0);
    assert_eq!(done.unreadable, vec!["journal/0000000000000001-newer.json"]);
    // Still behind, so still refused.
    assert_eq!(Journal::new(store).check(&records).await, Standing::Behind);
}

/// A stand-in for an S3-compatible bucket: `ListObjectsV2` two keys a page, `PUT` refusing to
/// overwrite with `If-None-Match: *`, `GET`, and every request's Signature Version 4 checked
/// against what actually arrived.
#[derive(Clone, Default)]
struct Bucket {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// Each request, as `METHOD path?query`.
    seen: Arc<Mutex<Vec<String>>>,
}

const BUCKET: &str = "journal-test";
const ACCESS: &str = "AKSTANDIN";
const SECRET: &str = "the-bucket-secret";

fn xml_error(status: StatusCode, code: &str) -> Response {
    (status, format!("<Error><Code>{code}</Code></Error>")).into_response()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn bucket(
    State(bucket): State<Bucket>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    bucket.seen.lock().unwrap().push(format!(
        "{method} {}",
        uri.path_and_query().map(|p| p.as_str()).unwrap_or_default()
    ));
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    // The signature, recomputed from the request as it arrived.
    let authorization = header("authorization");
    let Some(signed) = authorization
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
    else {
        return xml_error(StatusCode::FORBIDDEN, "AccessDenied");
    };
    let signed_headers: Vec<(String, String)> = signed
        .split(';')
        .map(|name| (name.to_owned(), header(name)))
        .collect();
    let payload = header("x-amz-content-sha256");
    if payload != hex(&Sha256::digest(&body)) {
        return xml_error(StatusCode::BAD_REQUEST, "XAmzContentSHA256Mismatch");
    }
    let query: Vec<(String, String)> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let query_refs: Vec<(&str, &str)> = query
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let amz_date = header("x-amz-date");
    let expected = journal::sign(&Signing {
        method: method.as_str(),
        path: uri.path(),
        query: &query_refs,
        headers: &signed_headers,
        payload: &payload,
        amz_date: &amz_date,
        region: "auto",
        access_key: ACCESS,
        secret_key: SECRET,
    });
    if amz_date.len() != 16 || !signed.contains("host") || authorization != expected {
        return xml_error(StatusCode::FORBIDDEN, "SignatureDoesNotMatch");
    }

    let path = uri.path().trim_start_matches('/');
    let (name, key) = match path.split_once('/') {
        Some((name, key)) => (name, Some(key.to_owned())),
        None => (path, None),
    };
    if name != BUCKET {
        return xml_error(StatusCode::NOT_FOUND, "NoSuchBucket");
    }
    let mut objects = bucket.objects.lock().unwrap();
    match (method, key) {
        (Method::PUT, Some(key)) => {
            if objects.contains_key(&key) && header("if-none-match") == "*" {
                return xml_error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed");
            }
            objects.insert(key, body.to_vec());
            StatusCode::OK.into_response()
        }
        (Method::GET, Some(key)) => match objects.get(&key) {
            Some(body) => body.clone().into_response(),
            None => xml_error(StatusCode::NOT_FOUND, "NoSuchKey"),
        },
        (Method::GET, None) => {
            let param = |name: &str| {
                query
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            };
            assert_eq!(param("list-type").as_deref(), Some("2"));
            let prefix = param("prefix").unwrap_or_default();
            // A token with what a query has to escape in it.
            let after = param("continuation-token")
                .map(|token| {
                    token
                        .trim_start_matches("after/")
                        .trim_end_matches('=')
                        .to_owned()
                })
                .or_else(|| param("start-after"));
            let matching: Vec<&String> = objects
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .filter(|k| {
                    after
                        .as_ref()
                        .is_none_or(|after| k.as_str() > after.as_str())
                })
                .collect();
            let page: Vec<&String> = matching.iter().take(2).copied().collect();
            let truncated = matching.len() > page.len();
            let mut xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult \
                 xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{BUCKET}</Name>\
                 <IsTruncated>{truncated}</IsTruncated>"
            );
            for key in &page {
                xml.push_str(&format!("<Contents><Key>{key}</Key></Contents>"));
            }
            if truncated {
                xml.push_str(&format!(
                    "<NextContinuationToken>after/{}=</NextContinuationToken>",
                    page.last().unwrap()
                ));
            }
            xml.push_str("</ListBucketResult>");
            xml.into_response()
        }
        _ => xml_error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    }
}

async fn stand_in_bucket() -> (Bucket, Url) {
    let state = Bucket::default();
    let app = axum::Router::new()
        .fallback(bucket)
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, url)
}

fn s3(endpoint: &Url, secret: &str) -> Store {
    Store::S3(S3::new(
        endpoint.clone(),
        BUCKET.into(),
        journal::Addressing::Path,
        "auto".into(),
        ACCESS.into(),
        secret.into(),
        yardsort_server::oidc::http_client_builder()
            .no_proxy()
            .build()
            .unwrap(),
    ))
}

#[tokio::test]
async fn a_bucket_journal_is_signed_paged_and_never_overwritten() {
    let (bucket, endpoint) = stand_in_bucket().await;
    let store = s3(&endpoint, SECRET);
    assert_eq!(store.list(None).await.unwrap(), Vec::<String>::new());

    let entries: Vec<Entry> = (1..=5)
        .map(|i| {
            Entry::new(
                i,
                Closed::DevicesRevoked {
                    account: "acc".into(),
                    devices: vec![format!("device-{i}")],
                    reason: Reason::ByOwner,
                },
            )
        })
        .collect();
    let rows: Vec<journal::Row> = entries.iter().map(journal::Row::of).collect();
    for row in &rows {
        store.put(&row.key, &row.body).await.unwrap();
    }
    // A key is written once: the same key again leaves the first.
    store.put(&rows[0].key, "something else").await.unwrap();
    assert_eq!(
        store.get(&rows[0].key).await.unwrap(),
        rows[0].body.as_bytes()
    );

    // Three pages of two, followed by their tokens.
    bucket.seen.lock().unwrap().clear();
    let keys: Vec<String> = rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(store.list(None).await.unwrap(), keys);
    assert_eq!(bucket.seen.lock().unwrap().len(), 3);
    assert_eq!(store.list(Some(&keys[1])).await.unwrap(), keys[2..]);

    // A wrong secret is refused, and says why.
    let error = s3(&endpoint, "a wrong secret")
        .put("journal/0000000000000009-x.json", "{}")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("403") && error.contains("SignatureDoesNotMatch"),
        "{error}"
    );

    // Restored records with none of it replay all of it.
    let records = MemoryRecords::default();
    assert_eq!(
        Journal::new(store.clone()).check(&records).await,
        Standing::Behind
    );
    let done = journal::reconcile(&store, &records, now_ms())
        .await
        .unwrap();
    assert_eq!(done.replayed, 5, "{done:?}");
    assert_eq!(
        Journal::new(store.clone()).check(&records).await,
        Standing::Current
    );

    // And what the records close is sent there.
    let ada = records.account("https://id.one", "ada", 10).await.unwrap();
    assert!(records.delete_account(&ada, 11).await.unwrap().is_some());
    assert_eq!(
        Journal::new(store.clone()).flush(&records).await.unwrap(),
        1
    );
    assert_eq!(store.list(None).await.unwrap().len(), 6);
    assert!(records.journal_unwritten(10).await.unwrap().is_empty());
}

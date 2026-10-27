//! Pairing over HTTP, on each store: a host registers and offers, a phone of the same account
//! redeems the offer and asks, the host's person answers. And everything another account, or the
//! wrong device of the same one, must not be able to do along the way.

mod common;

use common::served::*;
use remote_proto::account::{Credentials, DeviceKind};
use remote_proto::pairing::{
    GrantPage, GrantState, GrantSummary, HostPage, HostSummary, Offer, MAX_OFFER_MISSES,
    OFFER_TTL_MS,
};
use remote_proto::{ErrorCode, ProjectGrant, ProjectId, Scope};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use yardsort_server::http;
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
    a_phone_pairs_with_a_host_and_the_hosts_person_decides,
    another_account_reaches_none_of_it,
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

async fn refused(response: reqwest::Response, status: StatusCode, code: ErrorCode) {
    assert_eq!(response.status(), status);
    assert_eq!(error(response).await.code, code);
}

async fn register(s: &Served, host: &Credentials, body: Value) -> reqwest::Response {
    s.post_as(&host.access, "v1/hosts", Some(&body)).await
}

async fn offer(s: &Served, host: &Credentials, id: &HostSummary) -> Offer {
    ok(
        s.post_as(&host.access, &format!("v1/hosts/{}/offers", id.id), None)
            .await,
    )
    .await
}

async fn pair(
    s: &Served,
    device: &Credentials,
    offer: Value,
    scopes: &[&str],
) -> reqwest::Response {
    s.post_as(
        &device.access,
        "v1/pairings",
        Some(&json!({ "offer": offer, "scopes": scopes })),
    )
    .await
}

fn code(offer: &Offer) -> Value {
    // As a person might type it: lower case, no dash.
    let typed = offer.code.expose().replace('-', "").to_lowercase();
    json!({ "kind": "code", "value": typed })
}

fn secret(offer: &Offer) -> Value {
    json!({ "kind": "secret", "value": offer.secret.expose() })
}

async fn host_grants(s: &Served, host: &Credentials, id: &HostSummary) -> Vec<GrantSummary> {
    let page: GrantPage = ok(s
        .get_as(&host.access, &format!("v1/hosts/{}/grants", id.id))
        .await)
    .await;
    page.grants
}

async fn a_phone_pairs_with_a_host_and_the_hosts_person_decides(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;

    let registered: HostSummary =
        ok(register(&s, &host, json!({ "name": "Workstation" })).await).await;
    assert!(registered.current);
    // A phone is not a host.
    refused(
        register(&s, &phone, json!({ "name": "Not a host" })).await,
        StatusCode::FORBIDDEN,
        ErrorCode::Forbidden,
    )
    .await;
    let seen: HostPage = ok(s.get_as(&phone.access, "v1/hosts").await).await;
    assert_eq!(seen.hosts.len(), 1);
    assert_eq!(seen.hosts[0].id, registered.id);
    assert!(!seen.hosts[0].current);
    // Only the host makes offers for itself.
    refused(
        s.post_as(
            &phone.access,
            &format!("v1/hosts/{}/offers", registered.id),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;

    let first = offer(&s, &host, &registered).await;
    assert!(first.secret.expose().starts_with("ys1.po."));
    assert_eq!(first.code.expose().len(), 9);
    assert_eq!(first.expires_in_ms, OFFER_TTL_MS as u64);
    // A host cannot pair with itself.
    refused(
        pair(&s, &host, code(&first), &["workspace:read"]).await,
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
    )
    .await;
    let first = offer(&s, &host, &registered).await;

    let asked: GrantSummary = ok(pair(
        &s,
        &phone,
        code(&first),
        &["workspace:read", "terminal:read", "terminal:control"],
    )
    .await)
    .await;
    assert_eq!(asked.state, GrantState::Pending);
    assert_eq!(asked.host_name, "Workstation");
    assert_eq!(asked.device, phone.device);
    assert!(asked.scopes.is_empty() && asked.projects.is_none());
    assert!(asked.expires_in_ms.unwrap() <= OFFER_TTL_MS as u64);
    // The offer is spent, whichever half is presented.
    refused(
        pair(&s, &phone, secret(&first), &["workspace:read"]).await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;

    // Another device of the same account is neither the host nor the device asking: it sees
    // nothing of the request, and cannot take it back.
    let laptop = s.sign_in("alice", "Laptop", DeviceKind::Computer).await;
    refused(
        s.get_as(&laptop.access, &format!("v1/grants/{}", asked.id))
            .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    refused(
        s.post_as(
            &laptop.access,
            &format!("v1/grants/{}/revoke", asked.id),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    let theirs: GrantPage = ok(s.get_as(&laptop.access, "v1/grants").await).await;
    assert!(theirs.grants.is_empty());

    let waiting = host_grants(&s, &host, &registered).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].device_name, "Phone");
    assert_eq!(waiting[0].state, GrantState::Pending);

    let approve = |who: &Credentials, body: Value| {
        let path = format!("v1/grants/{}/approve", asked.id);
        let access = who.access.clone();
        let s = &s;
        async move { s.post_as(&access, &path, Some(&body)).await }
    };
    let project = ProjectId::new("9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d").unwrap();
    let view_only = json!({
        "scopes": ["workspace:read", "terminal:read"],
        "projects": { "kind": "only", "projects": [project] },
    });
    // The phone cannot answer its own request.
    refused(
        approve(&phone, view_only.clone()).await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    // Nor can the host give more than was asked for.
    refused(
        approve(
            &host,
            json!({ "scopes": ["workspace:read", "memory:read"], "projects": { "kind": "all" } }),
        )
        .await,
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
    )
    .await;
    let given: GrantSummary = ok(approve(&host, view_only.clone()).await).await;
    assert_eq!(given.state, GrantState::Approved);
    assert_eq!(
        given.scopes,
        [Scope::WorkspaceRead, Scope::TerminalRead].into()
    );
    assert_eq!(given.projects, Some(ProjectGrant::Only([project].into())));
    assert_eq!(given.expires_in_ms, None);
    refused(
        approve(&host, view_only).await,
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
    )
    .await;

    // The phone sees what it was given.
    let mine: GrantSummary = ok(s
        .get_as(&phone.access, &format!("v1/grants/{}", asked.id))
        .await)
    .await;
    assert_eq!(mine, given);
    let all: GrantPage = ok(s.get_as(&phone.access, "v1/grants").await).await;
    assert_eq!(all.grants, vec![given.clone()]);

    // A second request, denied: the grant in force stands, and the denial leaves the host's list.
    let second = offer(&s, &host, &registered).await;
    let again: GrantSummary = ok(pair(&s, &phone, secret(&second), &["memory:read"]).await).await;
    let denied: GrantSummary = ok(s
        .post_as(&host.access, &format!("v1/grants/{}/deny", again.id), None)
        .await)
    .await;
    assert_eq!(denied.state, GrantState::Denied);
    let listed = host_grants(&s, &host, &registered).await;
    assert_eq!(
        listed.iter().map(|g| g.id.clone()).collect::<Vec<_>>(),
        vec![given.id.clone()]
    );

    // The phone gives its grant back; the host no longer lists it.
    let revoked = s
        .post_as(
            &phone.access,
            &format!("v1/grants/{}/revoke", given.id),
            None,
        )
        .await;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    assert!(host_grants(&s, &host, &registered).await.is_empty());
    let mine: GrantSummary = ok(s
        .get_as(&phone.access, &format!("v1/grants/{}", given.id))
        .await)
    .await;
    assert_eq!(mine.state, GrantState::Revoked);

    // The host signs in again: the same host, now on its new device. The old one is not it.
    let again = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let moved: HostSummary =
        ok(register(&s, &again, json!({ "host": registered.id, "name": "Desk" })).await).await;
    assert_eq!(
        (moved.id.clone(), moved.name.as_str()),
        (registered.id.clone(), "Desk")
    );
    refused(
        s.post_as(
            &host.access,
            &format!("v1/hosts/{}/offers", registered.id),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;

    // A grant in force, then the host removed: it is revoked, and the host is gone from the list.
    let third = offer(&s, &again, &moved).await;
    let asked: GrantSummary = ok(pair(&s, &phone, code(&third), &["workspace:read"]).await).await;
    ok::<GrantSummary>(
        s.post_as(
            &again.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({ "scopes": ["workspace:read"], "projects": { "kind": "all" } })),
        )
        .await,
    )
    .await;
    let removed = s
        .post_as(
            &again.access,
            &format!("v1/hosts/{}/remove", moved.id),
            None,
        )
        .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let seen: HostPage = ok(s.get_as(&phone.access, "v1/hosts").await).await;
    assert!(seen.hosts.is_empty());
    let mine: GrantSummary = ok(s
        .get_as(&phone.access, &format!("v1/grants/{}", asked.id))
        .await)
    .await;
    assert_eq!(mine.state, GrantState::Revoked);
}

async fn another_account_reaches_none_of_it(records: impl Records + 'static) {
    let s = Served::start(records).await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;
    let bob = s.sign_in("bob", "Bob's phone", DeviceKind::Phone).await;
    let bobs_host = s.sign_in("bob", "Bob's host", DeviceKind::Host).await;
    let registered: HostSummary =
        ok(register(&s, &host, json!({ "name": "Workstation" })).await).await;
    let alices = offer(&s, &host, &registered).await;

    // Bob holding Alice's code or secret gets nothing, and spends nothing.
    for key in [code(&alices), secret(&alices)] {
        refused(
            pair(&s, &bob, key, &["workspace:read"]).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        )
        .await;
    }
    let bobs_hosts: HostPage = ok(s.get_as(&bob.access, "v1/hosts").await).await;
    assert!(bobs_hosts.hosts.is_empty());
    // Nor can Bob's host take Alice's host's id.
    refused(
        register(
            &s,
            &bobs_host,
            json!({ "host": registered.id, "name": "Mine" }),
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    for path in [
        format!("v1/hosts/{}/offers", registered.id),
        format!("v1/hosts/{}/remove", registered.id),
    ] {
        refused(
            s.post_as(&bobs_host.access, &path, None).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        )
        .await;
    }
    refused(
        s.get_as(
            &bobs_host.access,
            &format!("v1/hosts/{}/grants", registered.id),
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;

    // Alice's own phone still redeems it: Bob's misses were his account's, not hers.
    let asked: GrantSummary = ok(pair(&s, &phone, code(&alices), &["workspace:read"]).await).await;
    refused(
        s.get_as(&bob.access, &format!("v1/grants/{}", asked.id))
            .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    for path in [
        format!("v1/grants/{}/deny", asked.id),
        format!("v1/grants/{}/revoke", asked.id),
    ] {
        refused(
            s.post_as(&bobs_host.access, &path, None).await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        )
        .await;
    }
    refused(
        s.post_as(
            &bobs_host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({ "scopes": ["workspace:read"], "projects": { "kind": "all" } })),
        )
        .await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    let bobs_grants: GrantPage = ok(s.get_as(&bob.access, "v1/grants").await).await;
    assert!(bobs_grants.grants.is_empty());
    // And Alice's request still waits, untouched.
    let waiting = host_grants(&s, &host, &registered).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].state, GrantState::Pending);

    // A device signed out loses its grant: the host stops listing it.
    ok::<GrantSummary>(
        s.post_as(
            &host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({ "scopes": ["workspace:read"], "projects": { "kind": "all" } })),
        )
        .await,
    )
    .await;
    let signed_out = s
        .post_as(
            &host.access,
            &format!("v1/devices/{}/revoke", phone.device),
            None,
        )
        .await;
    assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);
    assert!(host_grants(&s, &host, &registered).await.is_empty());
}

#[tokio::test]
async fn wrong_codes_spend_the_accounts_open_offer() {
    let s = Served::start(MemoryRecords::default()).await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;
    let registered: HostSummary =
        ok(register(&s, &host, json!({ "name": "Workstation" })).await).await;
    let open = offer(&s, &host, &registered).await;
    let wrong = if open.code.expose() == "0000-0000" {
        "1111-1111"
    } else {
        "0000-0000"
    };
    for _ in 0..MAX_OFFER_MISSES {
        refused(
            pair(
                &s,
                &phone,
                json!({ "kind": "code", "value": wrong }),
                &["workspace:read"],
            )
            .await,
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
        )
        .await;
    }
    refused(
        pair(&s, &phone, code(&open), &["workspace:read"]).await,
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
    )
    .await;
    // A new offer works.
    let fresh = offer(&s, &host, &registered).await;
    ok::<GrantSummary>(pair(&s, &phone, code(&fresh), &["workspace:read"]).await).await;
}

#[tokio::test]
async fn malformed_pairings_are_refused_before_anything_is_spent() {
    let s = Served::start(MemoryRecords::default()).await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;
    let registered: HostSummary =
        ok(register(&s, &host, json!({ "name": "Workstation" })).await).await;
    let open = offer(&s, &host, &registered).await;
    for (offer, scopes) in [
        (code(&open), vec![]),
        (
            json!({ "kind": "code", "value": "ABC-123" }),
            vec!["workspace:read"],
        ),
        (
            json!({ "kind": "secret", "value": "" }),
            vec!["workspace:read"],
        ),
    ] {
        refused(
            pair(&s, &phone, offer, &scopes).await,
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
        )
        .await;
    }
    refused(
        s.post_as(
            &phone.access,
            "v1/pairings",
            Some(&json!({ "nonsense": true })),
        )
        .await,
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
    )
    .await;
    refused(
        s.client
            .post(s.url("v1/pairings"))
            .json(&json!({ "offer": code(&open), "scopes": ["workspace:read"] }))
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthenticated,
    )
    .await;
    refused(
        register(&s, &host, json!({ "name": "two\nlines" })).await,
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
    )
    .await;
    // None of that spent the offer.
    ok::<GrantSummary>(pair(&s, &phone, code(&open), &["workspace:read"]).await).await;
}

#[tokio::test]
async fn one_address_may_try_only_so_many_pairings_and_offers() {
    let s = Served::start_with(
        MemoryRecords::default(),
        http::Options {
            offers_per_minute: 2,
            pairings_per_minute: 2,
            ..http::Options::default()
        },
    )
    .await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let phone = s.sign_in("alice", "Phone", DeviceKind::Phone).await;
    let registered: HostSummary =
        ok(register(&s, &host, json!({ "name": "Workstation" })).await).await;
    offer(&s, &host, &registered).await;
    let last = offer(&s, &host, &registered).await;
    refused(
        s.post_as(
            &host.access,
            &format!("v1/hosts/{}/offers", registered.id),
            None,
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::RateLimited,
    )
    .await;
    let wrong = json!({ "kind": "code", "value": "ZZZZ-ZZZZ" });
    pair(&s, &phone, wrong.clone(), &["workspace:read"]).await;
    pair(&s, &phone, wrong, &["workspace:read"]).await;
    refused(
        pair(&s, &phone, code(&last), &["workspace:read"]).await,
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::RateLimited,
    )
    .await;
}

//! The server over HTTP, served on a real port: a whole sign-in from the device's first request to
//! revoking it, and what each route answers when the request is wrong. Run on each store, like
//! `records.rs`, with the stand-in identity provider in `common/idp.rs`.

mod common;

use common::idp::APP;
use common::served::*;
use remote_proto::account::{
    Credentials, DeviceKind, DevicePage, NewDevice, Secret, ServerDescription, TokenRequest,
};
use remote_proto::{ErrorCode, Platform};
use reqwest::header::{CACHE_CONTROL, LOCATION, WWW_AUTHENTICATE};
use reqwest::StatusCode;
use yardsort_server::records::{MemoryRecords, Records};
use yardsort_server::{http, tokens};

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

for_each_store!(a_device_signs_in_refreshes_and_is_revoked_over_http);

async fn a_device_signs_in_refreshes_and_is_revoked_over_http(records: impl Records + 'static) {
    let s = Served::start(records).await;

    let server: ServerDescription = s.get("v1/server").await.json().await.unwrap();
    assert_eq!(server.issuer, s.idp.issuer());

    // The device starts; the browser is sent to the provider.
    let verifier = tokens::random();
    let mut start = s.url("v1/login");
    start
        .query_pairs_mut()
        .append_pair("challenge", &tokens::s256(&verifier))
        .append_pair("challengeMethod", "S256")
        .append_pair("redirect", APP);
    let to_provider = s.client.get(start).send().await.unwrap();
    assert_eq!(to_provider.status(), StatusCode::SEE_OTHER);
    assert_eq!(to_provider.headers()[CACHE_CONTROL], "no-store");
    let authorize = location(&to_provider);
    assert_eq!(
        query(&authorize)["redirect_uri"],
        s.url("v1/login/callback").as_str()
    );

    // The provider sends the browser back, and the server sends it on to the device.
    let (code, state) = s.idp.approve(&authorize, "alice");
    let mut callback = s.url("v1/login/callback");
    callback
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &state);
    let to_device = s.client.get(callback).send().await.unwrap();
    assert_eq!(to_device.status(), StatusCode::SEE_OTHER);
    let back = location(&to_device);
    assert!(back.as_str().starts_with(APP), "{back}");
    let handoff = query(&back)["code"].clone();

    let signed_in = s
        .token(&TokenRequest::SignIn {
            code: Secret::new(handoff),
            verifier: Secret::new(verifier),
            device: NewDevice {
                name: "Phone".into(),
                kind: DeviceKind::Phone,
                platform: Platform::Ios,
            },
        })
        .await;
    assert_eq!(signed_in.status(), StatusCode::OK);
    assert_eq!(signed_in.headers()[CACHE_CONTROL], "no-store");
    let credentials: Credentials = signed_in.json().await.unwrap();

    let page: DevicePage = s
        .devices(Some(&credentials.access))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page.devices.len(), 1);
    assert!(page.devices[0].current);

    let unsigned = s.devices(None).await;
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(unsigned.headers()[WWW_AUTHENTICATE], "Bearer");
    assert_eq!(error(unsigned).await.code, ErrorCode::Unauthenticated);

    let refreshed = s
        .token(&TokenRequest::Refresh {
            refresh: credentials.refresh.clone(),
        })
        .await;
    assert_eq!(refreshed.status(), StatusCode::OK);
    let renewed: Credentials = refreshed.json().await.unwrap();
    assert_eq!(renewed.device, credentials.device);

    let revoke = |id: &str, access: &Secret| {
        s.client
            .post(s.url(&format!("v1/devices/{id}/revoke")))
            .bearer_auth(access.expose())
            .send()
    };
    let unknown = revoke("no-such-device", &renewed.access).await.unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let revoked = revoke(credentials.device.as_str(), &renewed.access)
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        s.devices(Some(&renewed.access)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn wrong_requests_get_the_matching_status_and_nothing_cached() {
    let s = Served::start(MemoryRecords::default()).await;

    let incomplete = s.get("v1/login?challenge=x").await;
    assert_eq!(incomplete.status(), StatusCode::BAD_REQUEST);
    assert!(incomplete.text().await.unwrap().contains("incomplete"));

    let elsewhere = s
        .get(&format!(
            "v1/login?challenge={}&challengeMethod=S256&redirect=https://attacker.example.invalid/",
            tokens::s256(&tokens::random())
        ))
        .await;
    assert_eq!(elsewhere.status(), StatusCode::BAD_REQUEST);
    assert!(elsewhere.headers().get(LOCATION).is_none());

    let stale = s.get("v1/login/callback?code=c&state=never-issued").await;
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
    assert!(stale.headers().get(LOCATION).is_none());

    let not_json = s
        .client
        .post(s.url("v1/token"))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(not_json.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error(not_json).await.code, ErrorCode::InvalidRequest);

    let forged = s
        .token(&TokenRequest::Refresh {
            refresh: Secret::new("ys1.rt.not-one-the-server-issued"),
        })
        .await;
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);

    let huge = s
        .client
        .post(s.url("v1/token"))
        .header("content-type", "application/json")
        .body(vec![b' '; http::MAX_BODY + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(huge.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let nowhere = s.get("v2/anything").await;
    assert_eq!(nowhere.status(), StatusCode::NOT_FOUND);
    assert_eq!(nowhere.headers()[CACHE_CONTROL], "no-store");
    assert_eq!(error(nowhere).await.code, ErrorCode::NotFound);

    assert_eq!(s.get("health").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn one_address_may_start_only_so_many_sign_ins_and_token_requests() {
    let s = Served::start_with(
        MemoryRecords::default(),
        http::Options {
            client_ip_header: Some("fly-client-ip".parse().unwrap()),
            logins_per_minute: 2,
            tokens_per_minute: 3,
            ..http::Options::default()
        },
    )
    .await;
    let start = |from: &'static str| {
        let challenge = tokens::s256(&tokens::random());
        s.client
            .get(s.url(&format!(
                "v1/login?challenge={challenge}&challengeMethod=S256&redirect={APP}"
            )))
            .header("fly-client-ip", from)
            .send()
    };
    for _ in 0..2 {
        assert_eq!(
            start("192.0.2.1").await.unwrap().status(),
            StatusCode::SEE_OTHER
        );
    }
    let refused = start("192.0.2.1").await.unwrap();
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.headers()["retry-after"], "30");
    // Another address, as the proxy names it, is counted on its own.
    assert_eq!(
        start("192.0.2.2").await.unwrap().status(),
        StatusCode::SEE_OTHER
    );

    // Without the header, the connection's own address counts: here, loopback.
    let forged = TokenRequest::Refresh {
        refresh: Secret::new("ys1.rt.not-one-the-server-issued"),
    };
    for _ in 0..3 {
        assert_eq!(s.token(&forged).await.status(), StatusCode::UNAUTHORIZED);
    }
    let refused = s.token(&forged).await;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error(refused).await.code, ErrorCode::RateLimited);
    // Other routes are not limited.
    assert_eq!(s.get("v1/server").await.status(), StatusCode::OK);
}

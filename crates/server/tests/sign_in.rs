//! Signing in, through the control service, against the stand-in identity provider in
//! `common/idp.rs`. Each test that makes the provider misbehave checks that nobody was signed in.

mod common;

use common::idp::*;
use jsonwebtoken::{Algorithm, EncodingKey};
use remote_proto::account::{
    Credentials, DeviceKind, LoginParams, NewDevice, Secret, TokenRequest,
};
use remote_proto::{DeviceId, ErrorCode, Platform, RemoteError, ServerId};
use url::Url;
use yardsort_server::oidc::{self, ProviderConfig, KEYS_REFETCH_MS, PROVIDER_MAX_AGE_MS};
use yardsort_server::records::{Device, Issued, MemoryRecords, Records};
use yardsort_server::{
    tokens, Caller, ControlService, LoginFinish, ServerConfig, ACCESS_TTL_MS, HANDOFF_TTL_MS,
    LOGIN_TTL_MS, MAX_DEVICES_PER_ACCOUNT, REFRESH_TTL_MS,
};

struct Setup {
    idp: Idp,
    service: ControlService<MemoryRecords>,
}

async fn setup(secret: Option<&str>) -> Setup {
    let idp = Idp::start(secret).await;
    let config = ServerConfig {
        server: ServerId::new("srv-test").unwrap(),
        name: "Test server".into(),
        public_url: Url::parse("https://yardsort.example.invalid").unwrap(),
        provider: ProviderConfig {
            issuer: idp.issuer(),
            client_id: CLIENT_ID.into(),
            client_secret: secret.map(Secret::new),
        },
        device_redirects: vec![APP.into(), "http://127.0.0.1/signed-in".into()],
    };
    let http = oidc::http_client_builder().no_proxy().build().unwrap();
    let service = ControlService::new(config, MemoryRecords::default(), http).unwrap();
    Setup { idp, service }
}

fn device(name: &str) -> NewDevice {
    NewDevice {
        name: name.into(),
        kind: DeviceKind::Phone,
        platform: Platform::Android,
    }
}

/// A device's side of starting: its verifier, kept, and the sign-in it asked for.
fn params(verifier: &str, redirect: &str) -> LoginParams {
    LoginParams {
        challenge: tokens::s256(verifier),
        challenge_method: "S256".into(),
        redirect: redirect.into(),
    }
}

fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Through the browser: start, sign in at the provider as `subject`, and come back. The device's
/// verifier and the one-time code its redirect received.
async fn browse(s: &Setup, subject: &str, now: i64) -> (String, String) {
    let verifier = tokens::random();
    let authorize = s
        .service
        .begin_login(&params(&verifier, APP), now)
        .await
        .unwrap();
    let (code, state) = s.idp.approve(&authorize, subject);
    match s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    {
        LoginFinish::SignedIn(back) => {
            assert!(back.as_str().starts_with(APP), "{back}");
            (verifier, query(&back, "code").unwrap())
        }
        other => panic!("{other:?}"),
    }
}

async fn redeem(
    s: &Setup,
    code: &str,
    verifier: &str,
    name: &str,
    now: i64,
) -> Result<Credentials, RemoteError> {
    s.service
        .token(
            &TokenRequest::SignIn {
                code: Secret::new(code),
                verifier: Secret::new(verifier),
                device: device(name),
            },
            now,
        )
        .await
}

async fn sign_in(s: &Setup, subject: &str, name: &str, now: i64) -> Credentials {
    let (verifier, code) = browse(s, subject, now).await;
    redeem(s, &code, &verifier, name, now).await.unwrap()
}

async fn refresh(s: &Setup, refresh: &Secret, now: i64) -> Result<Credentials, RemoteError> {
    s.service
        .token(
            &TokenRequest::Refresh {
                refresh: refresh.clone(),
            },
            now,
        )
        .await
}

async fn caller(s: &Setup, credentials: &Credentials, now: i64) -> Result<Caller, RemoteError> {
    s.service
        .authenticate(credentials.access.expose(), now)
        .await
}

fn code_of<T: std::fmt::Debug>(result: Result<T, RemoteError>) -> ErrorCode {
    result.unwrap_err().code
}

#[tokio::test]
async fn signing_in_asks_the_provider_once_and_the_device_then_holds_the_servers_own_credentials() {
    let s = setup(Some("a secret: with % and +")).await;
    let now = now_ms();

    // The provider is sent only the server's own callback.
    let verifier = tokens::random();
    let authorize = s
        .service
        .begin_login(&params(&verifier, APP), now)
        .await
        .unwrap();
    assert_eq!(
        query(&authorize, "redirect_uri").as_deref(),
        Some(s.service.callback_url().as_str())
    );
    assert_eq!(
        s.service.callback_url().as_str(),
        "https://yardsort.example.invalid/v1/login/callback"
    );
    assert_ne!(
        query(&authorize, "code_challenge"),
        Some(tokens::s256(&verifier)),
        "the device's challenge and the provider's are separate"
    );
    let (code, state) = s.idp.approve(&authorize, "subject-1");
    let LoginFinish::SignedIn(back) = s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    else {
        panic!()
    };
    let pairs: Vec<_> = back.query_pairs().map(|(k, _)| k.into_owned()).collect();
    assert_eq!(pairs, ["code"], "only the one-time code goes to the device");
    let credentials = redeem(&s, &query(&back, "code").unwrap(), &verifier, "Phone", now)
        .await
        .unwrap();
    assert!(credentials.access.expose().starts_with("ys1.at."));
    assert_eq!(credentials.access_expires_in_ms, ACCESS_TTL_MS as u64);

    let me = caller(&s, &credentials, now).await.unwrap();
    assert_eq!(me.device, credentials.device);
    let page = s.service.devices(&me).await.unwrap();
    assert_eq!(page.devices.len(), 1);
    assert!(page.devices[0].current);
    assert_eq!(page.devices[0].name, "Phone");
    assert_eq!(page.devices[0].last_seen_at, Some(now));
    assert_eq!(s.idp.calls(), (1, 1));

    // A month of use: refreshing and calling never reach the provider, so a provider that bills
    // by active user counts this device once, for the day it signed in.
    let mut held = credentials;
    for day in 1..=30 {
        let at = now + day * 24 * 60 * 60_000;
        held = refresh(&s, &held.refresh, at).await.unwrap();
        caller(&s, &held, at).await.unwrap();
    }
    assert_eq!(
        s.idp.calls(),
        (1, 1),
        "the provider heard from no device after sign-in"
    );
}

#[tokio::test]
async fn a_public_client_sends_its_id_and_no_secret() {
    let s = setup(None).await;
    let credentials = sign_in(&s, "subject-1", "Phone", now_ms()).await;
    assert!(caller(&s, &credentials, now_ms()).await.is_ok());
}

#[tokio::test]
async fn an_account_is_a_subject_and_sees_and_revokes_only_its_own_devices() {
    let s = setup(None).await;
    let now = now_ms();
    let phone = sign_in(&s, "subject-1", "Phone", now).await;
    let laptop = sign_in(&s, "subject-1", "Laptop", now + 1).await;
    let theirs = sign_in(&s, "subject-2", "Their phone", now + 2).await;
    assert_eq!(s.service.records().account_count(), 2);

    let me = caller(&s, &phone, now).await.unwrap();
    let mine = s.service.devices(&me).await.unwrap();
    let names: Vec<_> = mine.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        ["Laptop", "Phone"],
        "same subject, same account; newest first"
    );

    let them = caller(&s, &theirs, now).await.unwrap();
    let their_list = s.service.devices(&them).await.unwrap();
    assert_eq!(their_list.devices.len(), 1);
    assert_eq!(their_list.devices[0].id, theirs.device);

    // Another account's device is not found, exactly as a device that does not exist, and is
    // left working.
    let missing = DeviceId::new("no-such-device").unwrap();
    let not_mine = s.service.revoke(&me, &theirs.device, now).await;
    let nothing = s.service.revoke(&me, &missing, now).await;
    assert_eq!(not_mine, nothing);
    assert_eq!(code_of(not_mine), ErrorCode::NotFound);
    assert!(caller(&s, &theirs, now).await.is_ok());
    assert!(refresh(&s, &theirs.refresh, now).await.is_ok());

    // Its own other device it can revoke, and that device stops at once.
    s.service.revoke(&me, &laptop.device, now).await.unwrap();
    assert_eq!(
        code_of(caller(&s, &laptop, now).await),
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        code_of(refresh(&s, &laptop.refresh, now).await),
        ErrorCode::Unauthenticated
    );
    let listed = s.service.devices(&me).await.unwrap();
    let laptop_row = listed
        .devices
        .iter()
        .find(|d| d.id == laptop.device)
        .unwrap();
    assert_eq!(laptop_row.revoked_at, Some(now));
    assert!(caller(&s, &phone, now).await.is_ok());
}

#[tokio::test]
async fn a_sign_in_code_works_once_briefly_and_only_with_its_verifier() {
    let s = setup(None).await;
    let now = now_ms();

    let (verifier, code) = browse(&s, "subject-1", now).await;
    let wrong = tokens::random();
    assert_eq!(
        code_of(redeem(&s, &code, &wrong, "Phone", now).await),
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        code_of(redeem(&s, &code, &verifier, "Phone", now).await),
        ErrorCode::Unauthenticated,
        "a wrong verifier spends the code: there is no second guess"
    );

    let (verifier, code) = browse(&s, "subject-1", now).await;
    assert!(redeem(&s, &code, &verifier, "Phone", now).await.is_ok());
    assert!(
        redeem(&s, &code, &verifier, "Phone", now).await.is_err(),
        "used once"
    );

    let (verifier, code) = browse(&s, "subject-1", now).await;
    assert!(
        redeem(&s, &code, &verifier, "Phone", now + HANDOFF_TTL_MS)
            .await
            .is_err(),
        "a minute at most"
    );

    // Something that is not a sign-in code at all, such as a refresh credential.
    let credentials = sign_in(&s, "subject-1", "Phone", now).await;
    assert!(
        redeem(&s, credentials.refresh.expose(), &verifier, "Phone", now)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_providers_answer_counts_once_and_only_while_the_sign_in_is_fresh() {
    let s = setup(None).await;
    let now = now_ms();
    let start = || async {
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), now)
            .await
            .unwrap();
        s.idp.approve(&authorize, "subject-1")
    };

    let (code, state) = start().await;
    assert!(matches!(
        s.service
            .finish_login(Some(&code), Some(&state), None, now)
            .await,
        LoginFinish::SignedIn(_)
    ));
    assert!(
        matches!(
            s.service
                .finish_login(Some(&code), Some(&state), None, now)
                .await,
            LoginFinish::Unknown(_)
        ),
        "the same answer twice"
    );
    assert!(matches!(
        s.service
            .finish_login(Some(&code), Some("a-state-never-issued"), None, now)
            .await,
        LoginFinish::Unknown(_)
    ));

    let (code, state) = start().await;
    let late = s
        .service
        .finish_login(Some(&code), Some(&state), None, now + LOGIN_TTL_MS)
        .await;
    let LoginFinish::Refused { to, .. } = late else {
        panic!("{late:?}")
    };
    assert_eq!(query(&to, "error").as_deref(), Some("unauthenticated"));
    assert_eq!(query(&to, "code"), None);

    // The person cancelled at the provider.
    let (_, state) = start().await;
    let cancelled = s
        .service
        .finish_login(None, Some(&state), Some("access_denied"), now)
        .await;
    let LoginFinish::Refused { to, why } = cancelled else {
        panic!("{cancelled:?}")
    };
    assert!(to.as_str().starts_with(APP));
    assert!(why.contains("access_denied"));
    assert_eq!(s.service.records().account_count(), 1);
}

#[tokio::test]
async fn a_sign_in_goes_back_only_to_an_allowed_redirect() {
    let s = setup(None).await;
    let now = now_ms();
    for redirect in [
        "https://attacker.example.invalid/catch",
        "yardsort://signed-in/elsewhere",
        "yardsort://signed-in#fragment",
        "http://localhost:4000/signed-in",
        "http://127.0.0.1:4000/other",
        "not a url",
    ] {
        let refused = s
            .service
            .begin_login(&params(&tokens::random(), redirect), now)
            .await;
        assert_eq!(code_of(refused), ErrorCode::InvalidRequest, "{redirect}");
    }
    assert_eq!(s.idp.calls(), (0, 0));

    // A computer's client on whatever port it found free.
    let verifier = tokens::random();
    let loopback = "http://127.0.0.1:49731/signed-in";
    let authorize = s
        .service
        .begin_login(&params(&verifier, loopback), now)
        .await
        .unwrap();
    let (code, state) = s.idp.approve(&authorize, "subject-1");
    let LoginFinish::SignedIn(back) = s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    else {
        panic!()
    };
    assert!(back.as_str().starts_with(loopback));
}

#[tokio::test]
async fn an_id_token_is_checked_before_anyone_is_signed_in() {
    for tamper in [
        Tamper::Issuer,
        Tamper::Audience,
        Tamper::OtherAzp,
        Tamper::SeveralAudiencesNoAzp,
        Tamper::Nonce,
        Tamper::NoNonce,
        Tamper::Expired,
        Tamper::FromTheFuture,
        Tamper::UnpublishedKey,
        Tamper::UnknownKeyId,
        Tamper::AlgNone,
        Tamper::HmacWithThePublicKey,
        Tamper::RsaSignatureUnderAnEcKeyId,
    ] {
        let s = setup(None).await;
        let now = now_ms();
        s.idp.with(|i| i.tamper = tamper);
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), now)
            .await
            .unwrap();
        let (code, state) = s.idp.approve(&authorize, "subject-1");
        let finish = s
            .service
            .finish_login(Some(&code), Some(&state), None, now)
            .await;
        let LoginFinish::Refused { to, why } = finish else {
            panic!("{tamper:?} was accepted: {finish:?}")
        };
        assert_eq!(query(&to, "code"), None, "{tamper:?}");
        assert!(why.contains("ID token"), "{tamper:?}: {why}");
        assert_eq!(s.service.records().account_count(), 0, "{tamper:?}");
    }
}

#[tokio::test]
async fn either_kind_of_published_key_signs_a_person_in() {
    let s = setup(None).await;
    s.idp.with(|i| {
        i.signer = Signer {
            kid: "ec-p256",
            alg: Algorithm::ES256,
            key: EncodingKey::from_ec_der(EC),
        }
    });
    let credentials = sign_in(&s, "subject-1", "Phone", now_ms()).await;
    assert!(caller(&s, &credentials, now_ms()).await.is_ok());
}

#[tokio::test]
async fn new_provider_keys_are_fetched_but_unknown_ones_cannot_make_the_server_hammer_it() {
    let s = setup(None).await;
    let now = now_ms();
    sign_in(&s, "subject-1", "Phone", now).await;
    assert_eq!(s.idp.calls().1, 1);

    // The provider rotates to a key the server has not seen.
    s.idp.with(|i| {
        i.published.push(jwk("rsa-b"));
        i.signer = Signer {
            kid: "rsa-b",
            alg: Algorithm::RS256,
            key: EncodingKey::from_rsa_der(RSA_B),
        };
    });
    let later = now + KEYS_REFETCH_MS;
    sign_in(&s, "subject-1", "Phone", later).await;
    assert_eq!(s.idp.calls().1, 2, "fetched again for the new key");

    // Tokens naming a key nobody publishes, one after another: at most one fetch a minute.
    s.idp.with(|i| i.tamper = Tamper::UnknownKeyId);
    for n in 0..5 {
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), later + KEYS_REFETCH_MS + n)
            .await
            .unwrap();
        let (code, state) = s.idp.approve(&authorize, "subject-1");
        let finish = s
            .service
            .finish_login(Some(&code), Some(&state), None, later + KEYS_REFETCH_MS + n)
            .await;
        assert!(matches!(finish, LoginFinish::Refused { .. }));
    }
    assert_eq!(s.idp.calls().1, 3);
}

#[tokio::test]
async fn a_key_the_provider_withdraws_stops_signing_anyone_in_within_the_hour() {
    let s = setup(None).await;
    let now = now_ms();
    sign_in(&s, "subject-1", "Phone", now).await;
    assert_eq!(s.idp.calls().1, 1);

    // The provider withdraws its key, but a copy of it still signs tokens.
    s.idp.with(|i| i.published = vec![jwk("rsa-b")]);
    let soon = now + PROVIDER_MAX_AGE_MS - 1;
    sign_in(&s, "subject-1", "Phone", soon).await;
    assert_eq!(s.idp.calls().1, 1, "the key set is still fresh");

    let later = now + PROVIDER_MAX_AGE_MS;
    let authorize = s
        .service
        .begin_login(&params(&tokens::random(), APP), later)
        .await
        .unwrap();
    let (code, state) = s.idp.approve(&authorize, "subject-1");
    let finish = s
        .service
        .finish_login(Some(&code), Some(&state), None, later)
        .await;
    assert!(matches!(finish, LoginFinish::Refused { .. }));
    assert_eq!(s.idp.calls().1, 2, "fetched again once an hour old");
}

#[tokio::test]
async fn a_provider_that_names_another_issuer_is_not_used() {
    let s = setup(None).await;
    s.idp
        .with(|i| i.discovery_issuer = Some("https://auth.example.invalid".into()));
    let refused = s
        .service
        .begin_login(&params(&tokens::random(), APP), now_ms())
        .await;
    assert_eq!(code_of(refused), ErrorCode::Internal);

    let config = |issuer: &str| ServerConfig {
        server: ServerId::new("srv-test").unwrap(),
        name: "Test server".into(),
        public_url: Url::parse("https://yardsort.example.invalid").unwrap(),
        provider: ProviderConfig {
            issuer: issuer.into(),
            client_id: CLIENT_ID.into(),
            client_secret: None,
        },
        device_redirects: vec![],
    };
    let http = oidc::http_client().unwrap();
    assert!(ControlService::new(
        config("http://auth.example.invalid"),
        MemoryRecords::default(),
        http.clone()
    )
    .is_err());
    assert!(ControlService::new(
        config("https://auth.example.invalid"),
        MemoryRecords::default(),
        http
    )
    .is_ok());
}

#[tokio::test]
async fn an_account_may_have_only_so_many_devices_signed_in() {
    let s = setup(None).await;
    let now = now_ms();
    // Devices unused for as long as a refresh lasts no longer count.
    let records = s.service.records();
    let account = records
        .account(&s.idp.issuer(), "subject-1", now)
        .await
        .unwrap();
    for n in 0..MAX_DEVICES_PER_ACCOUNT {
        let long_ago = now - REFRESH_TTL_MS;
        let device = Device {
            id: DeviceId::new(format!("unused-{n}")).unwrap(),
            account: account.clone(),
            name: format!("Unused {n}"),
            kind: DeviceKind::Phone,
            platform: Platform::Ios,
            created_at: long_ago,
            last_seen_at: None,
            revoked: None,
        };
        let issued = Issued {
            access: tokens::hash(&tokens::random()),
            access_expires_at: long_ago + ACCESS_TTL_MS,
            refresh: tokens::hash(&tokens::random()),
            refresh_expires_at: long_ago + REFRESH_TTL_MS,
        };
        records.add_device(device, issued).await.unwrap();
    }

    let mut first = None;
    for n in 0..MAX_DEVICES_PER_ACCOUNT {
        let credentials = sign_in(&s, "subject-1", &format!("Device {n}"), now).await;
        first.get_or_insert(credentials);
    }
    let (verifier, code) = browse(&s, "subject-1", now).await;
    let refused = redeem(&s, &code, &verifier, "One too many", now).await;
    assert_eq!(code_of(refused), ErrorCode::Forbidden);
    // Someone else is counted on their own.
    sign_in(&s, "subject-2", "Phone", now).await;

    // Signing one out makes room.
    let first = first.unwrap();
    let me = caller(&s, &first, now).await.unwrap();
    s.service.revoke(&me, &first.device, now).await.unwrap();
    sign_in(&s, "subject-1", "Replacement", now).await;
}

#[tokio::test]
async fn a_refresh_credential_works_once_and_a_copy_signs_the_device_out() {
    let s = setup(None).await;
    let now = now_ms();
    let first = sign_in(&s, "subject-1", "Phone", now).await;
    let other = sign_in(&s, "subject-1", "Laptop", now).await;

    let second = refresh(&s, &first.refresh, now + 1).await.unwrap();
    assert_eq!(second.device, first.device);
    assert_ne!(second.refresh, first.refresh);
    assert!(caller(&s, &second, now + 1).await.is_ok());

    // The spent one again: only a copy could hold it, so the device is signed out.
    assert_eq!(
        code_of(refresh(&s, &first.refresh, now + 2).await),
        ErrorCode::Unauthenticated
    );
    assert!(caller(&s, &second, now + 2).await.is_err());
    assert!(refresh(&s, &second.refresh, now + 2).await.is_err());
    let me = caller(&s, &other, now + 2).await.unwrap();
    let listed = s.service.devices(&me).await.unwrap();
    let phone = listed
        .devices
        .iter()
        .find(|d| d.id == first.device)
        .unwrap();
    assert_eq!(phone.revoked_at, Some(now + 2));
    assert!(
        caller(&s, &other, now + 2).await.is_ok(),
        "only that device"
    );

    // Lapsed: an access after its quarter hour, a refresh after its months unused.
    assert!(caller(&s, &other, now + ACCESS_TTL_MS).await.is_err());
    assert!(refresh(&s, &other.refresh, now + REFRESH_TTL_MS)
        .await
        .is_err());
    // Credentials of one kind are not accepted as another.
    assert!(s
        .service
        .authenticate(other.refresh.expose(), now)
        .await
        .is_err());
    assert!(refresh(&s, &other.access, now).await.is_err());
}

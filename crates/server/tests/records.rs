//! What every [`Records`] store must do, checked against each: [`MemoryRecords`] always, and
//! [`PgRecords`] against a real PostgreSQL when `YARDSORT_TEST_DATABASE_URL` names one (see
//! `common/pg.rs`).

mod common;

use std::sync::Arc;

use remote_proto::account::{DeviceKind, Secret};
use remote_proto::{DeviceId, Platform};
use yardsort_server::postgres::PgRecords;
use yardsort_server::records::{
    AccountId, Device, Handoff, Issued, MemoryRecords, PendingLogin, Records, RevokeReason,
    Revoked, Rotation, SEEN_RESOLUTION_MS,
};
use yardsort_server::tokens::{self, TokenHash};

/// Each test, once per store.
macro_rules! for_each_store {
    ($($name:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(std::sync::Arc::new(super::MemoryRecords::default())).await;
                }
            )*
        }
        mod postgres {
            $(
                #[tokio::test]
                async fn $name() {
                    if let Some(db) = super::common::pg::database().await {
                        super::$name(db.records.clone()).await;
                        db.drop().await;
                    }
                }
            )*
        }
    };
}

for_each_store!(
    an_account_is_one_issuer_and_subject,
    a_sign_in_in_progress_is_taken_once,
    lapsed_sign_ins_and_handoffs_are_dropped,
    a_handoff_is_taken_once,
    an_access_credential_finds_its_device,
    a_refresh_credential_rotates_once_and_a_copy_revokes_the_device,
    a_spent_refresh_credential_is_remembered_through_later_rotations,
    a_lapsed_refresh_credential_is_expired,
    racing_rotations_of_one_credential_let_exactly_one_through,
    seen_is_written_at_most_once_a_minute,
    an_account_lists_and_revokes_only_its_own_devices,
    signed_in_devices_hold_a_live_refresh_credential,
    devices_keep_every_wire_name,
);

const T: i64 = 1_800_000_000_000;

fn h(text: &str) -> TokenHash {
    tokens::hash(text)
}

fn device_id() -> DeviceId {
    DeviceId::new(uuid::Uuid::new_v4().to_string()).unwrap()
}

fn issued(name: &str, now: i64) -> Issued {
    Issued {
        access: h(&format!("{name}-access")),
        access_expires_at: now + 1000,
        refresh: h(&format!("{name}-refresh")),
        refresh_expires_at: now + 10_000,
    }
}

async fn enroll(records: &impl Records, account: &AccountId, name: &str, now: i64) -> Device {
    let device = Device {
        id: device_id(),
        account: account.clone(),
        name: name.into(),
        kind: DeviceKind::Phone,
        platform: Platform::Ios,
        created_at: now,
        last_seen_at: None,
        revoked: None,
    };
    records
        .add_device(device.clone(), issued(name, now))
        .await
        .unwrap();
    device
}

async fn an_account_is_one_issuer_and_subject(records: Arc<impl Records>) {
    let a = records.account("https://id.one", "alice", T).await.unwrap();
    assert_eq!(
        records
            .account("https://id.one", "alice", T + 1)
            .await
            .unwrap(),
        a
    );
    let other_subject = records.account("https://id.one", "bob", T).await.unwrap();
    let other_issuer = records.account("https://id.two", "alice", T).await.unwrap();
    assert_ne!(other_subject, a);
    assert_ne!(other_issuer, a);
    assert_ne!(other_issuer, other_subject);
}

fn login(state: &str, expires_at: i64) -> PendingLogin {
    PendingLogin {
        state: h(state),
        nonce: h(&format!("{state}-nonce")),
        provider_verifier: Secret::new(format!("{state}-verifier")),
        device_challenge: format!("{state}-challenge"),
        device_redirect: "yardsort://signed-in".into(),
        expires_at,
    }
}

async fn a_sign_in_in_progress_is_taken_once(records: Arc<impl Records>) {
    records.put_login(login("a", T + 600), T).await.unwrap();
    records.put_login(login("b", T + 600), T).await.unwrap();
    let a = records.take_login(&h("a")).await.unwrap().unwrap();
    assert_eq!(a.nonce, h("a-nonce"));
    assert_eq!(a.provider_verifier.expose(), "a-verifier");
    assert_eq!(a.device_challenge, "a-challenge");
    assert_eq!(a.device_redirect, "yardsort://signed-in");
    assert_eq!(a.expires_at, T + 600);
    assert!(records.take_login(&h("a")).await.unwrap().is_none());
    assert!(records.take_login(&h("b")).await.unwrap().is_some());
    assert!(records.take_login(&h("never")).await.unwrap().is_none());
}

async fn lapsed_sign_ins_and_handoffs_are_dropped(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    records.put_login(login("old", T + 10), T).await.unwrap();
    records
        .put_handoff(handoff("old", &account, T + 10), T)
        .await
        .unwrap();
    // Taken before anything else is put: lapsed, but still there for the caller to judge.
    records.put_login(login("kept", T + 10), T).await.unwrap();
    assert!(records.take_login(&h("kept")).await.unwrap().is_some());

    records
        .put_login(login("new", T + 600), T + 20)
        .await
        .unwrap();
    records
        .put_handoff(handoff("new", &account, T + 600), T + 20)
        .await
        .unwrap();
    assert!(records.take_login(&h("old")).await.unwrap().is_none());
    assert!(records.take_handoff(&h("old")).await.unwrap().is_none());
    assert!(records.take_login(&h("new")).await.unwrap().is_some());
}

fn handoff(code: &str, account: &AccountId, expires_at: i64) -> Handoff {
    Handoff {
        code: h(code),
        account: account.clone(),
        device_challenge: format!("{code}-challenge"),
        expires_at,
    }
}

async fn a_handoff_is_taken_once(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    records
        .put_handoff(handoff("c", &account, T + 60), T)
        .await
        .unwrap();
    let taken = records.take_handoff(&h("c")).await.unwrap().unwrap();
    assert_eq!(taken.account, account);
    assert_eq!(taken.device_challenge, "c-challenge");
    assert_eq!(taken.expires_at, T + 60);
    assert!(records.take_handoff(&h("c")).await.unwrap().is_none());
}

async fn an_access_credential_finds_its_device(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let phone = enroll(&*records, &account, "phone", T).await;
    let (found, expires_at) = records.access(&h("phone-access")).await.unwrap().unwrap();
    assert_eq!(found, phone);
    assert_eq!(expires_at, T + 1000);
    assert!(records.access(&h("phone-refresh")).await.unwrap().is_none());
    assert!(records.access(&h("nothing")).await.unwrap().is_none());
}

async fn a_refresh_credential_rotates_once_and_a_copy_revokes_the_device(
    records: Arc<impl Records>,
) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let phone = enroll(&*records, &account, "phone", T).await;

    let second = issued("second", T + 100);
    match records
        .rotate(&h("phone-refresh"), second, T + 100)
        .await
        .unwrap()
    {
        Rotation::Rotated(device) => assert_eq!(device.id, phone.id),
        other => panic!("{other:?}"),
    }
    let (found, _) = records.access(&h("second-access")).await.unwrap().unwrap();
    assert_eq!(found.id, phone.id);

    // The first one again: only a copy could still hold it.
    let third = issued("third", T + 200);
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), third, T + 200)
            .await
            .unwrap(),
        Rotation::Reused
    );
    let (found, _) = records.access(&h("second-access")).await.unwrap().unwrap();
    assert_eq!(
        found.revoked,
        Some(Revoked {
            at: T + 200,
            reason: RevokeReason::RefreshReused
        })
    );
    // And the current one stops too.
    assert_eq!(
        records
            .rotate(&h("second-refresh"), issued("fourth", T + 300), T + 300)
            .await
            .unwrap(),
        Rotation::Revoked
    );
    assert!(records.access(&h("fourth-access")).await.unwrap().is_none());
    assert_eq!(
        records
            .rotate(&h("nothing"), issued("fifth", T + 300), T + 300)
            .await
            .unwrap(),
        Rotation::Unknown
    );
}

/// A store may tidy spent credentials as it rotates, but not before they lapse: a copy presented
/// after several rotations must still revoke the device.
async fn a_spent_refresh_credential_is_remembered_through_later_rotations(
    records: Arc<impl Records>,
) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    let mut current = "phone".to_owned();
    for (n, at) in [T + 100, T + 2_000, T + 5_000].into_iter().enumerate() {
        let next = format!("rotation-{n}");
        let rotated = records
            .rotate(&h(&format!("{current}-refresh")), issued(&next, at), at)
            .await
            .unwrap();
        assert!(matches!(rotated, Rotation::Rotated(_)), "{rotated:?}");
        current = next;
    }
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), issued("copy", T + 6_000), T + 6_000)
            .await
            .unwrap(),
        Rotation::Reused
    );
}

async fn a_lapsed_refresh_credential_is_expired(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    let late = T + 10_000;
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), issued("late", late), late)
            .await
            .unwrap(),
        Rotation::Expired
    );
    assert!(records.access(&h("late-access")).await.unwrap().is_none());
}

async fn racing_rotations_of_one_credential_let_exactly_one_through(
    records: Arc<impl Records + 'static>,
) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    let racers: Vec<_> = (0..8)
        .map(|i| {
            let records = records.clone();
            tokio::spawn(async move {
                records
                    .rotate(
                        &h("phone-refresh"),
                        issued(&format!("racer-{i}"), T + 1),
                        T + 1,
                    )
                    .await
                    .unwrap()
            })
        })
        .collect();
    let mut through = 0;
    for racer in racers {
        match racer.await.unwrap() {
            Rotation::Rotated(_) => through += 1,
            Rotation::Reused | Rotation::Revoked => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(through, 1);
}

async fn seen_is_written_at_most_once_a_minute(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let phone = enroll(&*records, &account, "phone", T).await;
    let last_seen = || async { records.devices(&account).await.unwrap()[0].last_seen_at };
    assert_eq!(last_seen().await, None);
    records.seen(&phone.id, T + 1).await.unwrap();
    assert_eq!(last_seen().await, Some(T + 1));
    records.seen(&phone.id, T + 2).await.unwrap();
    assert_eq!(last_seen().await, Some(T + 1));
    records
        .seen(&phone.id, T + 1 + SEEN_RESOLUTION_MS)
        .await
        .unwrap();
    assert_eq!(last_seen().await, Some(T + 1 + SEEN_RESOLUTION_MS));
}

async fn an_account_lists_and_revokes_only_its_own_devices(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;
    let laptop = enroll(&*records, &alice, "laptop", T + 1).await;
    let bobs = enroll(&*records, &bob, "bobs", T).await;

    let mut ids: Vec<_> = records
        .devices(&alice)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    ids.sort();
    let mut expected = vec![phone.id.clone(), laptop.id.clone()];
    expected.sort();
    assert_eq!(ids, expected);

    let by_owner = |at| Revoked {
        at,
        reason: RevokeReason::ByOwner,
    };
    assert!(!records
        .revoke(&alice, &bobs.id, by_owner(T + 5))
        .await
        .unwrap());
    assert_eq!(records.devices(&bob).await.unwrap()[0].revoked, None);
    assert!(!records
        .revoke(&alice, &device_id(), by_owner(T + 5))
        .await
        .unwrap());

    assert!(records
        .revoke(&alice, &phone.id, by_owner(T + 5))
        .await
        .unwrap());
    // Revoking again is not an error, and the first time stands.
    assert!(records
        .revoke(&alice, &phone.id, by_owner(T + 9))
        .await
        .unwrap());
    let phone_now = records
        .devices(&alice)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.id == phone.id)
        .unwrap();
    assert_eq!(phone_now.revoked, Some(by_owner(T + 5)));
}

async fn signed_in_devices_hold_a_live_refresh_credential(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    enroll(&*records, &alice, "phone", T).await;
    // Rotated: one spent credential and one live one, still one device.
    enroll(&*records, &alice, "laptop", T).await;
    records
        .rotate(
            &h("laptop-refresh"),
            issued("laptop-2", T + 9_000),
            T + 9_000,
        )
        .await
        .unwrap();
    let tablet = enroll(&*records, &alice, "tablet", T).await;
    let revoked = Revoked {
        at: T + 100,
        reason: RevokeReason::ByOwner,
    };
    assert!(records.revoke(&alice, &tablet.id, revoked).await.unwrap());
    enroll(&*records, &bob, "bob-phone", T).await;

    assert_eq!(
        records.signed_in_devices(&alice, T + 9_500).await.unwrap(),
        2
    );
    assert_eq!(records.signed_in_devices(&bob, T + 9_500).await.unwrap(), 1);
    // The phone's only credential lapses at T + 10 000; the laptop's renewed one later.
    assert_eq!(
        records.signed_in_devices(&alice, T + 10_000).await.unwrap(),
        1
    );
    assert_eq!(
        records.signed_in_devices(&alice, T + 19_000).await.unwrap(),
        0
    );
}

async fn devices_keep_every_wire_name(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    for (i, (kind, platform)) in [
        (DeviceKind::Phone, Platform::Android),
        (DeviceKind::Computer, Platform::Macos),
        (DeviceKind::Host, Platform::Linux),
        (DeviceKind::Other, Platform::Windows),
        (DeviceKind::Phone, Platform::Other),
    ]
    .into_iter()
    .enumerate()
    {
        let device = Device {
            id: device_id(),
            account: account.clone(),
            name: format!("Ana’s device {i} — ünïcode"),
            kind,
            platform,
            created_at: T + i as i64,
            last_seen_at: None,
            revoked: None,
        };
        records
            .add_device(device.clone(), issued(&format!("d{i}"), T))
            .await
            .unwrap();
        let (found, _) = records
            .access(&h(&format!("d{i}-access")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, device);
    }
}

// What only PostgreSQL has to do.

#[tokio::test]
async fn a_server_keeps_its_id_and_its_schema_across_restarts() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let first = db.records.server_id(T).await.unwrap();
    assert_eq!(db.records.server_id(T + 1).await.unwrap(), first);
    let account = db
        .records
        .account("https://id.one", "alice", T)
        .await
        .unwrap();

    // Connecting again finds the schema current and changes nothing.
    let again = PgRecords::connect_with(db.config.clone()).await.unwrap();
    assert_eq!(again.server_id(T + 2).await.unwrap(), first);
    assert_eq!(
        again.account("https://id.one", "alice", T).await.unwrap(),
        account
    );
    db.drop().await;
}

#[tokio::test]
async fn instances_starting_together_migrate_once() {
    let Some(empty) = common::pg::empty().await else {
        return;
    };
    let starts: Vec<_> = (0..4)
        .map(|_| {
            let config = empty.config.clone();
            tokio::spawn(async move { PgRecords::connect_with(config).await })
        })
        .collect();
    let mut ids = Vec::new();
    for start in starts {
        let records = start.await.unwrap().unwrap();
        ids.push(records.server_id(T).await.unwrap());
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "one server, one id: {ids:?}");
    empty.drop().await;
}

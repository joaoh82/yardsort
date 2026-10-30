//! What every [`Records`] store must do, checked against each: [`MemoryRecords`] always, and
//! [`PgRecords`] against a real PostgreSQL when `YARDSORT_TEST_DATABASE_URL` names one (see
//! `common/pg.rs`).

mod common;

use std::sync::Arc;

use std::collections::BTreeSet;

use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::MAX_OFFER_MISSES;
use remote_proto::{DeviceId, GrantId, HostId, Platform, ProjectGrant, ProjectId, Scope};
use yardsort_server::postgres::PgRecords;
use yardsort_server::records::{
    AccountId, Allowance, Decision, Device, GrantFilter, GrantRecord, GrantStatus, Handoff, Host,
    HostOffer, Issued, LinkPlace, MemoryRecords, PendingLogin, Records, Redemption, RevokeReason,
    Revoked, Rotation, Signal, SEEN_RESOLUTION_MS,
};
use yardsort_server::tokens::{self, TokenHash};
use yardsort_server::REFRESH_RETRY_MS;

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
    a_refresh_whose_answer_was_lost_can_be_presented_once_more,
    a_spent_refresh_credential_is_not_retried_late,
    a_lapsed_refresh_credential_is_expired,
    racing_rotations_of_one_credential_let_exactly_one_through,
    seen_is_written_at_most_once_a_minute,
    an_account_lists_and_revokes_only_its_own_devices,
    signed_in_devices_hold_a_live_refresh_credential,
    the_operator_revokes_every_device_of_one_account,
    devices_keep_every_wire_name,
    a_host_is_registered_moved_and_removed_within_its_account,
    an_offer_is_redeemed_once_within_its_account,
    misses_spend_only_their_accounts_open_offers,
    a_grant_is_answered_once_and_a_device_holds_one_per_host,
    revoking_a_grant_or_removing_its_host_ends_it,
    racing_approvals_leave_one_grant_in_force,
    grants_keep_every_scope_and_project,
    a_link_place_is_kept_by_its_instance_and_taken_by_a_later_link,
    pairing_requests_waiting_on_an_instances_links,
    signals_come_back_to_every_listener_in_order,
    a_listener_coming_after_the_last_one_left_still_hears,
    a_rate_limit_is_one_bucket_per_gate_and_key_and_refills_over_a_minute,
    racing_requests_take_no_more_than_a_rate_limit_holds,
    one_given_back_is_there_to_take_again_and_never_overfills,
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
    // The device has what it was given: it uses it.
    let (found, _) = records.access(&h("second-access")).await.unwrap().unwrap();
    assert_eq!(found.id, phone.id);

    // The first one again: only a copy could still hold it.
    let third = issued("third", T + 200);
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), third, T + 200)
            .await
            .unwrap(),
        Rotation::Reused(account.clone())
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
        Rotation::Reused(account.clone())
    );
}

/// A device whose answer was lost presents the spent credential again: it never had what was
/// issued in its place, so that is dropped and the device carries on. Once only.
async fn a_refresh_whose_answer_was_lost_can_be_presented_once_more(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let phone = enroll(&*records, &account, "phone", T).await;
    let rotated = records
        .rotate(&h("phone-refresh"), issued("lost", T + 100), T + 100)
        .await
        .unwrap();
    assert!(matches!(rotated, Rotation::Rotated(_)), "{rotated:?}");

    // The answer never arrived; the device tries again with what it has.
    match records
        .rotate(&h("phone-refresh"), issued("again", T + 200), T + 200)
        .await
        .unwrap()
    {
        Rotation::Retried(device) => {
            assert_eq!(device.id, phone.id);
            assert_eq!(device.revoked, None);
        }
        other => panic!("{other:?}"),
    }
    // What the lost answer carried works no more; what the retry got does.
    assert!(records.access(&h("lost-access")).await.unwrap().is_none());
    assert_eq!(
        records
            .rotate(&h("lost-refresh"), issued("x", T + 300), T + 300)
            .await
            .unwrap(),
        Rotation::Unknown
    );
    let (found, _) = records.access(&h("again-access")).await.unwrap().unwrap();
    assert_eq!(found.revoked, None);
    assert_eq!(
        records.signed_in_devices(&account, T + 300).await.unwrap(),
        1
    );

    // A third time is not a lost answer.
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), issued("third", T + 400), T + 400)
            .await
            .unwrap(),
        Rotation::Reused(account.clone())
    );
    let (found, _) = records.access(&h("again-access")).await.unwrap().unwrap();
    assert!(found.revoked.is_some());
}

/// After the window, or once the device has traded in what it was given, presenting a spent
/// credential again is a copy.
async fn a_spent_refresh_credential_is_not_retried_late(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let phone = enroll(&*records, &account, "phone", T).await;
    records
        .rotate(&h("phone-refresh"), issued("second", T + 100), T + 100)
        .await
        .unwrap();
    let late = T + 100 + REFRESH_RETRY_MS;
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), issued("late", late), late)
            .await
            .unwrap(),
        Rotation::Reused(account.clone())
    );
    let (found, _) = records.access(&h("second-access")).await.unwrap().unwrap();
    assert_eq!(found.id, phone.id);
    assert!(found.revoked.is_some());

    let tablet = enroll(&*records, &account, "tablet", T).await;
    records
        .rotate(&h("tablet-refresh"), issued("tablet-2", T + 100), T + 100)
        .await
        .unwrap();
    records
        .rotate(&h("tablet-2-refresh"), issued("tablet-3", T + 200), T + 200)
        .await
        .unwrap();
    assert_eq!(
        records
            .rotate(&h("tablet-refresh"), issued("copy", T + 300), T + 300)
            .await
            .unwrap(),
        Rotation::Reused(account.clone())
    );
    let (found, _) = records
        .access(&h("tablet-3-access"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, tablet.id);
    assert!(found.revoked.is_some());
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
    let (mut through, mut retried) = (0, 0);
    for racer in racers {
        match racer.await.unwrap() {
            Rotation::Rotated(_) => through += 1,
            // One more may look like a lost answer; a third cannot.
            Rotation::Retried(_) => retried += 1,
            Rotation::Reused(_) | Rotation::Revoked => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(through, 1);
    assert!(retried <= 1, "{retried} retries");
    let devices = records.devices(&account).await.unwrap();
    assert!(devices[0].revoked.is_some(), "eight at once are copies");
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

async fn the_operator_revokes_every_device_of_one_account(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    // The same subject at another provider is another person.
    let elsewhere = records.account("https://id.two", "alice", T).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;
    let laptop = enroll(&*records, &alice, "laptop", T).await;
    let earlier = enroll(&*records, &alice, "earlier", T).await;
    let by_owner = Revoked {
        at: T + 1,
        reason: RevokeReason::ByOwner,
    };
    assert!(records.revoke(&alice, &earlier.id, by_owner).await.unwrap());
    let theirs = enroll(&*records, &elsewhere, "theirs", T).await;

    let by_operator = |at| Revoked {
        at,
        reason: RevokeReason::ByOperator,
    };
    assert_eq!(
        records
            .revoke_account("https://id.one", "alice", by_operator(T + 5))
            .await
            .unwrap(),
        Some(2)
    );
    let revoked = |devices: Vec<Device>, id: &DeviceId| {
        devices.into_iter().find(|d| d.id == *id).unwrap().revoked
    };
    let now = records.devices(&alice).await.unwrap();
    assert_eq!(revoked(now.clone(), &phone.id), Some(by_operator(T + 5)));
    assert_eq!(revoked(now.clone(), &laptop.id), Some(by_operator(T + 5)));
    // An earlier revocation, and its reason, stand.
    assert_eq!(revoked(now, &earlier.id), Some(by_owner));
    assert_eq!(
        revoked(records.devices(&elsewhere).await.unwrap(), &theirs.id),
        None
    );
    assert_eq!(records.signed_in_devices(&alice, T + 6).await.unwrap(), 0);
    assert_eq!(
        records.signed_in_devices(&elsewhere, T + 6).await.unwrap(),
        1
    );

    // Again: nothing left to revoke, and the first time stands.
    assert_eq!(
        records
            .revoke_account("https://id.one", "alice", by_operator(T + 9))
            .await
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        revoked(records.devices(&alice).await.unwrap(), &phone.id),
        Some(by_operator(T + 5))
    );

    // A subject that never signed in is not an account, and asking does not make one.
    assert_eq!(
        records
            .revoke_account("https://id.one", "nobody", by_operator(T + 9))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        records
            .revoke_account("https://id.one", "nobody", by_operator(T + 9))
            .await
            .unwrap(),
        None
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

fn new_host(account: &AccountId, device: &Device, name: &str, now: i64) -> Host {
    Host {
        id: HostId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        account: account.clone(),
        device: device.id.clone(),
        name: name.into(),
        created_at: now,
        last_seen_at: None,
    }
}

async fn host_device(records: &impl Records, account: &AccountId, name: &str) -> Device {
    let device = Device {
        id: device_id(),
        account: account.clone(),
        name: name.into(),
        kind: DeviceKind::Host,
        platform: Platform::Linux,
        created_at: T,
        last_seen_at: None,
        revoked: None,
    };
    records
        .add_device(device.clone(), issued(name, T))
        .await
        .unwrap();
    device
}

fn offer(host: &Host, name: &str, expires_at: i64) -> HostOffer {
    HostOffer {
        host: host.id.clone(),
        account: host.account.clone(),
        secret: h(&format!("{name}-secret")),
        code: h(&format!("{name}-code")),
        expires_at,
    }
}

fn pending(host: &Host, device: &Device, now: i64) -> GrantRecord {
    GrantRecord {
        id: GrantId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        account: host.account.clone(),
        host: host.id.clone(),
        device: device.id.clone(),
        requested: [Scope::WorkspaceRead, Scope::TerminalRead].into(),
        scopes: BTreeSet::new(),
        projects: None,
        status: GrantStatus::Pending,
        created_at: now,
        expires_at: now + 300,
        decided_at: None,
    }
}

fn approve(scopes: &[Scope]) -> Decision {
    Decision::Approve {
        scopes: scopes.iter().copied().collect(),
        projects: ProjectGrant::All,
    }
}

async fn status(records: &impl Records, account: &AccountId, id: &GrantId) -> GrantStatus {
    records
        .grants(account, GrantFilter::One(id.clone()))
        .await
        .unwrap()
        .pop()
        .unwrap()
        .grant
        .status
}

async fn a_host_is_registered_moved_and_removed_within_its_account(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let first = host_device(&*records, &alice, "workstation").await;
    let host = new_host(&alice, &first, "Workstation", T);
    records.add_host(host.clone()).await.unwrap();
    assert_eq!(
        records.host(&alice, &host.id).await.unwrap(),
        Some(host.clone())
    );
    assert_eq!(records.hosts(&alice).await.unwrap(), vec![host.clone()]);
    assert_eq!(records.host(&bob, &host.id).await.unwrap(), None);
    assert!(records.hosts(&bob).await.unwrap().is_empty());

    // Its device's last use shows as the host's.
    records.seen(&first.id, T + 7).await.unwrap();
    let seen = records.host(&alice, &host.id).await.unwrap().unwrap();
    assert_eq!(seen.last_seen_at, Some(T + 7));

    // Signed in again: the same host, on its new device, under its new name.
    let second = host_device(&*records, &alice, "workstation-again").await;
    let moved = records
        .rebind_host(&alice, &host.id, &second.id, "Desk")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (moved.device.clone(), moved.name.as_str()),
        (second.id.clone(), "Desk")
    );
    assert_eq!(moved.last_seen_at, None);
    assert_eq!(moved.created_at, T);
    // Another account cannot take it over, and trying changes nothing.
    let bobs = host_device(&*records, &bob, "bobs").await;
    assert_eq!(
        records
            .rebind_host(&bob, &host.id, &bobs.id, "Mine")
            .await
            .unwrap(),
        None
    );
    assert_eq!(records.host(&alice, &host.id).await.unwrap(), Some(moved));

    assert!(!records.remove_host(&bob, &host.id, T + 9).await.unwrap());
    assert!(records.remove_host(&alice, &host.id, T + 9).await.unwrap());
    assert!(!records.remove_host(&alice, &host.id, T + 10).await.unwrap());
    assert_eq!(records.host(&alice, &host.id).await.unwrap(), None);
    assert!(records.hosts(&alice).await.unwrap().is_empty());
    // A removed host stays removed: signing in again does not bring it back.
    assert_eq!(
        records
            .rebind_host(&alice, &host.id, &second.id, "Back")
            .await
            .unwrap(),
        None
    );
}

async fn an_offer_is_redeemed_once_within_its_account(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let device = host_device(&*records, &alice, "workstation").await;
    let host = new_host(&alice, &device, "Workstation", T);
    records.add_host(host.clone()).await.unwrap();

    records
        .put_offer(offer(&host, "o1", T + 300), T)
        .await
        .unwrap();
    // Another account holding the secret gets nothing, and spends nothing.
    assert_eq!(
        records
            .redeem_offer(&bob, &h("o1-secret"), T + 1)
            .await
            .unwrap(),
        Redemption::Missed
    );
    assert_eq!(
        records
            .redeem_offer(&alice, &h("o1-code"), T + 1)
            .await
            .unwrap(),
        Redemption::Taken(offer(&host, "o1", T + 300))
    );
    assert_eq!(
        records
            .redeem_offer(&alice, &h("o1-secret"), T + 2)
            .await
            .unwrap(),
        Redemption::Missed
    );

    // A new offer replaces the host's open one.
    records
        .put_offer(offer(&host, "o2", T + 300), T)
        .await
        .unwrap();
    records
        .put_offer(offer(&host, "o3", T + 300), T)
        .await
        .unwrap();
    assert_eq!(
        records
            .redeem_offer(&alice, &h("o2-secret"), T + 1)
            .await
            .unwrap(),
        Redemption::Missed
    );
    assert!(matches!(
        records
            .redeem_offer(&alice, &h("o3-secret"), T + 1)
            .await
            .unwrap(),
        Redemption::Taken(_)
    ));

    // A lapsed one is not redeemed.
    records
        .put_offer(offer(&host, "o4", T + 300), T)
        .await
        .unwrap();
    assert_eq!(
        records
            .redeem_offer(&alice, &h("o4-code"), T + 300)
            .await
            .unwrap(),
        Redemption::Missed
    );

    // Removing the host takes its open offer with it.
    records
        .put_offer(offer(&host, "o5", T + 300), T)
        .await
        .unwrap();
    assert!(records.remove_host(&alice, &host.id, T + 1).await.unwrap());
    assert_eq!(
        records
            .redeem_offer(&alice, &h("o5-code"), T + 2)
            .await
            .unwrap(),
        Redemption::Missed
    );
}

async fn misses_spend_only_their_accounts_open_offers(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let alices = new_host(&alice, &host_device(&*records, &alice, "a").await, "A", T);
    let bobs = new_host(&bob, &host_device(&*records, &bob, "b").await, "B", T);
    records.add_host(alices.clone()).await.unwrap();
    records.add_host(bobs.clone()).await.unwrap();
    records
        .put_offer(offer(&alices, "a1", T + 300), T)
        .await
        .unwrap();
    records
        .put_offer(offer(&bobs, "b1", T + 300), T)
        .await
        .unwrap();
    for i in 0..MAX_OFFER_MISSES - 1 {
        let wrong = h(&format!("guess-{i}"));
        assert_eq!(
            records.redeem_offer(&alice, &wrong, T + 1).await.unwrap(),
            Redemption::Missed
        );
    }
    // One short of the limit, the right code still works.
    assert!(matches!(
        records
            .redeem_offer(&alice, &h("a1-code"), T + 2)
            .await
            .unwrap(),
        Redemption::Taken(_)
    ));
    records
        .put_offer(offer(&alices, "a2", T + 300), T)
        .await
        .unwrap();
    for i in 0..MAX_OFFER_MISSES {
        let wrong = h(&format!("guess-{i}"));
        records.redeem_offer(&alice, &wrong, T + 3).await.unwrap();
    }
    // At the limit, it no longer does: the host has to show a new one.
    assert_eq!(
        records
            .redeem_offer(&alice, &h("a2-code"), T + 4)
            .await
            .unwrap(),
        Redemption::Missed
    );
    records
        .put_offer(offer(&alices, "a3", T + 300), T + 5)
        .await
        .unwrap();
    assert!(matches!(
        records
            .redeem_offer(&alice, &h("a3-code"), T + 6)
            .await
            .unwrap(),
        Redemption::Taken(_)
    ));
    // Alice's misses were not counted against Bob's offer.
    assert!(matches!(
        records
            .redeem_offer(&bob, &h("b1-code"), T + 6)
            .await
            .unwrap(),
        Redemption::Taken(_)
    ));
}

async fn a_grant_is_answered_once_and_a_device_holds_one_per_host(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let host = new_host(
        &alice,
        &host_device(&*records, &alice, "ws").await,
        "Workstation",
        T,
    );
    records.add_host(host.clone()).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;
    let laptop = enroll(&*records, &alice, "laptop", T).await;

    let first = pending(&host, &phone, T);
    records.add_grant(first.clone()).await.unwrap();
    // Another account cannot answer it.
    assert_eq!(
        records
            .decide(&bob, &first.id, approve(&[Scope::WorkspaceRead]), T + 1)
            .await
            .unwrap(),
        None
    );
    let approved = records
        .decide(&alice, &first.id, approve(&[Scope::WorkspaceRead]), T + 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, GrantStatus::Approved);
    assert_eq!(approved.scopes, [Scope::WorkspaceRead].into());
    assert_eq!(approved.projects, Some(ProjectGrant::All));
    assert_eq!(approved.decided_at, Some(T + 1));
    assert_eq!(approved.requested, first.requested);
    // Answered once.
    assert_eq!(
        records
            .decide(&alice, &first.id, Decision::Deny, T + 2)
            .await
            .unwrap(),
        None
    );

    // A second approval for the same device on the same host replaces the first.
    let second = pending(&host, &phone, T + 3);
    let laptops = pending(&host, &laptop, T + 3);
    records.add_grant(second.clone()).await.unwrap();
    records.add_grant(laptops.clone()).await.unwrap();
    records
        .decide(&alice, &laptops.id, approve(&[Scope::TerminalRead]), T + 4)
        .await
        .unwrap()
        .unwrap();
    records
        .decide(&alice, &second.id, approve(&[Scope::TerminalRead]), T + 4)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status(&*records, &alice, &first.id).await,
        GrantStatus::Revoked
    );
    assert_eq!(
        status(&*records, &alice, &second.id).await,
        GrantStatus::Approved
    );
    // Another device's grant on it stands.
    assert_eq!(
        status(&*records, &alice, &laptops.id).await,
        GrantStatus::Approved
    );

    // A denial is an answer too, and it revokes nothing.
    let third = pending(&host, &phone, T + 5);
    records.add_grant(third.clone()).await.unwrap();
    let denied = records
        .decide(&alice, &third.id, Decision::Deny, T + 6)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (denied.status, denied.projects),
        (GrantStatus::Denied, None)
    );
    assert_eq!(
        status(&*records, &alice, &second.id).await,
        GrantStatus::Approved
    );

    // Nobody answered in time: too late to.
    let late = pending(&host, &phone, T + 10);
    records.add_grant(late.clone()).await.unwrap();
    assert_eq!(
        records
            .decide(
                &alice,
                &late.id,
                approve(&[Scope::WorkspaceRead]),
                late.expires_at
            )
            .await
            .unwrap(),
        None
    );

    // Listed newest first, by host, by device and one by one, with names; never another
    // account's.
    let of_host = records
        .grants(&alice, GrantFilter::OfHost(host.id.clone()))
        .await
        .unwrap();
    assert_eq!(of_host.len(), 5);
    assert!(of_host
        .windows(2)
        .all(|w| w[0].grant.created_at >= w[1].grant.created_at));
    assert!(of_host.iter().all(|v| v.host_name == "Workstation"));
    let for_laptop = records
        .grants(&alice, GrantFilter::ForDevice(laptop.id.clone()))
        .await
        .unwrap();
    assert_eq!(for_laptop.len(), 1);
    assert_eq!(for_laptop[0].device_name, "laptop");
    assert!(!for_laptop[0].device_revoked);
    assert!(records
        .grants(&bob, GrantFilter::OfHost(host.id.clone()))
        .await
        .unwrap()
        .is_empty());
    assert!(records
        .grants(&bob, GrantFilter::One(first.id.clone()))
        .await
        .unwrap()
        .is_empty());

    // Signing a device out shows on its grants.
    let by_owner = Revoked {
        at: T + 20,
        reason: RevokeReason::ByOwner,
    };
    assert!(records.revoke(&alice, &laptop.id, by_owner).await.unwrap());
    let for_laptop = records
        .grants(&alice, GrantFilter::ForDevice(laptop.id.clone()))
        .await
        .unwrap();
    assert!(for_laptop[0].device_revoked);
}

async fn revoking_a_grant_or_removing_its_host_ends_it(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let bob = records.account("https://id.one", "bob", T).await.unwrap();
    let host = new_host(
        &alice,
        &host_device(&*records, &alice, "ws").await,
        "Workstation",
        T,
    );
    records.add_host(host.clone()).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;

    let given = pending(&host, &phone, T);
    records.add_grant(given.clone()).await.unwrap();
    records
        .decide(&alice, &given.id, approve(&[Scope::WorkspaceRead]), T + 1)
        .await
        .unwrap();
    assert!(!records.revoke_grant(&bob, &given.id, T + 2).await.unwrap());
    assert_eq!(
        status(&*records, &alice, &given.id).await,
        GrantStatus::Approved
    );
    assert!(records
        .revoke_grant(&alice, &given.id, T + 2)
        .await
        .unwrap());
    let revoked = records
        .grants(&alice, GrantFilter::One(given.id.clone()))
        .await
        .unwrap()
        .pop()
        .unwrap()
        .grant;
    assert_eq!(revoked.status, GrantStatus::Revoked);
    assert_eq!(revoked.decided_at, Some(T + 2));
    // It keeps what it gave.
    assert_eq!(revoked.scopes, [Scope::WorkspaceRead].into());
    // Revoked once; the first time stands.
    assert!(records
        .revoke_grant(&alice, &given.id, T + 3)
        .await
        .unwrap());
    let again = records
        .grants(&alice, GrantFilter::One(given.id.clone()))
        .await
        .unwrap()
        .pop()
        .unwrap()
        .grant;
    assert_eq!(again.decided_at, Some(T + 2));

    // A denied grant stays denied.
    let refused = pending(&host, &phone, T + 4);
    records.add_grant(refused.clone()).await.unwrap();
    records
        .decide(&alice, &refused.id, Decision::Deny, T + 5)
        .await
        .unwrap();
    assert!(records
        .revoke_grant(&alice, &refused.id, T + 6)
        .await
        .unwrap());
    assert_eq!(
        status(&*records, &alice, &refused.id).await,
        GrantStatus::Denied
    );

    // Removing the host revokes what is pending and what is in force.
    let waiting = pending(&host, &phone, T + 7);
    let in_force = pending(&host, &phone, T + 7);
    records.add_grant(waiting.clone()).await.unwrap();
    records.add_grant(in_force.clone()).await.unwrap();
    records
        .decide(&alice, &in_force.id, approve(&[Scope::TerminalRead]), T + 8)
        .await
        .unwrap();
    assert!(records.remove_host(&alice, &host.id, T + 9).await.unwrap());
    for id in [&waiting.id, &in_force.id] {
        assert_eq!(status(&*records, &alice, id).await, GrantStatus::Revoked);
    }
    assert_eq!(
        status(&*records, &alice, &refused.id).await,
        GrantStatus::Denied
    );
}

async fn racing_approvals_leave_one_grant_in_force(records: Arc<impl Records + 'static>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let host = new_host(
        &alice,
        &host_device(&*records, &alice, "ws").await,
        "Workstation",
        T,
    );
    records.add_host(host.clone()).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;
    let grants: Vec<GrantRecord> = (0..8).map(|_| pending(&host, &phone, T)).collect();
    for grant in &grants {
        records.add_grant(grant.clone()).await.unwrap();
    }
    let racers: Vec<_> = grants
        .iter()
        .map(|grant| {
            let (records, account, id) = (records.clone(), alice.clone(), grant.id.clone());
            tokio::spawn(async move {
                records
                    .decide(&account, &id, approve(&[Scope::WorkspaceRead]), T + 1)
                    .await
                    .unwrap()
            })
        })
        .collect();
    for racer in racers {
        assert!(racer.await.unwrap().is_some());
    }
    let in_force = records
        .grants(&alice, GrantFilter::ForDevice(phone.id.clone()))
        .await
        .unwrap()
        .into_iter()
        .filter(|v| v.grant.status == GrantStatus::Approved)
        .count();
    assert_eq!(in_force, 1);
}

async fn grants_keep_every_scope_and_project(records: Arc<impl Records>) {
    let alice = records.account("https://id.one", "alice", T).await.unwrap();
    let host = new_host(
        &alice,
        &host_device(&*records, &alice, "ws").await,
        "Ana’s desk — ü",
        T,
    );
    records.add_host(host.clone()).await.unwrap();
    let phone = enroll(&*records, &alice, "phone", T).await;
    let every: BTreeSet<Scope> = [
        Scope::WorkspaceRead,
        Scope::ActivityRead,
        Scope::MemoryRead,
        Scope::TerminalRead,
        Scope::TerminalControl,
        Scope::DeviceManage,
    ]
    .into();
    let projects = ProjectGrant::Only(
        ["p-1", "p-2"]
            .into_iter()
            .map(|p| ProjectId::new(p).unwrap())
            .collect(),
    );
    let grant = GrantRecord {
        requested: every.clone(),
        ..pending(&host, &phone, T)
    };
    records.add_grant(grant.clone()).await.unwrap();
    let listed = records
        .grants(&alice, GrantFilter::One(grant.id.clone()))
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(listed.grant, grant);
    assert_eq!(listed.host_name, "Ana’s desk — ü");
    let decided = records
        .decide(
            &alice,
            &grant.id,
            Decision::Approve {
                scopes: every.clone(),
                projects: projects.clone(),
            },
            T + 1,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decided.scopes, every);
    assert_eq!(decided.projects, Some(projects));
}

async fn a_link_place_is_kept_by_its_instance_and_taken_by_a_later_link(
    records: Arc<impl Records>,
) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let mut hosts = Vec::new();
    for name in ["Workstation", "Laptop"] {
        let device = host_device(&*records, &account, name).await;
        let host = new_host(&account, &device, name, T);
        records.add_host(host.clone()).await.unwrap();
        hosts.push(host.id);
    }
    let place = |host: &HostId, instance: &str, link: &str, seen_at| LinkPlace {
        host: host.clone(),
        instance: instance.into(),
        link: link.into(),
        seen_at,
    };
    assert_eq!(records.link_place(&hosts[0]).await.unwrap(), None);
    records
        .put_link(place(&hosts[0], "a", "link-1", T))
        .await
        .unwrap();
    records
        .put_link(place(&hosts[1], "b", "link-2", T))
        .await
        .unwrap();

    // Each instance keeps only its own.
    let kept = records.keep_links("a", T + 5_000).await.unwrap();
    assert_eq!(kept, [place(&hosts[0], "a", "link-1", T + 5_000)]);
    assert_eq!(
        records.link_place(&hosts[1]).await.unwrap(),
        Some(place(&hosts[1], "b", "link-2", T)),
        "another instance's is not touched"
    );

    // The host connects again on b: a's link is no longer among a's.
    records
        .put_link(place(&hosts[0], "b", "link-3", T + 6_000))
        .await
        .unwrap();
    assert!(records
        .keep_links("a", T + 10_000)
        .await
        .unwrap()
        .is_empty());
    let mut on_b = records.keep_links("b", T + 10_000).await.unwrap();
    on_b.sort_by(|x, y| x.link.cmp(&y.link));
    assert_eq!(
        on_b,
        [
            place(&hosts[1], "b", "link-2", T + 10_000),
            place(&hosts[0], "b", "link-3", T + 10_000),
        ]
    );

    // The old link closing late forgets nothing of the new one's; the new one closing does.
    records.drop_link(&hosts[0], "link-1").await.unwrap();
    assert_eq!(
        records.link_place(&hosts[0]).await.unwrap(),
        Some(place(&hosts[0], "b", "link-3", T + 10_000))
    );
    records.drop_link(&hosts[0], "link-3").await.unwrap();
    assert_eq!(records.link_place(&hosts[0]).await.unwrap(), None);
    assert!(records.link_place(&hosts[1]).await.unwrap().is_some());
}

async fn pairing_requests_waiting_on_an_instances_links(records: Arc<impl Records>) {
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    let mut hosts = Vec::new();
    for name in ["Workstation", "Laptop"] {
        let device = host_device(&*records, &account, name).await;
        let host = new_host(&account, &device, name, T);
        records.add_host(host.clone()).await.unwrap();
        hosts.push((host, device));
    }
    let phone = enroll(&*records, &account, "Phone", T).await;
    let mut asking = Vec::new();
    for (host, _) in &hosts {
        let grant = pending(host, &phone, T);
        records.add_grant(grant.clone()).await.unwrap();
        asking.push(grant.id);
    }
    // One already answered is not waiting.
    let tablet = enroll(&*records, &account, "Tablet", T).await;
    let answered = pending(&hosts[0].0, &tablet, T);
    records.add_grant(answered.clone()).await.unwrap();
    records
        .decide(&account, &answered.id, Decision::Deny, T + 1)
        .await
        .unwrap();

    // Only the hosts whose links are on a.
    assert!(records.pending_on_links("a", T).await.unwrap().is_empty());
    for (host, instance) in [(&hosts[0].0, "a"), (&hosts[1].0, "b")] {
        records
            .put_link(LinkPlace {
                host: host.id.clone(),
                instance: instance.into(),
                link: format!("link-{instance}"),
                seen_at: T,
            })
            .await
            .unwrap();
    }
    assert_eq!(
        records.pending_on_links("a", T + 2).await.unwrap(),
        [(hosts[0].0.id.clone(), asking[0].clone())]
    );
    // Lapsed: no longer waiting.
    let lapsed = pending(&hosts[0].0, &phone, T).expires_at;
    assert!(records
        .pending_on_links("a", lapsed)
        .await
        .unwrap()
        .is_empty());
}

async fn signals_come_back_to_every_listener_in_order(records: Arc<impl Records>) {
    let mut first = records.signals();
    let mut second = records.signals();
    for listening in [&mut first, &mut second] {
        // Each says it is listening, so that nothing sent from here on is missed.
        loop {
            let heard = tokio::time::timeout(std::time::Duration::from_secs(10), listening.recv())
                .await
                .expect("never listening")
                .unwrap();
            if heard == Signal::RecheckAll {
                break;
            }
        }
    }
    let account = AccountId("account-1".into());
    let pairing = Signal::Pairing(HostId::new("host-1").unwrap(), GrantId::new("g-1").unwrap());
    records
        .signal(Signal::Recheck(account.clone()))
        .await
        .unwrap();
    records.signal(pairing.clone()).await.unwrap();
    for listening in [&mut first, &mut second] {
        let mut heard = Vec::new();
        while heard.len() < 2 {
            let signal = tokio::time::timeout(std::time::Duration::from_secs(10), listening.recv())
                .await
                .expect("a signal never came")
                .unwrap();
            // Another listener starting says so to everyone; that is no harm.
            if signal != Signal::RecheckAll {
                heard.push(signal);
            }
        }
        assert_eq!(heard, [Signal::Recheck(account.clone()), pairing.clone()]);
    }
}

async fn a_listener_coming_after_the_last_one_left_still_hears(records: Arc<impl Records>) {
    let mut first = records.signals();
    let started = tokio::time::timeout(std::time::Duration::from_secs(10), first.recv()).await;
    assert_eq!(
        started.expect("never listening").unwrap(),
        Signal::RecheckAll
    );
    drop(first);
    // Heard with nobody listening, which is when listening stops; and long enough for it to.
    records.signal(Signal::RecheckAll).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let mut second = records.signals();
    let account = AccountId("account-1".into());
    // Sent again until heard, since listening may be starting over.
    let heard = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            records
                .signal(Signal::Recheck(account.clone()))
                .await
                .unwrap();
            let next = std::time::Duration::from_millis(200);
            if let Ok(Ok(Signal::Recheck(heard))) = tokio::time::timeout(next, second.recv()).await
            {
                return heard;
            }
        }
    })
    .await;
    assert_eq!(heard.expect("a later listener heard nothing"), account);
}

async fn a_rate_limit_is_one_bucket_per_gate_and_key_and_refills_over_a_minute(
    records: Arc<impl Records>,
) {
    let (one, two) = (h("address-1"), h("address-2"));
    for _ in 0..3 {
        assert_eq!(
            records.take_allowance("logins", &one, 3, T).await.unwrap(),
            Allowance::Taken
        );
    }
    // Empty: a third of a minute until the next, at three a minute.
    let Allowance::Wait(wait) = records.take_allowance("logins", &one, 3, T).await.unwrap() else {
        panic!("a fourth was let through");
    };
    assert!((wait.as_secs_f64() - 20.0).abs() < 0.01, "{wait:?}");
    // Another key, and another gate, have buckets of their own.
    assert_eq!(
        records.take_allowance("logins", &two, 3, T).await.unwrap(),
        Allowance::Taken
    );
    assert_eq!(
        records.take_allowance("tokens", &one, 3, T).await.unwrap(),
        Allowance::Taken
    );
    // Asking while empty takes nothing, and a third of a minute brings one back.
    let Allowance::Wait(wait) = records
        .take_allowance("logins", &one, 3, T + 10_000)
        .await
        .unwrap()
    else {
        panic!("let through before the refill");
    };
    assert!((wait.as_secs_f64() - 10.0).abs() < 0.01, "{wait:?}");
    assert_eq!(
        records
            .take_allowance("logins", &one, 3, T + 20_000)
            .await
            .unwrap(),
        Allowance::Taken
    );
    // A clock behind the bucket's refills nothing.
    assert!(matches!(
        records.take_allowance("logins", &one, 3, T).await.unwrap(),
        Allowance::Wait(_)
    ));

    // A sweep forgets buckets untouched since `before`, and only those.
    records.sweep_allowances(T + 20_000).await.unwrap();
    assert!(matches!(
        records
            .take_allowance("logins", &one, 3, T + 20_000)
            .await
            .unwrap(),
        Allowance::Wait(_)
    ));
    records.sweep_allowances(T + 20_001).await.unwrap();
    assert_eq!(
        records
            .take_allowance("logins", &one, 3, T + 20_000)
            .await
            .unwrap(),
        Allowance::Taken
    );
}

async fn racing_requests_take_no_more_than_a_rate_limit_holds(
    records: Arc<impl Records + 'static>,
) {
    let racers: Vec<_> = (0..12)
        .map(|_| {
            let records = records.clone();
            tokio::spawn(async move {
                records
                    .take_allowance("pairings", &h("address-1"), 5, T)
                    .await
                    .unwrap()
            })
        })
        .collect();
    let mut taken = 0;
    for racer in racers {
        if racer.await.unwrap() == Allowance::Taken {
            taken += 1;
        }
    }
    assert_eq!(taken, 5);
}

async fn one_given_back_is_there_to_take_again_and_never_overfills(records: Arc<impl Records>) {
    let key = h("address-1");
    // Nothing taken yet: nothing to give back, and nothing made.
    records.return_allowance("tickets", &key, 2).await.unwrap();
    for _ in 0..2 {
        assert_eq!(
            records.take_allowance("tickets", &key, 2, T).await.unwrap(),
            Allowance::Taken
        );
    }
    records.return_allowance("tickets", &key, 2).await.unwrap();
    assert_eq!(
        records.take_allowance("tickets", &key, 2, T).await.unwrap(),
        Allowance::Taken
    );
    assert!(matches!(
        records.take_allowance("tickets", &key, 2, T).await.unwrap(),
        Allowance::Wait(_)
    ));
    // Given back more often than taken, it holds no more than its limit.
    for _ in 0..5 {
        records.return_allowance("tickets", &key, 2).await.unwrap();
    }
    for _ in 0..2 {
        assert_eq!(
            records.take_allowance("tickets", &key, 2, T).await.unwrap(),
            Allowance::Taken
        );
    }
    assert!(matches!(
        records.take_allowance("tickets", &key, 2, T).await.unwrap(),
        Allowance::Wait(_)
    ));
}

#[test]
fn signals_travel_as_a_few_words_and_nothing_else_reads_as_one() {
    let host = HostId::new("host-1").unwrap();
    let grant = GrantId::new("grant:1").unwrap();
    for signal in [
        Signal::Recheck(AccountId("9b2f6c1e-account".into())),
        Signal::RecheckAll,
        Signal::Pairing(host.clone(), grant),
        Signal::Linked(host),
    ] {
        assert_eq!(Signal::decode(&signal.encode()), Some(signal));
    }
    for other in [
        "",
        "recheck",
        "recheck ",
        "pairing h",
        "pairing h g x",
        "revoke a",
        "host-renamed x",
        "linked",
        "linked h x",
        // A listening connection's probe of itself.
        "probe 0b5c3f9e-8f7a-4f1e-9d55-2f3c8f4c1a7e",
    ] {
        assert_eq!(Signal::decode(other), None, "{other:?}");
    }
}

// What only PostgreSQL has to do.

/// A connection of the test's own to the test's schema, to hold a lock the way another request
/// would.
async fn raw(db: &common::pg::TestDatabase) -> tokio_postgres::Client {
    let (client, connection) = db.config.connect(tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    client
}

/// A retry has locked the access credential issued in its place and dropped it. A request
/// presenting that credential for the first time waits, and is then refused: it may not count as
/// a use the retry did not see.
#[tokio::test]
async fn a_first_use_waits_for_a_retry_and_loses_to_it() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let records = db.records.clone();
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    records
        .rotate(&h("phone-refresh"), issued("second", T + 100), T + 100)
        .await
        .unwrap();

    let retry = raw(&db).await;
    let second = h("second-access").0.to_vec();
    retry.batch_execute("BEGIN").await.unwrap();
    retry
        .execute(
            "SELECT used FROM access_credentials WHERE hash = $1 FOR UPDATE",
            &[&second],
        )
        .await
        .unwrap();
    retry
        .execute("DELETE FROM access_credentials WHERE hash = $1", &[&second])
        .await
        .unwrap();
    let request = tokio::spawn({
        let records = records.clone();
        async move { records.access(&h("second-access")).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!request.is_finished(), "the first use waits for the retry");
    retry.batch_execute("COMMIT").await.unwrap();
    assert!(request.await.unwrap().unwrap().is_none());
    db.drop().await;
}

/// What was issued in place of a spent refresh credential is gone, which nothing does within the
/// window today: presenting it again is then reuse, as in the memory store.
#[tokio::test]
async fn a_spent_refresh_credential_whose_successor_is_gone_is_a_copy() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let records = db.records.clone();
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    records
        .rotate(&h("phone-refresh"), issued("second", T + 100), T + 100)
        .await
        .unwrap();
    raw(&db)
        .await
        .execute(
            "DELETE FROM refresh_credentials WHERE hash = $1",
            &[&h("second-refresh").0.to_vec()],
        )
        .await
        .unwrap();
    assert_eq!(
        records
            .rotate(&h("phone-refresh"), issued("again", T + 200), T + 200)
            .await
            .unwrap(),
        Rotation::Reused(account)
    );
    db.drop().await;
}

/// A request is using the access credential issued in place of a spent refresh credential. That
/// refresh credential presented again waits for the request, and is then a copy.
#[tokio::test]
async fn a_retry_waits_for_a_first_use_and_loses_to_it() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let records = db.records.clone();
    let account = records.account("https://id.one", "alice", T).await.unwrap();
    enroll(&*records, &account, "phone", T).await;
    records
        .rotate(&h("phone-refresh"), issued("second", T + 100), T + 100)
        .await
        .unwrap();

    let request = raw(&db).await;
    request.batch_execute("BEGIN").await.unwrap();
    request
        .execute(
            "UPDATE access_credentials SET used = TRUE WHERE hash = $1",
            &[&h("second-access").0.to_vec()],
        )
        .await
        .unwrap();
    let copy = tokio::spawn({
        let records = records.clone();
        async move {
            records
                .rotate(&h("phone-refresh"), issued("copy", T + 200), T + 200)
                .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!copy.is_finished(), "the retry waits for the first use");
    request.batch_execute("COMMIT").await.unwrap();
    assert_eq!(copy.await.unwrap().unwrap(), Rotation::Reused(account));
    db.drop().await;
}

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

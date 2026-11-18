//! `yardsort-server revoke-account`, run as the operator runs it: the built binary, against
//! PostgreSQL (see `common/pg.rs`).

mod common;

use std::process::{Command, Output};
use std::time::Duration;

use remote_proto::account::DeviceKind;
use remote_proto::{DeviceId, Platform};
use yardsort_server::records::{AccountId, Device, Issued, Records, RevokeReason, Signal};
use yardsort_server::tokens;

const ISSUER: &str = "https://id.example.invalid";
const T: i64 = 1_800_000_000_000;

/// The binary with only the environment given here, of the server's own variables.
fn server(env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yardsort-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("YARDSORT_") || name.starts_with("DATABASE_URL") {
            command.env_remove(name);
        }
    }
    command.envs(env.iter().copied()).args(args);
    command.output().expect("running yardsort-server")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

async fn enroll(records: &impl Records, account: &AccountId, name: &str) -> DeviceId {
    let id = DeviceId::new(uuid::Uuid::new_v4().to_string()).unwrap();
    let device = Device {
        id: id.clone(),
        account: account.clone(),
        name: name.into(),
        kind: DeviceKind::Computer,
        platform: Platform::Linux,
        created_at: T,
        last_seen_at: None,
        revoked: None,
    };
    let issued = Issued {
        access: tokens::hash(&format!("{name}-access")),
        access_expires_at: i64::MAX,
        refresh: tokens::hash(&format!("{name}-refresh")),
        refresh_expires_at: i64::MAX,
    };
    records.add_device(device, issued).await.unwrap();
    id
}

#[tokio::test]
async fn revoke_account_signs_out_every_device_of_that_account_only() {
    let Some(db) = common::pg::database().await else {
        return;
    };
    let url = db.url();
    let alice = db.records.account(ISSUER, "alice", T).await.unwrap();
    let bob = db.records.account(ISSUER, "bob", T).await.unwrap();
    let laptop = enroll(&*db.records, &alice, "laptop").await;
    let phone = enroll(&*db.records, &alice, "phone").await;
    let bobs = enroll(&*db.records, &bob, "bobs").await;

    // As a running server listens.
    let mut signals = db.records.signals();
    let listening = tokio::time::timeout(Duration::from_secs(10), signals.recv()).await;
    assert_eq!(
        listening.expect("never listening").unwrap(),
        Signal::RecheckAll
    );

    let env = [
        ("DATABASE_URL", url.as_str()),
        ("YARDSORT_OIDC_ISSUER", ISSUER),
    ];
    let output = server(&env, &["revoke-account", "--subject", "alice"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("Revoked 2 devices of the account for alice"),
        "{}",
        text(&output)
    );
    // Running servers are told, and check every live socket now.
    assert!(
        text(&output).contains("Running servers close their live connections now."),
        "{}",
        text(&output)
    );
    let told = tokio::time::timeout(Duration::from_secs(10), signals.recv()).await;
    assert_eq!(told.expect("no signal").unwrap(), Signal::RecheckAll);

    let devices = db.records.devices(&alice).await.unwrap();
    for id in [&laptop, &phone] {
        let device = devices.iter().find(|d| d.id == *id).unwrap();
        assert_eq!(
            device.revoked.map(|r| r.reason),
            Some(RevokeReason::ByOperator)
        );
    }
    assert!(db
        .records
        .access(&tokens::hash("laptop-access"))
        .await
        .unwrap()
        .is_some_and(|(device, _)| device.revoked.is_some()));
    let bobs_now = db.records.devices(&bob).await.unwrap();
    assert_eq!(
        bobs_now.iter().find(|d| d.id == bobs).unwrap().revoked,
        None
    );

    // Another provider's "alice" is someone else, and a typo is an error, not a quiet zero.
    let elsewhere = server(
        &[("DATABASE_URL", url.as_str())],
        &[
            "revoke-account",
            "--issuer",
            "https://other.example.invalid",
            "--subject",
            "bob",
        ],
    );
    assert!(!elsewhere.status.success());
    assert!(
        text(&elsewhere).contains("no account for bob"),
        "{}",
        text(&elsewhere)
    );
    assert_eq!(db.records.devices(&bob).await.unwrap()[0].revoked, None);
    let typo = server(&env, &["revoke-account", "--subject", "alcie"]);
    assert!(!typo.status.success());
    db.drop().await;
}

/// The environment, the arguments, and what the error has to say.
type Case<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str], &'a str);

#[test]
fn revoke_account_says_what_it_is_missing() {
    let cases: [Case; 4] = [
        (&[], &["revoke-account"], "--subject is required"),
        (
            &[],
            &["revoke-account", "--subject"],
            "--subject needs a value",
        ),
        (
            &[],
            &["revoke-account", "--subject", "alice"],
            "DATABASE_URL is not set",
        ),
        (
            &[("DATABASE_URL", "postgres://u@127.0.0.1:1/x")],
            &["revoke-account", "--subject", "alice"],
            "give --issuer, or set YARDSORT_OIDC_ISSUER",
        ),
    ];
    for (env, args, expected) in cases {
        let output = server(env, args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            text(&output).contains(expected),
            "{args:?}: {}",
            text(&output)
        );
    }
    let unknown = server(&[], &["revoke-acount"]);
    assert!(!unknown.status.success());
    assert!(
        text(&unknown).contains("unknown command"),
        "{}",
        text(&unknown)
    );
}

#[tokio::test]
async fn usage_prints_each_accounts_days_between_two_dates() {
    use yardsort_server::usage::{day_of, parse_date, UsageDelta};
    let Some(db) = common::pg::database().await else {
        return;
    };
    let url = db.url();
    let alice = db.records.account(ISSUER, "alice", T).await.unwrap();
    let phone = enroll(&*db.records, &alice, "usage-phone").await;
    let day = parse_date("2027-01-14").unwrap();
    let now = day * 24 * 60 * 60_000;
    assert_eq!(day_of(now), day);
    let delta = |day, relay_bytes| UsageDelta {
        account: alice.clone(),
        day,
        relay_bytes,
        peak_streams: 2,
        sync_bytes: 30,
        devices: [phone.clone()].into(),
    };
    db.records
        .add_usage(&[delta(day - 1, 1000), delta(day, 500)], now)
        .await
        .unwrap();

    let env = [("DATABASE_URL", url.as_str())];
    let out = server(
        &env,
        &["usage", "--from", "2027-01-14", "--to", "2027-01-14"],
    );
    assert!(out.status.success(), "{}", text(&out));
    let printed = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(
        lines,
        [
            "day\taccount\tactive devices\tpeak connections\trelay bytes\tsync bytes",
            &*format!("2027-01-14\t{}\t1\t2\t500\t30", alice.0),
        ]
    );
    let both = server(
        &env,
        &["usage", "--from", "2027-01-13", "--to", "2027-01-14"],
    );
    assert_eq!(String::from_utf8_lossy(&both.stdout).lines().count(), 3);
    // Nothing that names a person: the server's account id only.
    assert!(!printed.contains("alice"));

    let wrong = server(&env, &["usage", "--from", "14/01/2027"]);
    assert!(!wrong.status.success());
    assert!(text(&wrong).contains("YYYY-MM-DD"), "{}", text(&wrong));
    let backwards = server(
        &env,
        &["usage", "--from", "2027-01-14", "--to", "2027-01-13"],
    );
    assert!(text(&backwards).contains("after"), "{}", text(&backwards));
    db.drop().await;
}

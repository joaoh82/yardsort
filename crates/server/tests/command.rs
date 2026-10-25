//! `yardsort-server revoke-account`, run as the operator runs it: the built binary, against
//! PostgreSQL (see `common/pg.rs`).

mod common;

use std::process::{Command, Output};

use remote_proto::account::DeviceKind;
use remote_proto::{DeviceId, Platform};
use yardsort_server::records::{AccountId, Device, Issued, Records, RevokeReason};
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

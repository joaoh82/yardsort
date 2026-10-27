//! Pair a pretend device with a host, as the phone app will, or be a pretend host.
//!
//! ```text
//! cargo run -p yardsort-server --example pair -- phone [SERVER] CODE [--wait-for-revoke] [--zitadel-api]
//! cargo run -p yardsort-server --example pair -- host [SERVER] [--control] [--revoke-after SECONDS] [--zitadel-api]
//! ```
//!
//! `SERVER` is `http://127.0.0.1:8787` unless given.
//!
//! **phone** signs in as a phone, redeems the host's pairing `CODE`, asks to watch and type, and
//! waits up to five minutes for the host's answer. With `--wait-for-revoke` it then waits for
//! the grant to be taken back. It signs itself out at the end.
//!
//! **host** stands in for the app as a host: it signs in as one, registers, prints a pairing code,
//! and approves the first device that redeems it: to watch, or to watch and type with
//! `--control`, on every project. With `--revoke-after` it takes the grant back after that many
//! seconds. It removes itself and signs out at the end.
//!
//! `--zitadel-api` signs in without a browser, as the sign-in example does (see `sign-in.rs`).
//! Credentials and the QR code's secret are never printed. The short code is: it is what a person
//! reads off the host.

use std::time::Duration;

use remote_client::{Client, Session, SIGN_IN_TIMEOUT};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::pairing::{
    GrantDecision, GrantState, GrantSummary, HostRegistration, OfferKey, PairingRequest,
};
use remote_proto::{Platform, ProjectGrant, Scope};

mod common;
use common::{open, through_zitadel_api};

const WATCH: [Scope; 4] = [
    Scope::WorkspaceRead,
    Scope::ActivityRead,
    Scope::MemoryRead,
    Scope::TerminalRead,
];

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let mut plain = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(seconds) = value("--revoke-after") {
        plain.retain(|a| *a != seconds);
    }
    let role = plain.first().cloned().unwrap_or_default();
    let rest = &plain[1.min(plain.len())..];
    let (server, rest) = match rest.first() {
        Some(first) if first.contains("://") => (first.as_str(), &rest[1..]),
        _ => ("http://127.0.0.1:8787", rest),
    };
    let client = Client::new(server).map_err(|e| e.to_string())?;
    let zitadel = flag("--zitadel-api");
    match role.as_str() {
        "phone" => {
            let code = rest
                .first()
                .ok_or("give the code the host shows: pair phone [SERVER] CODE")?;
            phone(&client, code, zitadel, flag("--wait-for-revoke")).await
        }
        "host" => {
            let revoke_after = value("--revoke-after")
                .map(|s| {
                    s.parse::<u64>()
                        .map_err(|_| format!("{s:?} is not seconds"))
                })
                .transpose()?;
            host(&client, zitadel, flag("--control"), revoke_after).await
        }
        _ => Err("pair phone [SERVER] CODE, or pair host [SERVER]".into()),
    }
}

async fn sign_in(
    client: &Client,
    kind: DeviceKind,
    name: &str,
    zitadel: bool,
) -> Result<Session, String> {
    let sign_in = client.begin_sign_in().await.map_err(|e| e.to_string())?;
    let device = NewDevice {
        name: name.into(),
        kind,
        platform: Platform::current(),
    };
    let credentials = if zitadel {
        let http = yardsort_server::oidc::http_client_builder()
            .build()
            .map_err(|e| e.to_string())?;
        let url = sign_in.url().clone();
        let (credentials, browsed) = tokio::join!(
            sign_in.finish(client, device, Duration::from_secs(60)),
            through_zitadel_api(&http, url),
        );
        browsed?;
        credentials
    } else {
        println!("Sign in, in a browser:\n\n  {}\n", sign_in.url());
        open(sign_in.url().as_str());
        sign_in.finish(client, device, SIGN_IN_TIMEOUT).await
    }
    .map_err(|e| e.to_string())?;
    println!("Signed in as a {kind:?}, device {}.", credentials.device);
    Ok(Session::new(client.clone(), credentials))
}

/// Nothing is saved, so a new refresh credential needs no keeping.
fn kept(_: &Secret) -> Result<(), String> {
    Ok(())
}

async fn phone(
    client: &Client,
    code: &str,
    zitadel: bool,
    wait_for_revoke: bool,
) -> Result<(), String> {
    let mut session = sign_in(client, DeviceKind::Phone, "Pairing tool phone", zitadel).await?;
    let access = session.access(kept).await.map_err(|e| e.to_string())?;
    let mut asked: Vec<Scope> = WATCH.into();
    asked.push(Scope::TerminalControl);
    let request = PairingRequest {
        offer: OfferKey::Code(Secret::new(code)),
        scopes: asked.into_iter().collect(),
    };
    let grant = client
        .pair(&access, &request)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "Asked {} to be let in; waiting for its answer.",
        grant.host_name
    );
    let answered = wait(&mut session, &grant, Duration::from_secs(300), |g| {
        g.state != GrantState::Pending
    })
    .await?;
    println!("{}", described(&answered));
    let outcome = if answered.state != GrantState::Approved {
        Err(format!("not let in: {:?}", answered.state))
    } else if wait_for_revoke {
        println!("Waiting for the grant to be taken back, for up to ten minutes.");
        let taken = wait(&mut session, &answered, Duration::from_secs(600), |g| {
            g.state == GrantState::Revoked
        })
        .await?;
        println!("{}", described(&taken));
        Ok(())
    } else {
        Ok(())
    };
    // Signing out revokes the device's grant too, so the host drops it from its list.
    println!(
        "Signing out; {} no longer lists this device.",
        grant.host_name
    );
    let access = session.access(kept).await.map_err(|e| e.to_string())?;
    let _ = client.revoke_device(&access, &session.device).await;
    outcome
}

async fn host(
    client: &Client,
    zitadel: bool,
    control: bool,
    revoke_after: Option<u64>,
) -> Result<(), String> {
    let mut session = sign_in(client, DeviceKind::Host, "Pairing tool host", zitadel).await?;
    let access = session.access(kept).await.map_err(|e| e.to_string())?;
    let host = client
        .register_host(
            &access,
            &HostRegistration {
                host: None,
                name: "Pairing tool host".into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let offer = client
        .offer(&access, &host.id)
        .await
        .map_err(|e| e.to_string())?;
    println!("Code: {}", offer.code.expose());
    println!("Waiting for a device to redeem it, for up to five minutes.");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let asked = loop {
        let access = session.access(kept).await.map_err(|e| e.to_string())?;
        let page = client
            .host_grants(&access, &host.id)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(asked) = page
            .grants
            .into_iter()
            .find(|g| g.state == GrantState::Pending)
        {
            break asked;
        }
        if tokio::time::Instant::now() > deadline {
            return Err("nobody redeemed the code".into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let mut offered: Vec<Scope> = WATCH.into();
    if control {
        offered.push(Scope::TerminalControl);
    }
    let decision = GrantDecision {
        scopes: asked
            .requested
            .iter()
            .filter(|s| offered.contains(s))
            .copied()
            .collect(),
        projects: ProjectGrant::All,
    };
    let access = session.access(kept).await.map_err(|e| e.to_string())?;
    let given = client
        .decide(&access, &asked.id, Some(&decision))
        .await
        .map_err(|e| e.to_string())?;
    println!("{}", described(&given));
    if let Some(seconds) = revoke_after {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        let access = session.access(kept).await.map_err(|e| e.to_string())?;
        client
            .revoke_grant(&access, &given.id)
            .await
            .map_err(|e| e.to_string())?;
        println!("Took {}'s grant back.", given.device_name);
    }
    let access = session.access(kept).await.map_err(|e| e.to_string())?;
    let _ = client.remove_host(&access, &host.id).await;
    let _ = client.revoke_device(&access, &session.device).await;
    Ok(())
}

/// Poll the grant until `done`, or give up at the deadline.
async fn wait(
    session: &mut Session,
    grant: &GrantSummary,
    within: Duration,
    done: impl Fn(&GrantSummary) -> bool,
) -> Result<GrantSummary, String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let access = session.access(kept).await.map_err(|e| e.to_string())?;
        let now = session
            .client
            .grant(&access, &grant.id)
            .await
            .map_err(|e| e.to_string())?;
        if done(&now) {
            return Ok(now);
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("still {:?} after {}s", now.state, within.as_secs()));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn described(grant: &GrantSummary) -> String {
    let projects = match &grant.projects {
        Some(ProjectGrant::All) => "every project".to_owned(),
        Some(ProjectGrant::Only(p)) => format!("{} project(s)", p.len()),
        None => "no project".to_owned(),
    };
    match grant.state {
        GrantState::Approved => format!(
            "Let in by {}: {} on {projects}.",
            grant.host_name,
            if grant.scopes.contains(&Scope::TerminalControl) {
                "watch and type"
            } else {
                "watch"
            }
        ),
        GrantState::Revoked => format!(
            "Revoked: {} no longer lets {} in.",
            grant.host_name, grant.device_name
        ),
        other => format!("{other:?}."),
    }
}

//! Two pretend hosts of one account sync through a server, as the app and `yardsort-host` will:
//! R6 slice 1's smoke test (docs/design/42-remote-r6.md).
//!
//! ```text
//! cargo run -p yardsort-server --example sync -- two-hosts [SERVER] --keep FILE [--follow | --zitadel-api]
//! cargo run -p yardsort-server --example sync -- other-account [SERVER] --keep FILE [--follow | --zitadel-api]
//! ```
//!
//! `SERVER` is `http://127.0.0.1:8787` unless given. Each device signs in in a browser, unless
//! `--follow` follows the provider's redirects without one, which only the stand-in's provider
//! (`examples/stand-in.rs`) lets through, or `--zitadel-api` signs in through ZITADEL's API, as
//! the sign-in example does.
//!
//! **two-hosts** signs in a phone and two hosts, registers the hosts, and opens each one's relay
//! link. Then it checks, saying each as it passes: a phone may not sync; one host turns a project
//! on with a note, the other is offered it and may not write to it until it links it; a push sent
//! twice is applied once; an edit on one host is a hint on the other's link and not on its own;
//! a stale edit is a conflict with the other's text; a revoke over a stale base wins, and a stale
//! edit after it does not bring the note back; a deleted record refuses a stale create; a cursor
//! the server never gave asks for a snapshot; a push over the limit is too large. It writes the
//! project's id to `FILE`, and leaves the project and both hosts for `other-account`.
//!
//! **other-account** signs in a host as whoever the provider signs in now, which must be another
//! account, and checks it is offered nothing, pulls nothing, and can neither snapshot, link nor
//! write the project in `FILE`. Then it removes its host.
//!
//! With `--clean`, `two-hosts` ends by removing the project from the server and both hosts from
//! the account, which a run against staging should do.
//!
//! Credentials are never printed. Record bodies are made up here and say nothing about anyone.

use std::time::Duration;

use remote_client::{Client, ClientError, Incoming, LinkReader, RelayLink, Session};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::pairing::HostRegistration;
use remote_proto::relay::{HostNotice, LinkFrame, TicketRequest, NOTICE_CHANNEL};
use remote_proto::sync::{
    Action, OpId, OpResult, PushRequest, RecordKind, SyncId, SyncOp, MAX_RECORD_BODY,
};
use remote_proto::{ErrorCode, HostId, Platform};
use serde_json::json;

mod common;
use common::{open, through_zitadel_api};

/// How long a hint has to arrive, and how long to wait to say one did not.
const HINT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let keep = args
        .iter()
        .position(|a| a == "--keep")
        .and_then(|at| args.get(at + 1))
        .cloned()
        .ok_or("say where to keep the project's id: --keep FILE")?;
    let plain: Vec<&String> = args
        .iter()
        .filter(|a| !a.starts_with("--") && **a != keep)
        .collect();
    let mode = plain.first().map(|s| s.as_str()).unwrap_or_default();
    let server = plain
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("http://127.0.0.1:8787");
    let client = Client::new(server).map_err(|e| e.to_string())?;
    let how = if flag("--zitadel-api") {
        How::Zitadel
    } else if flag("--follow") {
        How::Follow
    } else {
        How::Browser
    };
    match mode {
        "two-hosts" => two_hosts(&client, how, &keep).await,
        "other-account" => other_account(&client, how, &keep).await,
        _ => Err(
            "sync two-hosts [SERVER] --keep FILE, or sync other-account [SERVER] --keep FILE"
                .into(),
        ),
    }
}

#[derive(Clone, Copy)]
enum How {
    Browser,
    Follow,
    Zitadel,
}

async fn sign_in(
    client: &Client,
    kind: DeviceKind,
    name: &str,
    how: How,
) -> Result<Secret, String> {
    let sign_in = client.begin_sign_in().await.map_err(|e| e.to_string())?;
    let device = NewDevice {
        name: name.into(),
        kind,
        platform: Platform::current(),
    };
    let http = yardsort_server::oidc::http_client_builder()
        .build()
        .map_err(|e| e.to_string())?;
    let url = sign_in.url().clone();
    let credentials = match how {
        How::Browser => {
            println!("Sign {name} in, in a browser:\n\n  {url}\n");
            open(url.as_str());
            sign_in
                .finish(client, device, remote_client::SIGN_IN_TIMEOUT)
                .await
        }
        How::Follow => {
            // As a browser would, to the listener on this machine at the end.
            let browser = yardsort_server::oidc::http_client_builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .map_err(|e| e.to_string())?;
            let follow = async {
                // The listener answers the last redirect; its page is not needed.
                let _ = browser.get(url).send().await;
                Ok::<_, String>(())
            };
            let (credentials, _) = tokio::join!(
                sign_in.finish(client, device, Duration::from_secs(60)),
                follow
            );
            credentials
        }
        How::Zitadel => {
            let (credentials, browsed) = tokio::join!(
                sign_in.finish(client, device, Duration::from_secs(60)),
                through_zitadel_api(&http, url),
            );
            browsed?;
            credentials
        }
    }
    .map_err(|e| e.to_string())?;
    let mut session = Session::new(client.clone(), credentials);
    session.access(|_| Ok(())).await.map_err(|e| e.to_string())
}

/// A host device, signed in and registered.
async fn host(client: &Client, name: &str, how: How) -> Result<(Secret, HostId), String> {
    let access = sign_in(client, DeviceKind::Host, name, how).await?;
    let host = client
        .register_host(
            &access,
            &HostRegistration {
                host: None,
                name: name.into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok((access, host.id))
}

fn new_id() -> SyncId {
    SyncId::new(uuid::Uuid::new_v4().to_string()).expect("a UUID v4")
}

fn new_op() -> OpId {
    OpId::new(uuid::Uuid::new_v4().to_string()).expect("a UUID v4")
}

fn put(
    kind: RecordKind,
    id: &SyncId,
    project: &SyncId,
    base: Option<u64>,
    body: serde_json::Value,
) -> SyncOp {
    SyncOp {
        op: new_op(),
        action: Action::Put,
        kind,
        id: id.clone(),
        project: project.clone(),
        base,
        schema: Some(1),
        body: Some(body),
    }
}

fn change(
    action: Action,
    kind: RecordKind,
    id: &SyncId,
    project: &SyncId,
    base: Option<u64>,
) -> SyncOp {
    SyncOp {
        op: new_op(),
        action,
        kind,
        id: id.clone(),
        project: project.clone(),
        base,
        schema: None,
        body: None,
    }
}

async fn push(client: &Client, access: &Secret, ops: Vec<SyncOp>) -> Result<Vec<OpResult>, String> {
    client
        .sync_push(access, &PushRequest { ops })
        .await
        .map(|reply| reply.results)
        .map_err(|e| e.to_string())
}

async fn one(client: &Client, access: &Secret, op: SyncOp) -> Result<OpResult, String> {
    Ok(push(client, access, vec![op]).await?.remove(0))
}

fn revision(result: &OpResult) -> Result<u64, String> {
    match result {
        OpResult::Applied { revision, .. } => Ok(*revision),
        other => Err(format!("not applied: {other:?}")),
    }
}

fn refused_with(error: &ClientError) -> Option<ErrorCode> {
    error.code()
}

/// Whether a sync hint arrives on `link` within [`HINT`].
async fn hinted(link: &mut LinkReader) -> bool {
    let Ok(Ok(Incoming::Binary(bytes))) = tokio::time::timeout(HINT, link.next()).await else {
        return false;
    };
    matches!(
        LinkFrame::decode(&bytes),
        Ok(LinkFrame::Text(NOTICE_CHANNEL, text))
            if serde_json::from_slice::<HostNotice>(text).ok() == Some(HostNotice::SyncChanged)
    )
}

fn passed(what: &str) {
    println!("ok  {what}");
}

fn check(holds: bool, what: &str) -> Result<(), String> {
    if holds {
        passed(what);
        Ok(())
    } else {
        Err(format!("FAILED  {what}"))
    }
}

async fn link(client: &Client, access: &Secret, host: &HostId) -> Result<RelayLink, String> {
    client
        .open_relay(
            access,
            &TicketRequest::Host {
                host: host.clone(),
                protocol: Some(remote_proto::ProtocolVersion::CURRENT),
            },
        )
        .await
        .map_err(|e| e.to_string())
}

async fn two_hosts(client: &Client, how: How, keep: &str) -> Result<(), String> {
    let phone = sign_in(client, DeviceKind::Phone, "Sync smoke phone", how).await?;
    let (here, here_id) = host(client, "Sync smoke here", how).await?;
    let (there, there_id) = host(client, "Sync smoke there", how).await?;
    let mut here_link = link(client, &here, &here_id).await?;
    let mut there_link = link(client, &there, &there_id).await?;
    println!("Signed in a phone and two hosts, each with its relay link.");

    let project = new_id();
    let note = new_id();
    let workspace = new_id();
    let refused = client
        .sync_push(
            &phone,
            &PushRequest {
                ops: vec![put(
                    RecordKind::Project,
                    &project,
                    &project,
                    None,
                    json!({"name": "smoke"}),
                )],
            },
        )
        .await;
    check(
        refused.as_ref().err().and_then(refused_with) == Some(ErrorCode::Forbidden),
        "a phone may not sync",
    )?;

    let made = push(
        client,
        &here,
        vec![
            put(
                RecordKind::Project,
                &project,
                &project,
                None,
                json!({"name": "smoke", "remote": "https://example.com/smoke.git"}),
            ),
            put(
                RecordKind::Note,
                &note,
                &project,
                None,
                json!({"text": "Run the tests first."}),
            ),
            put(
                RecordKind::Workspace,
                &workspace,
                &project,
                None,
                json!({"name": "fix-login"}),
            ),
        ],
    )
    .await?;
    check(
        made.iter().all(|r| matches!(r, OpResult::Applied { .. })),
        "one host turns a project on, with a note and a workspace",
    )?;
    std::fs::write(keep, project.as_str()).map_err(|e| format!("{keep}: {e}"))?;

    let offered = client
        .sync_projects(&there)
        .await
        .map_err(|e| e.to_string())?;
    let mine = offered.projects.iter().find(|p| p.record.id == project);
    check(
        mine.is_some_and(|p| {
            !p.linked
                && p.record.body.as_ref().map(|b| b["remote"].clone())
                    == Some(json!("https://example.com/smoke.git"))
        }),
        "the other host is offered it, with its hint, unlinked",
    )?;
    let early = one(
        client,
        &there,
        put(
            RecordKind::Note,
            &note,
            &project,
            Some(1),
            json!({"text": "Mine."}),
        ),
    )
    .await?;
    check(
        matches!(&early, OpResult::Refused { error, .. } if error.code == ErrorCode::Forbidden),
        "and may not write to it before linking it",
    )?;
    client
        .sync_link(&there, &project, true)
        .await
        .map_err(|e| e.to_string())?;
    let snapshot = client
        .sync_snapshot(&there, Some(&project), None)
        .await
        .map_err(|e| e.to_string())?;
    check(
        snapshot.records.len() == 3,
        "linked, its snapshot has the project's three records",
    )?;

    let edit = put(
        RecordKind::Note,
        &note,
        &project,
        Some(1),
        json!({"text": "Run the tests first, always."}),
    );
    let first = one(client, &there, edit.clone()).await?;
    let again = one(client, &there, edit).await?;
    check(
        revision(&first)? == 2 && first == again,
        "an edit pushed twice is applied once, and answered the same",
    )?;
    check(
        hinted(&mut here_link.reader).await,
        "the first host hears of it on its link",
    )?;
    check(
        !hinted(&mut there_link.reader).await,
        "the host that pushed it does not",
    )?;
    let page = client
        .sync_pull(
            &here,
            made.iter()
                .filter_map(|r| match r {
                    OpResult::Applied { cursor, .. } => Some(*cursor),
                    _ => None,
                })
                .max()
                .unwrap_or(0),
        )
        .await
        .map_err(|e| e.to_string())?;
    check(
        page.records.len() == 1
            && page.records[0].host == there_id
            && page.records[0].body.as_ref().map(|b| b["text"].clone())
                == Some(json!("Run the tests first, always.")),
        "and pulls the other host's text",
    )?;

    let stale = one(
        client,
        &here,
        put(
            RecordKind::Note,
            &note,
            &project,
            Some(1),
            json!({"text": "Run the tests later."}),
        ),
    )
    .await?;
    check(
        matches!(&stale, OpResult::Conflict { current, .. }
            if current.revision == 2
                && current.body.as_ref().map(|b| b["text"].clone())
                    == Some(json!("Run the tests first, always."))),
        "an edit made against the older text is a conflict, with the newer text",
    )?;
    let revoke = one(
        client,
        &here,
        change(Action::Revoke, RecordKind::Note, &note, &project, Some(1)),
    )
    .await?;
    check(
        revision(&revoke)? == 3,
        "a revoke made against the older text still applies",
    )?;
    let resurrect = one(
        client,
        &there,
        put(
            RecordKind::Note,
            &note,
            &project,
            Some(2),
            json!({"text": "Run the tests first, always."}),
        ),
    )
    .await?;
    check(
        matches!(&resurrect, OpResult::Conflict { current, .. } if current.revoked),
        "an edit from before the revoke does not bring the note back",
    )?;

    let deleted = one(
        client,
        &there,
        change(
            Action::Delete,
            RecordKind::Workspace,
            &workspace,
            &project,
            Some(1),
        ),
    )
    .await?;
    let recreated = one(
        client,
        &here,
        put(
            RecordKind::Workspace,
            &workspace,
            &project,
            None,
            json!({"name": "fix-login"}),
        ),
    )
    .await?;
    check(
        revision(&deleted)? == 2
            && matches!(&recreated, OpResult::Conflict { current, .. } if current.deleted),
        "a deleted workspace refuses a stale create",
    )?;

    let ahead = client.sync_pull(&here, 1 << 40).await;
    check(
        ahead.as_ref().err().and_then(refused_with) == Some(ErrorCode::CursorExpired),
        "a cursor the server never gave asks for a snapshot",
    )?;

    let text = "x".repeat(MAX_RECORD_BODY - 100);
    let over: Vec<SyncOp> = (0..20)
        .map(|_| {
            put(
                RecordKind::Note,
                &new_id(),
                &project,
                None,
                json!({ "text": text }),
            )
        })
        .collect();
    let too_large = client.sync_push(&here, &PushRequest { ops: over }).await;
    check(
        matches!(&too_large, Err(ClientError::Remote(error)) if error.code == ErrorCode::InvalidRequest),
        "a push over the limit is refused whole",
    )?;
    let after = client
        .sync_snapshot(&here, Some(&project), None)
        .await
        .map_err(|e| e.to_string())?;
    check(after.records.len() == 3, "and nothing of it was kept")?;

    if std::env::args().any(|a| a == "--clean") {
        clean(
            client,
            &here,
            &project,
            &[(&here, &here_id), (&there, &there_id)],
        )
        .await?;
    }
    drop((here_link, there_link));
    println!("All passed.");
    Ok(())
}

async fn other_account(client: &Client, how: How, keep: &str) -> Result<(), String> {
    let project = std::fs::read_to_string(keep).map_err(|e| format!("{keep}: {e}"))?;
    let project = SyncId::new(project.trim()).map_err(|e| e.to_string())?;
    let (theirs, theirs_id) = host(client, "Sync smoke elsewhere", how).await?;
    let offered = client
        .sync_projects(&theirs)
        .await
        .map_err(|e| e.to_string())?;
    check(
        offered.projects.iter().all(|p| p.record.id != project),
        "another account is not offered the project",
    )?;
    let page = client
        .sync_pull(&theirs, 0)
        .await
        .map_err(|e| e.to_string())?;
    check(page.records.is_empty(), "and pulls nothing")?;
    let snapshot = client.sync_snapshot(&theirs, Some(&project), None).await;
    let linked = client.sync_link(&theirs, &project, true).await;
    check(
        snapshot.as_ref().err().and_then(refused_with) == Some(ErrorCode::NotFound)
            && linked.as_ref().err().and_then(refused_with) == Some(ErrorCode::NotFound),
        "and can neither take a snapshot of it nor link it, knowing its id",
    )?;
    let written = one(
        client,
        &theirs,
        put(
            RecordKind::Note,
            &new_id(),
            &project,
            None,
            json!({"text": "Theirs."}),
        ),
    )
    .await?;
    check(
        matches!(&written, OpResult::Refused { error, .. } if error.code == ErrorCode::NotFound),
        "nor write to it",
    )?;
    client
        .remove_host(&theirs, &theirs_id)
        .await
        .map_err(|e| e.to_string())?;
    println!("All passed.");
    Ok(())
}

/// Remove the project from the server, and the hosts from the account.
async fn clean(
    client: &Client,
    by: &Secret,
    project: &SyncId,
    hosts: &[(&Secret, &HostId)],
) -> Result<(), String> {
    let removed = one(
        client,
        by,
        change(
            Action::Delete,
            RecordKind::Project,
            project,
            project,
            Some(1),
        ),
    )
    .await?;
    check(
        revision(&removed)? == 2,
        "the project is removed from the server",
    )?;
    for (access, host) in hosts {
        client
            .remove_host(access, host)
            .await
            .map_err(|e| e.to_string())?;
    }
    println!("Removed both hosts.");
    Ok(())
}

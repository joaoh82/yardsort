//! Pair a pretend device with a host, as the phone app will, or be a pretend host.
//!
//! ```text
//! cargo run -p yardsort-server --example pair -- phone [SERVER] CODE [--relay [--type TEXT]] [--wait-for-revoke] [--zitadel-api]
//! cargo run -p yardsort-server --example pair -- host [SERVER] [--control] [--serve] [--revoke-after SECONDS] [--zitadel-api]
//! ```
//!
//! `SERVER` is `http://127.0.0.1:8787` unless given.
//!
//! **phone** signs in as a phone, redeems the host's pairing `CODE`, asks to watch and type, and
//! waits up to five minutes for the host's answer. With `--wait-for-revoke` it then waits for
//! the grant to be taken back. It signs itself out at the end.
//!
//! With `--relay` it then reaches the host through the server's relay, as the phone app will:
//! greets it, finds a running terminal in a workspace it may see, and watches it from a snapshot,
//! checking that every item comes numbered without a gap. `--type TEXT` takes control and types
//! `TEXT` and Enter. It watches for ten seconds, or, with `--wait-for-revoke`, until the relay
//! closes the socket, and says why it closed.
//!
//! **host** stands in for the app as a host: it signs in as one, registers, prints a pairing code,
//! and approves the first device that redeems it: to watch, or to watch and type with
//! `--control`, on every project. With `--revoke-after` it takes the grant back after that many
//! seconds. It removes itself and signs out at the end.
//!
//! With `--serve` it is a host a device can reach: it keeps a link to the relay with the app's
//! own connector (`remote_host`), and answers from a throwaway workspace with one program in a
//! real terminal, which prints back every line typed into it (`got:` in front). At the end it
//! says what the program was sent, and that it was still running when the link closed.
//!
//! `--zitadel-api` signs in without a browser, as the sign-in example does (see `sign-in.rs`).
//! Credentials and the QR code's secret are never printed. The short code is: it is what a person
//! reads off the host.

use std::sync::Arc;
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
    for valued in ["--revoke-after", "--type"] {
        if let Some(given) = value(valued) {
            plain.retain(|a| *a != given);
        }
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
            let relay = flag("--relay").then(|| Relay {
                typed: value("--type"),
            });
            phone(&client, code, zitadel, flag("--wait-for-revoke"), relay).await
        }
        "host" => {
            let revoke_after = value("--revoke-after")
                .map(|s| {
                    s.parse::<u64>()
                        .map_err(|_| format!("{s:?} is not seconds"))
                })
                .transpose()?;
            host(
                &client,
                zitadel,
                flag("--control"),
                flag("--serve"),
                revoke_after,
            )
            .await
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

/// What to do through the relay, once let in.
struct Relay {
    typed: Option<String>,
}

async fn phone(
    client: &Client,
    code: &str,
    zitadel: bool,
    wait_for_revoke: bool,
    relay: Option<Relay>,
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
    } else if let Some(relay) = relay {
        let access = session.access(kept).await.map_err(|e| e.to_string())?;
        through_relay(client, &access, &answered, relay, wait_for_revoke).await
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
    serve: bool,
    revoke_after: Option<u64>,
) -> Result<(), String> {
    let session = sign_in(client, DeviceKind::Host, "Pairing tool host", zitadel).await?;
    let session = Arc::new(tokio::sync::Mutex::new(session));
    let access = || async {
        session
            .lock()
            .await
            .access(kept)
            .await
            .map_err(|e| e.to_string())
    };
    let host = client
        .register_host(
            &access().await?,
            &HostRegistration {
                host: None,
                name: "Pairing tool host".into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    // Connected before anyone is let in, so a device never finds it offline.
    let served = if serve {
        let here = serving::Pretend::new(&host)?;
        let connector = remote_host::Connector::start(
            host.id.clone(),
            Arc::new(serving::Shared(Arc::clone(&session))),
            Arc::clone(&here) as Arc<dyn remote_host::Serving>,
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !matches!(connector.status(), remote_host::Status::Connected { .. }) {
            if tokio::time::Instant::now() > deadline {
                return Err(format!(
                    "not connected to the relay: {:?}",
                    connector.status()
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("Connected to the relay, serving one program.");
        Some((here, connector))
    } else {
        None
    };
    let offer = client
        .offer(&access().await?, &host.id)
        .await
        .map_err(|e| e.to_string())?;
    println!("Code: {}", offer.code.expose());
    println!("Waiting for a device to redeem it, for up to five minutes.");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let asked = loop {
        let page = client
            .host_grants(&access().await?, &host.id)
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
    // Kept here first, as the app keeps what its person approved: the device may come through
    // the relay the moment the server says yes.
    if let Some((here, _)) = &served {
        here.let_in(&asked, &decision)?;
    }
    let given = client
        .decide(&access().await?, &asked.id, Some(&decision))
        .await
        .map_err(|e| e.to_string())?;
    println!("{}", described(&given));
    if let Some(seconds) = revoke_after {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        client
            .revoke_grant(&access().await?, &given.id)
            .await
            .map_err(|e| e.to_string())?;
        println!("Took {}'s grant back.", given.device_name);
    }
    if let Some((here, connector)) = served {
        // Long enough for the relay to close the device's socket, and the host its channel.
        tokio::time::sleep(Duration::from_secs(2)).await;
        connector.stop().await;
        println!("{}", here.report());
    }
    let access = access().await?;
    let _ = client.remove_host(&access, &host.id).await;
    let _ = client
        .revoke_device(&access, &session.lock().await.device)
        .await;
    Ok(())
}

/// The pretend host's records and terminal, for `--serve`.
mod serving {
    use std::sync::{Arc, Mutex};

    use pty_host::{LaunchPlan, Numbered, PtyHost, SessionId, SessionState, TerminalHost};
    use remote_client::Session;
    use remote_host::{Access, AccessError, AccessFuture, Serving};
    use remote_proto::pairing::{GrantDecision, GrantSummary, HostSummary};
    use yardsort_core::git::Git;
    use yardsort_core::remote::serve::Context;
    use yardsort_core::remote::HostIdentity;
    use yardsort_core::store::Store;

    /// The host's session, shared between the tool and the connector.
    pub struct Shared(pub Arc<tokio::sync::Mutex<Session>>);

    impl Access for Shared {
        fn access(&self) -> AccessFuture<'_> {
            Box::pin(async move {
                let mut session = self.0.lock().await;
                let access = session
                    .access(super::kept)
                    .await
                    .map_err(|e| AccessError::Unavailable(e.to_string()))?;
                Ok((session.client.clone(), access))
            })
        }
    }

    pub struct Pretend {
        _dir: tempfile::TempDir,
        store: Store,
        git: Git,
        terminals: PtyHost,
        identity: HostIdentity,
        program: SessionId,
        seen: Arc<Mutex<String>>,
    }

    impl Pretend {
        /// A throwaway workspace with one program in it, watched from here.
        pub fn new(host: &HostSummary) -> Result<Arc<Self>, String> {
            let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
            let store = Store::open(&dir.path().join("yardsort.db")).map_err(|e| e.to_string())?;
            let project = store
                .add_project("demo", &dir.path().join("demo").to_string_lossy())
                .map_err(|e| e.to_string())?;
            let workspace = store
                .add_worktree(
                    &project.id,
                    "work",
                    &dir.path().join("demo-wt").to_string_lossy(),
                    None,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let terminals = PtyHost::new(Arc::new(|_| {}));
            let program = terminals
                .spawn(echo_lines(&workspace.id))
                .map_err(|e| e.to_string())?;
            let seen = Arc::new(Mutex::new(String::new()));
            let watching = Arc::clone(&seen);
            // Watched, not attached as a terminal: a terminal is the one left to answer the
            // program's queries, and on Windows ConPTY waits on its first one until answered.
            terminals
                .attach_numbered(
                    &program.id,
                    Box::new(move |item| {
                        if let Numbered::Output { bytes, .. } = item {
                            watching
                                .lock()
                                .unwrap()
                                .push_str(&String::from_utf8_lossy(bytes));
                        }
                        true
                    }),
                )
                .map_err(|e| e.to_string())?;
            Ok(Arc::new(Self {
                _dir: dir,
                store,
                git: yardsort_core::git::testing::git(),
                terminals,
                identity: HostIdentity {
                    id: host.id.clone(),
                    name: host.name.clone(),
                },
                program: program.id,
                seen,
            }))
        }

        /// Keep what this host approves for the device that asked.
        pub fn let_in(&self, asked: &GrantSummary, decision: &GrantDecision) -> Result<(), String> {
            yardsort_core::remote::host::keep_approved(
                &self.store,
                &asked.id,
                &asked.device,
                &asked.device_name,
                &decision.scopes,
                &decision.projects,
                0,
            )
            .map_err(|e| e.to_string())
        }

        /// What the program printed back, and whether it is still running.
        pub fn report(&self) -> String {
            let seen = self.seen.lock().unwrap();
            // Only what the program printed is kept, but a line can still start after the
            // escapes the program drew before it.
            let lines: Vec<&str> = seen
                .split(['\r', '\n'])
                .filter_map(|l| l.find("got:").map(|at| &l[at..]))
                .collect();
            if lines.is_empty() {
                let tail = seen.len().saturating_sub(400);
                let tail = (tail..=seen.len())
                    .find(|&at| seen.is_char_boundary(at))
                    .unwrap_or(seen.len());
                println!(
                    "Nothing printed back; the terminal here last drew {:?}.",
                    &seen[tail..]
                );
            }
            let running = matches!(
                self.terminals.info(&self.program).map(|i| i.state),
                Ok(SessionState::Running)
            );
            let _ = self.terminals.kill(&self.program);
            format!(
                "The program printed back {lines:?}, and was {} when the link closed.",
                if running {
                    "still running"
                } else {
                    "NOT running"
                }
            )
        }
    }

    impl Serving for Pretend {
        fn with(&self, f: &mut dyn FnMut(&Context<'_>)) {
            f(&Context {
                store: &self.store,
                git: &self.git,
                terminals: &self.terminals,
                host: &self.identity,
                interactive: self.terminals.shares_control(),
            });
        }
    }

    /// Prints back every line it reads, `got:` in front.
    fn echo_lines(workspace: &str) -> LaunchPlan {
        let (program, args): (&str, Vec<String>) = if cfg!(windows) {
            (
                "powershell.exe",
                vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    "while ($true) { $l = [Console]::In.ReadLine(); if ($l -eq $null) { break }; \
                     Write-Output \"got:$l\" }"
                        .into(),
                ],
            )
        } else {
            (
                "/bin/sh",
                vec![
                    "-c".into(),
                    "stty -echo; while IFS= read -r line; do echo \"got:$line\"; done".into(),
                ],
            )
        };
        LaunchPlan {
            program: program.into(),
            args,
            cwd: None,
            env: vec![],
            clear_env: false,
            size: pty_host::TermSize { cols: 80, rows: 24 },
            labels: [(
                yardsort_core::launch::WORKSPACE_LABEL.to_owned(),
                workspace.to_owned(),
            )]
            .into(),
            prompt: None,
        }
    }
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

/// Reach the host through the relay: greet it, watch a running terminal, maybe type into it.
async fn through_relay(
    client: &Client,
    access: &Secret,
    grant: &GrantSummary,
    relay: Relay,
    wait_for_revoke: bool,
) -> Result<(), String> {
    use remote_client::Incoming;
    use remote_proto::relay::TicketRequest;
    use serde_json::{json, Value};

    let request = TicketRequest::Device {
        host: grant.host.clone(),
    };
    // A host a moment from connecting is offline for that moment.
    let mut tries = 0;
    let link = loop {
        match client.open_relay(access, &request).await {
            Ok(link) => break link,
            Err(e) if e.code() == Some(remote_proto::ErrorCode::HostOffline) && tries < 10 => {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(format!("the relay: {e}")),
        }
    };
    let (mut reader, mut writer) = (link.reader, link.writer);
    println!("Connected to {} through the relay.", grant.host_name);
    let hello = json!({"client": "pairing-tool/1", "protocols": [1]});
    writer
        .text(hello.to_string())
        .await
        .map_err(|e| e.to_string())?;

    let mut items: Vec<u64> = Vec::new();
    // What the program drew after the snapshot.
    let mut drawn: Vec<u8> = Vec::new();
    let mut stream = None;
    // The next text message, keeping count of output frames on the way.
    macro_rules! next_text {
        () => {
            loop {
                match reader.next().await {
                    Ok(Incoming::Text(text)) => {
                        let value: Value =
                            serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        if value["event"] == "terminal.resized"
                            && Some(&value["stream"]) == stream.as_ref()
                        {
                            items.push(value["position"]["seq"].as_u64().unwrap_or_default());
                            continue;
                        }
                        break value;
                    }
                    Ok(Incoming::Binary(frame)) => {
                        if let Ok(frame) = remote_proto::OutputFrame::decode(&frame) {
                            if items.last() != Some(&frame.position.seq) {
                                items.push(frame.position.seq);
                            }
                            if items.first() != Some(&frame.position.seq) {
                                drawn.extend_from_slice(frame.bytes);
                            }
                        }
                    }
                    Err(reason) => {
                        return Err(format!("the relay closed the socket: {}", reason.text()))
                    }
                }
            }
        };
    }
    let welcome = next_text!();
    let Some(host) = welcome.get("welcome") else {
        return Err(format!("not welcomed: {welcome}"));
    };
    println!(
        "Welcomed by {} ({}).",
        host["host"]["name"].as_str().unwrap_or("?"),
        host["host"]["platform"].as_str().unwrap_or("?")
    );
    let mut asked = 0;
    let mut ask = |op: &str, params: Value| {
        asked += 1;
        let id = format!("r{asked}");
        let request = json!({"requestId": id, "protocol": {"major": 1, "minor": 1}, "op": op, "params": params});
        (id, request.to_string())
    };

    // A running terminal in a workspace this device may see.
    let (id, request) = ask("workspace.list", json!({}));
    writer.text(request).await.map_err(|e| e.to_string())?;
    let page = loop {
        let reply = next_text!();
        if reply["requestId"] == id {
            break reply;
        }
    };
    let mut terminal = None;
    for workspace in page["result"]["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let (id, request) = ask("session.list", json!({"workspace": workspace["id"]}));
        writer.text(request).await.map_err(|e| e.to_string())?;
        let sessions = loop {
            let reply = next_text!();
            if reply["requestId"] == id {
                break reply;
            }
        };
        let running = |t: &Value| t["activity"] != "exited" && t["id"].is_string();
        let found = sessions["result"]["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                (
                    s["title"].as_str().unwrap_or("a conversation").to_owned(),
                    s["terminal"].clone(),
                )
            })
            .chain(
                sessions["result"]["terminals"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        (
                            t["program"].as_str().unwrap_or("a program").to_owned(),
                            t["terminal"].clone(),
                        )
                    }),
            )
            .find(|(_, t)| running(t));
        if let Some((what, t)) = found {
            println!(
                "Watching {what:?} in {}.",
                workspace["name"].as_str().unwrap_or("?")
            );
            terminal = Some(t["id"].clone());
            break;
        }
    }
    let terminal = terminal.ok_or("no running terminal in any workspace this device may see")?;

    let (id, request) = ask("terminal.attach", json!({"terminal": terminal}));
    writer.text(request).await.map_err(|e| e.to_string())?;
    let attached = loop {
        let reply = next_text!();
        if reply["requestId"] == id {
            break reply;
        }
    };
    let result = &attached["result"];
    if result.is_null() {
        return Err(format!(
            "could not watch it: {}",
            attached["error"]["message"]
        ));
    }
    stream = Some(result["stream"].clone());
    println!(
        "Attached at item {}, {}x{}; controlled by {}.",
        result["position"]["seq"],
        result["size"]["cols"],
        result["size"]["rows"],
        result["controller"]["kind"]
    );

    if let Some(text) = relay.typed {
        let (id, request) = ask("terminal.acquire", json!({"terminal": terminal}));
        writer.text(request).await.map_err(|e| e.to_string())?;
        let lease = loop {
            let reply = next_text!();
            if reply["requestId"] == id {
                break reply;
            }
        };
        if lease["result"].is_null() {
            println!("Could not take control: {}", lease["error"]["message"]);
        } else {
            let held = json!({"terminal": terminal, "lease": lease["result"]["id"], "generation": lease["result"]["generation"]});
            let mut input = held.clone();
            input["seq"] = 1.into();
            input["data"] = format!("{text}\r").into();
            let (id, request) = ask("terminal.input", input);
            writer.text(request).await.map_err(|e| e.to_string())?;
            let typed = loop {
                let reply = next_text!();
                if reply["requestId"] == id {
                    break reply;
                }
            };
            match typed["error"]["message"].as_str() {
                None => println!(
                    "Typed {text:?} and Enter, under lease generation {}.",
                    lease["result"]["generation"]
                ),
                Some(why) => println!("Not typed: {why}"),
            }
            let (_, request) = ask("terminal.release", held);
            writer.text(request).await.map_err(|e| e.to_string())?;
        }
    }

    let watching = async {
        loop {
            let event = next_text!();
            if event["event"] == "terminal.detached" {
                return Err::<(), String>(format!("detached: {}", event["error"]["message"]));
            }
        }
    };
    let ended = if wait_for_revoke {
        println!("Watching until the relay closes the socket: revoke this device at the host.");
        watching.await
    } else {
        tokio::time::timeout(Duration::from_secs(10), watching)
            .await
            .unwrap_or(Ok(()))
    };
    let unbroken = items.windows(2).all(|w| w[1] == w[0] + 1);
    println!(
        "Saw {} items from {:?} to {:?}: {}.",
        items.len(),
        items.first(),
        items.last(),
        if unbroken {
            "numbered without a gap"
        } else {
            "NOT IN ORDER"
        }
    );
    println!(
        "After the snapshot the program drew {:?}.",
        String::from_utf8_lossy(&drawn)
    );
    writer.close().await;
    match ended {
        Err(why) if wait_for_revoke => {
            println!("{why}");
            Ok(())
        }
        other => other,
    }
}

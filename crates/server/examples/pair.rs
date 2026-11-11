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
//! `TEXT` and Enter. Then it lets go of the stream and attaches again from the last item it drew,
//! as a phone does after losing its connection, and says whether the host resumed from there.
//! It says when control changes hands and when the program ends. It follows the host's activity
//! from the moment it is welcomed, and prints each event as it arrives. It watches for ten
//! seconds, or, with `--wait-for-revoke`, until the relay closes the socket, and says why it
//! closed, and how many events it heard of.
//!
//! **host** stands in for the app as a host: it signs in as one, registers, prints a pairing code,
//! and approves the first device that redeems it: to watch, or to watch and type with
//! `--control`, on every project. With `--revoke-after` it takes the grant back after that many
//! seconds. It removes itself and signs out at the end.
//!
//! With `--serve` it is a host a device can reach: it keeps a link to the relay with the app's
//! own connector (`remote_host`), and answers from a throwaway workspace with one program in a
//! real terminal, which prints back every line typed into it (`got:` in front). Two seconds after
//! a device comes through the link it starts a second, short program in the same workspace, and
//! records its start and its exit as the app's launcher does, so the device has activity to hear
//! of; `--revoke-after` counts from then. It says when the relay tells it a device asks to pair.
//! At the end it says what the program was sent, and that it was still running when the link
//! closed.
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
        // Serving, the relay says when a device asks; this looks again at once.
        match &served {
            Some((here, _)) => {
                let _ = tokio::time::timeout(Duration::from_secs(1), here.asked.notified()).await;
            }
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
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
    if let Some((here, connector)) = &served {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while !matches!(connector.status(), remote_host::Status::Connected { devices } if devices > 0)
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Long enough for the device to start following.
        tokio::time::sleep(Duration::from_secs(2)).await;
        here.run_briefly()?;
        println!("Started a short program, so the device has activity to hear of.");
    }
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
    use remote_proto::relay::HostNotice;
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
        workspace: String,
        seen: Arc<Mutex<String>>,
        /// The relay said a device asks to pair.
        pub asked: tokio::sync::Notify,
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
                workspace: workspace.id,
                seen,
                asked: tokio::sync::Notify::new(),
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

        /// Start a short program in the workspace, and record its start and its exit as the
        /// app's launcher does.
        pub fn run_briefly(self: &Arc<Self>) -> Result<(), String> {
            use yardsort_core::activity::{
                record_exit, Continuation, ExitFacts, LaunchedBy, Recorder, RunDraft, RunKind, Via,
            };
            let draft = RunDraft {
                workspace_id: self.workspace.clone(),
                session_id: None,
                kind: RunKind::Shell,
                harness_id: None,
                harness_session_id: None,
                model: None,
                effort: None,
                program: "sh".into(),
                continuation: Continuation::Fresh,
            };
            let recorder = Recorder::new(&self.store, &Default::default(), LaunchedBy::App);
            let run = recorder.begin(&draft);
            let plan = if cfg!(windows) {
                ("cmd.exe", "/C", "ping -n 2 127.0.0.1 > NUL")
            } else {
                ("/bin/sh", "-c", "sleep 1")
            };
            let info = self
                .terminals
                .spawn(LaunchPlan {
                    program: plan.0.into(),
                    args: vec![plan.1.into(), plan.2.into()],
                    ..echo_lines(&self.workspace)
                })
                .map_err(|e| e.to_string())?;
            if let Some(run) = &run {
                recorder.spawned(run, &draft, &info.id.0, None);
            }
            let here = Arc::clone(self);
            std::thread::spawn(move || {
                for _ in 0..100 {
                    if let Ok(SessionState::Exited { exit }) =
                        here.terminals.info(&info.id).map(|i| i.state)
                    {
                        record_exit(&here.store, &info.id.0, &ExitFacts::from(&exit), Via::Live);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
            Ok(())
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
                starting: None,
                workflow: None,
            });
        }

        fn told(&self, notice: HostNotice) {
            if let HostNotice::PairingRequested { .. } = notice {
                println!("The relay says a device asks to pair.");
                self.asked.notify_one();
            }
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
    use remote_client::channel::{AttachmentEnd, Followed, Watched};
    use remote_client::Channel;
    use remote_proto::{
        ActivitySubscribeParams, AttachStart, SessionListParams, TerminalActivity,
        WorkspaceListParams,
    };

    // A host a moment from connecting is offline for that moment.
    let mut tries = 0;
    let channel = loop {
        match Channel::open(client, access, &grant.host, "pairing-tool/1").await {
            Ok(channel) => break channel,
            Err(e) if e.code() == Some(remote_proto::ErrorCode::HostOffline) && tries < 10 => {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(format!("the relay: {e}")),
        }
    };
    println!("Connected to {} through the relay.", grant.host_name);
    let host = channel.host();
    println!("Welcomed by {} ({:?}).", host.name, host.platform);
    if let Some(later) = &host.terminals_later {
        println!("Its terminals wait: {}", later.message);
    }

    // Everything this device may see, from now on.
    let heard = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    if host
        .capabilities
        .iter()
        .any(|op| op == "activity.subscribe")
    {
        match channel.subscribe(ActivitySubscribeParams::default()).await {
            Ok(mut subscription) => {
                println!("Following the host's activity.");
                let heard = Arc::clone(&heard);
                tokio::spawn(async move {
                    while let Some(followed) = subscription.next().await {
                        match followed {
                            Followed::Events(batch) => {
                                for event in batch.events {
                                    println!("Activity: {}.", event.kind);
                                    heard.lock().unwrap().push(event.kind);
                                }
                            }
                            Followed::Ended(why) => {
                                println!("Stopped following activity: {why:?}");
                            }
                        }
                    }
                });
            }
            Err(why) => println!("Could not follow the host's activity: {why}"),
        }
    } else {
        println!("This host does not offer activity as it happens.");
    }

    // A running terminal in a workspace this device may see.
    let page = channel
        .workspaces(WorkspaceListParams::default())
        .await
        .map_err(|e| e.to_string())?;
    let mut terminal = None;
    for workspace in &page.workspaces {
        let sessions = channel
            .sessions(SessionListParams {
                workspace: workspace.id.clone(),
                include_ended: false,
                cursor: None,
                limit: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        let running = |activity: TerminalActivity| activity != TerminalActivity::Exited;
        let found = sessions
            .sessions
            .iter()
            .filter_map(|s| s.terminal.as_ref().map(|t| (s.title.clone(), t)))
            .chain(
                sessions
                    .terminals
                    .iter()
                    .map(|t| (t.program.clone(), &t.terminal)),
            )
            .find(|(_, t)| running(t.activity));
        if let Some((what, t)) = found {
            println!("Watching {what:?} in {}.", workspace.name);
            terminal = Some(t.id.clone());
            break;
        }
    }
    let terminal = terminal.ok_or("no running terminal in any workspace this device may see")?;

    let mut attachment = channel
        .attach(terminal.clone(), None)
        .await
        .map_err(|e| format!("could not watch it: {e}"))?;
    let attached = attachment.attached();
    println!(
        "Attached at item {}, {}x{}; controlled by {}.",
        attached.position.seq,
        attached.size.cols,
        attached.size.rows,
        controller(&attached.controller)
    );

    if let Some(text) = relay.typed {
        match channel.acquire(terminal.clone(), false).await {
            Err(why) => println!("Could not take control: {why}"),
            Ok(control) => {
                let generation = control.lease().generation;
                match control.input(&format!("{text}\r"), false).await {
                    Ok(()) => {
                        println!("Typed {text:?} and Enter, under lease generation {generation}.")
                    }
                    Err(why) => println!("Not typed: {why}"),
                }
                let _ = control.release().await;
            }
        }
    }

    // The items seen, and what the program drew after the snapshot, until `stop`.
    let mut items: Vec<u64> = Vec::new();
    let mut drawn: Vec<u8> = Vec::new();
    let mut snapshot = Some(attached.position.seq);
    let watch = |watched: Watched,
                 items: &mut Vec<u64>,
                 drawn: &mut Vec<u8>,
                 snapshot: &mut Option<u64>|
     -> Option<String> {
        match watched {
            Watched::Output { position, bytes } => {
                if items.last() != Some(&position.seq) {
                    items.push(position.seq);
                }
                if *snapshot != Some(position.seq) {
                    *snapshot = None;
                    drawn.extend_from_slice(&bytes);
                }
            }
            Watched::Resized { position, .. } => items.push(position.seq),
            Watched::Control(who) => println!("Control is now {}.", controller(&who)),
            Watched::Exited(exit) => println!("The program ended, with code {}.", exit.code),
            Watched::Ended(AttachmentEnd::Detached(error)) => {
                return Some(format!("detached: {error}"))
            }
            Watched::Ended(why) => return Some(format!("stopped watching: {why:?}")),
        }
        None
    };
    // What came with the snapshot, briefly.
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(watched) = attachment.next().await {
            if let Some(why) = watch(watched, &mut items, &mut drawn, &mut snapshot) {
                return Err::<(), String>(why);
            }
        }
        Ok(())
    })
    .await;

    // Away and back: let go of the stream, and attach again from the last item drawn, as a
    // phone does after losing its connection.
    let left_at = attachment.drawn();
    attachment.detach().await.map_err(|e| e.to_string())?;
    let mut attachment = channel
        .attach(terminal, left_at)
        .await
        .map_err(|e| format!("could not watch it again: {e}"))?;
    let again = attachment.attached();
    let left = left_at.map_or("nothing".to_owned(), |p| format!("item {}", p.seq));
    if again.start == AttachStart::Resumed {
        println!(
            "Came back after {left}: resumed from there; controlled by {}.",
            controller(&again.controller)
        );
    } else {
        println!(
            "Came back after {left}: sent a snapshot at item {} instead.",
            again.position.seq
        );
        items.clear();
        snapshot = Some(again.position.seq);
    }

    let watching = async {
        while let Some(watched) = attachment.next().await {
            if let Some(why) = watch(watched, &mut items, &mut drawn, &mut snapshot) {
                return Err::<(), String>(why);
            }
        }
        Ok(())
    };
    let ended = if wait_for_revoke {
        println!("Watching until the relay closes the socket: revoke this device at the host.");
        tokio::select! {
            ended = watching => ended,
            end = channel.ended() => Err(format!("the relay closed the socket: {end}")),
        }
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
    let heard = heard.lock().unwrap().clone();
    println!(
        "Heard of {} events as they were recorded: {heard:?}.",
        heard.len()
    );
    channel.close();
    match ended {
        Err(why) if wait_for_revoke => {
            println!("{why}");
            Ok(())
        }
        other => other,
    }
}

fn controller(controller: &remote_proto::Controller) -> String {
    match controller {
        remote_proto::Controller::Nobody => "nobody".into(),
        remote_proto::Controller::Local => "the person at the host".into(),
        remote_proto::Controller::Device { name, .. } => format!("device ({name})"),
    }
}

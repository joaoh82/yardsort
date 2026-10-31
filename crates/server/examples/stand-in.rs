//! A server, an identity provider and a host on this machine, for clients written in other
//! languages to test against: the TypeScript protocol client (`packages/remote-client`) runs it.
//!
//! ```text
//! cargo run -q -p yardsort-server --example stand-in
//! ```
//!
//! It starts the server on a free loopback port with its records in memory, beside the stand-in
//! identity provider the server's own tests use, which signs whoever comes to it in at once. Then
//! it signs a host in, registers it, connects it to the relay with the app's own connector, and
//! serves a throwaway workspace with one program in a real terminal, which prints back every line
//! typed into it (`got:` in front). A device's sign-in can come back to `yardsort://signed-in`, the
//! app's own redirect, whose code a test reads from the server's last redirect without a browser,
//! or to a loopback address, as a tool on a computer does.
//!
//! The first line on stdout says where everything is:
//!
//! ```text
//! {"ready":{"server":"http://127.0.0.1:…/","host":"…","hostName":"Stand-in host"}}
//! ```
//!
//! After that it reads one command per line on stdin and answers each with one line,
//! `{"ok":…}` or `{"error":"…"}`. The commands are what the person at the host would do:
//!
//! - `{"op":"offer"}`: a pairing code, its secret and the link a QR code carries.
//! - `{"op":"pending"}`: the first grant waiting for an answer, within ten seconds.
//! - `{"op":"approve","grant":…,"scopes":[…]}`: let it in, kept here first as the app does.
//!   Without `scopes`, with everything it asked for.
//! - `{"op":"deny","grant":…}` and `{"op":"revoke","grant":…}`.
//! - `{"op":"run"}`: start a short program in the workspace and record it, for activity.
//! - `{"op":"typed"}`: the lines the program printed back.
//! - `{"op":"signInAs","subject":…}`: whom the provider signs in from now on, for a second
//!   account. Everyone is `someone` until then.
//! - `{"op":"removeHost"}`: take the host off the account.
//!
//! It stops when stdin closes, so a test that dies takes it with it. Nothing here is a secret
//! worth keeping: the server, its keys and its accounts last as long as the process.

use std::collections::BTreeSet;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{LaunchPlan, Numbered, PtyHost, SessionId, SessionState, TerminalHost};
use remote_client::{Client, Session};
use remote_host::{Access, AccessError, AccessFuture, Connector, Serving, Status};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::{GrantDecision, GrantState, HostRegistration, HostSummary};
use remote_proto::{GrantId, ProjectGrant, Scope};
use serde_json::{json, Value};
use yardsort_core::git::Git;
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::store::Store;
use yardsort_server::records::MemoryRecords;

#[allow(dead_code)]
#[path = "../tests/common/idp.rs"]
mod idp;
#[allow(dead_code)]
#[path = "../tests/common/served.rs"]
mod served;

#[tokio::main]
async fn main() -> Result<(), String> {
    // Every client of the stand-in comes from this machine's one address, so the limits per
    // address are raised past what a test run asks. The server's own tests hold it to them.
    let generous = 10_000;
    let served = served::Served::start_allowing(
        MemoryRecords::default(),
        yardsort_server::http::Options {
            client_ip_header: None,
            logins_per_minute: generous,
            tokens_per_minute: generous,
            offers_per_minute: generous,
            pairings_per_minute: generous,
            tickets_per_minute: generous,
        },
        yardsort_server::relay::Relay::default(),
        yardsort_server::config::DEFAULT_DEVICE_REDIRECTS
            .iter()
            .map(|r| (*r).to_owned())
            .collect(),
    )
    .await;
    let credentials = served
        .sign_in("someone", "Stand-in host", DeviceKind::Host)
        .await;
    let client = Client::new(served.base.as_str()).map_err(|e| e.to_string())?;
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        client.clone(),
        credentials,
    )));
    let shared = Shared(Arc::clone(&session));
    let host = client
        .register_host(
            &shared.now().await?,
            &HostRegistration {
                host: None,
                name: "Stand-in host".into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let here = Here::new(&host)?;
    let connector = Connector::start(
        host.id.clone(),
        Arc::new(Shared(Arc::clone(&session))),
        Arc::clone(&here) as Arc<dyn Serving>,
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !matches!(connector.status(), Status::Connected { .. }) {
        if tokio::time::Instant::now() > deadline {
            return Err(format!(
                "not connected to the relay: {:?}",
                connector.status()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    say(json!({"ready": {
        "server": served.base.as_str(),
        "host": host.id,
        "hostName": host.name,
    }}));

    let (lines, mut commands) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    while let Some(line) = commands.recv().await {
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(command) => obey(&served, &client, &shared, &host, &here, &command).await,
            Err(e) => Err(format!("not a command: {e}")),
        };
        say(match answer {
            Ok(value) => json!({ "ok": value }),
            Err(error) => json!({ "error": error }),
        });
    }
    connector.stop().await;
    here.stop();
    Ok(())
}

fn say(line: Value) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

async fn obey(
    served: &served::Served,
    client: &Client,
    shared: &Shared,
    host: &HostSummary,
    here: &Arc<Here>,
    command: &Value,
) -> Result<Value, String> {
    let fail = |e: remote_client::ClientError| e.to_string();
    let grant = || -> Result<GrantId, String> {
        command["grant"]
            .as_str()
            .and_then(|g| GrantId::new(g).ok())
            .ok_or_else(|| "name the grant".to_owned())
    };
    match command["op"].as_str().unwrap_or_default() {
        "offer" => {
            let offer = client
                .offer(&shared.now().await?, &host.id)
                .await
                .map_err(fail)?;
            let mut link = url::Url::parse("yardsort://pair").map_err(|e| e.to_string())?;
            link.query_pairs_mut()
                .append_pair("server", served.base.as_str())
                .append_pair("secret", offer.secret.expose());
            Ok(json!({
                "code": offer.code.expose(),
                "secret": offer.secret.expose(),
                "link": link.as_str(),
                "expiresInMs": offer.expires_in_ms,
            }))
        }
        "pending" => {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let page = client
                    .host_grants(&shared.now().await?, &host.id)
                    .await
                    .map_err(fail)?;
                if let Some(asked) = page
                    .grants
                    .into_iter()
                    .find(|g| g.state == GrantState::Pending)
                {
                    return serde_json::to_value(asked).map_err(|e| e.to_string());
                }
                if tokio::time::Instant::now() > deadline {
                    return Err("nobody asked".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        "approve" => {
            let grant = grant()?;
            let access = shared.now().await?;
            let asked = client.grant(&access, &grant).await.map_err(fail)?;
            let scopes: BTreeSet<Scope> = match command.get("scopes") {
                Some(scopes) => {
                    serde_json::from_value(scopes.clone()).map_err(|e| e.to_string())?
                }
                None => asked.requested.clone(),
            };
            let decision = GrantDecision {
                scopes,
                projects: ProjectGrant::All,
            };
            // Kept here first, as the app keeps what its person approved: the device may come
            // through the relay the moment the server says yes.
            here.let_in(&asked, &decision)?;
            let given = client
                .decide(&access, &grant, Some(&decision))
                .await
                .map_err(fail)?;
            serde_json::to_value(given).map_err(|e| e.to_string())
        }
        "deny" => {
            let given = client
                .decide(&shared.now().await?, &grant()?, None)
                .await
                .map_err(fail)?;
            serde_json::to_value(given).map_err(|e| e.to_string())
        }
        "revoke" => {
            client
                .revoke_grant(&shared.now().await?, &grant()?)
                .await
                .map_err(fail)?;
            Ok(Value::Null)
        }
        "run" => {
            here.run_briefly()?;
            Ok(Value::Null)
        }
        "typed" => Ok(json!(here.typed())),
        "signInAs" => {
            let subject = command["subject"]
                .as_str()
                .ok_or("name the subject")?
                .to_owned();
            served.idp.with(|s| s.subject = subject);
            Ok(Value::Null)
        }
        "removeHost" => {
            client
                .remove_host(&shared.now().await?, &host.id)
                .await
                .map_err(fail)?;
            Ok(Value::Null)
        }
        other => Err(format!("no such command: {other:?}")),
    }
}

/// The host's session, shared between the commands and the connector.
struct Shared(Arc<tokio::sync::Mutex<Session>>);

impl Shared {
    async fn now(&self) -> Result<Secret, String> {
        self.0
            .lock()
            .await
            .access(|_| Ok(()))
            .await
            .map_err(|e| e.to_string())
    }
}

impl Access for Shared {
    fn access(&self) -> AccessFuture<'_> {
        Box::pin(async move {
            let mut session = self.0.lock().await;
            let access = session
                .access(|_| Ok(()))
                .await
                .map_err(|e| AccessError::Unavailable(e.to_string()))?;
            Ok((session.client.clone(), access))
        })
    }
}

/// The host's records and terminals.
struct Here {
    _dir: tempfile::TempDir,
    store: Store,
    git: Git,
    terminals: PtyHost,
    identity: HostIdentity,
    program: SessionId,
    workspace: String,
    seen: Arc<Mutex<String>>,
}

impl Here {
    /// A throwaway workspace with one program in it, watched from here.
    fn new(host: &HostSummary) -> Result<Arc<Self>, String> {
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
        }))
    }

    /// Keep what this host approves for the device that asked.
    fn let_in(
        &self,
        asked: &remote_proto::pairing::GrantSummary,
        decision: &GrantDecision,
    ) -> Result<(), String> {
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

    /// The lines the program printed back.
    fn typed(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .split(['\r', '\n'])
            .filter_map(|l| l.find("got:").map(|at| l[at + 4..].to_owned()))
            .collect()
    }

    /// Start a short program in the workspace, and record its start and its exit as the app's
    /// launcher does.
    fn run_briefly(self: &Arc<Self>) -> Result<(), String> {
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
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(())
    }

    fn stop(&self) {
        let _ = self.terminals.kill(&self.program);
    }
}

impl Serving for Here {
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

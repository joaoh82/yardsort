//! The relay with a real host behind it: the connector the app runs, the core's records, and a
//! program in a real PTY. This is R2's gate: a phone types through the relay, a revoked phone
//! loses its live stream, and the relay failing leaves every program running and the person at
//! the host in charge of it.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::served::*;
use pty_host::{Controller, LaunchPlan, Numbered, PtyHost, SessionState};
use remote_client::{Client, Incoming, LinkReader, RelayLink};
use remote_host::{Access, AccessFuture, Connector, Serving, Status};
use remote_proto::account::{Credentials, DeviceKind, Secret};
use remote_proto::pairing::{GrantSummary, HostSummary, Offer};
use remote_proto::relay::{CloseReason, TicketRequest};
use remote_proto::{DeviceId, GrantId, HostId, OutputFrame, ProjectGrant, Scope};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use yardsort_core::git::Git;
use yardsort_core::remote::host::keep_approved;
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::store::Store;
use yardsort_server::records::MemoryRecords;

const SOON: Duration = Duration::from_secs(10);

async fn ok<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

/// A phone of `subject`'s account that `host`'s person approved, at the server, to watch and type.
async fn approved_phone(
    s: &Served,
    subject: &str,
    host: &Credentials,
    host_id: &HostId,
) -> (Credentials, GrantId) {
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let phone = s.sign_in(subject, "Phone", DeviceKind::Phone).await;
    let scopes = ["workspace:read", "terminal:read", "terminal:control"];
    let asked: GrantSummary = ok(s
        .post_as(
            &phone.access,
            "v1/pairings",
            Some(
                &json!({"offer": {"kind": "code", "value": offer.code.expose()}, "scopes": scopes}),
            ),
        )
        .await)
    .await;
    let _: GrantSummary = ok(s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({"scopes": scopes, "projects": {"kind": "all"}})),
        )
        .await)
    .await;
    (phone, asked.id)
}

/// The host's access credential, as the app's session hands it out.
struct HostAccess {
    base: String,
    access: Secret,
}

impl Access for HostAccess {
    fn access(&self) -> AccessFuture<'_> {
        Box::pin(async move { Ok((Client::new(&self.base).unwrap(), self.access.clone())) })
    }
}

/// The host's records and terminals.
struct Here {
    _dir: tempfile::TempDir,
    store: Store,
    git: Git,
    terminals: Arc<PtyHost>,
    identity: HostIdentity,
}

impl Serving for Here {
    fn with(&self, f: &mut dyn FnMut(&Context<'_>)) {
        f(&Context {
            store: &self.store,
            git: &self.git,
            terminals: self.terminals.as_ref(),
            host: &self.identity,
            interactive: true,
        });
    }
}

/// Echoes every line it reads, `got:` in front.
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

/// A phone through the relay, greeted, holding control of `terminal` and watching it.
struct Phone {
    reader: LinkReader,
    writer: remote_client::LinkWriter,
    drawn: String,
    asked: u32,
}

impl Phone {
    async fn connect(s: &Served, phone: &Credentials, host: &HostId) -> Self {
        let request = TicketRequest::Device { host: host.clone() };
        let RelayLink { reader, writer, .. } = Client::new(s.base.as_str())
            .unwrap()
            .open_relay(&phone.access, &request)
            .await
            .unwrap();
        let mut phone = Self {
            reader,
            writer,
            drawn: String::new(),
            asked: 0,
        };
        let hello = json!({"client": "test-phone/1", "protocols": [1]});
        phone.writer.text(hello.to_string()).await.unwrap();
        let welcome = phone.text().await;
        assert!(welcome.get("welcome").is_some(), "{welcome}");
        phone
    }

    /// The next text message, keeping what output frames draw on the way.
    async fn text(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(SOON, self.reader.next())
                .await
                .expect("the host said nothing")
                .unwrap_or_else(|reason| panic!("the socket closed: {reason:?}"));
            match message {
                Incoming::Text(text) => return serde_json::from_str(&text).unwrap(),
                Incoming::Binary(frame) => {
                    let frame = OutputFrame::decode(&frame).unwrap();
                    self.drawn.push_str(&String::from_utf8_lossy(frame.bytes));
                }
            }
        }
    }

    async fn ask(&mut self, op: &str, params: Value) -> Value {
        self.asked += 1;
        let id = format!("r{}", self.asked);
        let request = json!({"requestId": id, "protocol": {"major": 1, "minor": 1}, "op": op, "params": params});
        self.writer.text(request.to_string()).await.unwrap();
        loop {
            let reply = self.text().await;
            if reply["requestId"] == id {
                return reply;
            }
        }
    }

    /// Take control, watch, and type `line`; wait until the program answers it. Returns the
    /// lease.
    async fn take_and_type(&mut self, terminal: &str, line: &str) -> Value {
        let lease = self
            .ask("terminal.acquire", json!({"terminal": terminal}))
            .await;
        assert_eq!(lease["result"]["kind"], "lease", "{lease}");
        let attached = self
            .ask("terminal.attach", json!({"terminal": terminal}))
            .await;
        assert_eq!(attached["result"]["kind"], "attached", "{attached}");
        let input = json!({
            "terminal": terminal,
            "lease": lease["result"]["id"],
            "generation": lease["result"]["generation"],
            "seq": 1,
            "data": format!("{line}\r"),
        });
        let typed = self.ask("terminal.input", input).await;
        assert_eq!(typed["result"]["kind"], "done", "{typed}");
        let start = Instant::now();
        while !self.drawn.contains(&format!("got:{line}")) {
            assert!(start.elapsed() < SOON, "never drawn: {:?}", self.drawn);
            let _ = tokio::time::timeout(Duration::from_millis(200), self.text()).await;
        }
        lease["result"].clone()
    }

    /// Type input number `seq` under `lease`, and give control back straight away, without
    /// waiting to see the answer.
    async fn type_and_release(&mut self, terminal: &str, lease: &Value, seq: u64, line: &str) {
        let held = json!({
            "terminal": terminal,
            "lease": lease["id"],
            "generation": lease["generation"],
        });
        let mut input = held.clone();
        input["seq"] = seq.into();
        input["data"] = format!("{line}\r").into();
        let typed = self.ask("terminal.input", input).await;
        assert_eq!(typed["result"]["kind"], "done", "{typed}");
        let released = self.ask("terminal.release", held).await;
        assert_eq!(released["result"]["kind"], "done", "{released}");
    }

    async fn closed(&mut self) -> CloseReason {
        tokio::time::timeout(SOON, async {
            loop {
                if let Err(reason) = self.reader.next().await {
                    return reason;
                }
            }
        })
        .await
        .expect("the socket stayed open")
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < SOON, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_types_through_the_relay_loses_its_stream_when_revoked_and_a_failed_relay_leaves_the_program_running(
) {
    let s = Served::start(MemoryRecords::default()).await;
    let host = s.sign_in("alice", "Workstation", DeviceKind::Host).await;
    let summary: HostSummary = ok(s
        .post_as(
            &host.access,
            "v1/hosts",
            Some(&json!({"name": "Workstation"})),
        )
        .await)
    .await;
    let host_id = summary.id;

    // The host's own records: a workspace with a program running in it.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("alpha", "/code/alpha").unwrap();
    let workspace = store
        .add_worktree(&project.id, "work", "/code/alpha-wt", None, None)
        .unwrap();
    let here = Arc::new(Here {
        _dir: dir,
        store,
        git: yardsort_core::git::testing::git(),
        terminals: Arc::new(PtyHost::new(Arc::new(|_| {}))),
        identity: HostIdentity {
            id: host_id.clone(),
            name: "Workstation".into(),
        },
    });
    let program = here.terminals.spawn(echo_lines(&workspace.id)).unwrap();
    let terminal = program.id.0.clone();
    // What the person at the host sees. Watched rather than attached as a terminal: a terminal
    // here would be the one to answer the program's queries, and this one never answers, so on
    // Windows ConPTY would wait on its first question for ever.
    let at_host = Arc::new(Mutex::new(String::new()));
    let seen = Arc::clone(&at_host);
    here.terminals
        .attach_numbered(
            &program.id,
            Box::new(move |item| {
                if let Numbered::Snapshot { bytes, .. } | Numbered::Output { bytes, .. } = item {
                    seen.lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(bytes));
                }
                true
            }),
        )
        .unwrap();
    let let_in = |phone: &Credentials, grant: &GrantId| {
        keep_approved(
            &here.store,
            grant,
            &DeviceId::new(phone.device.to_string()).unwrap(),
            "Phone",
            &[
                Scope::WorkspaceRead,
                Scope::TerminalRead,
                Scope::TerminalControl,
            ]
            .into_iter()
            .collect(),
            &ProjectGrant::All,
            1,
        )
        .unwrap();
    };

    let connector = Connector::start(
        host_id.clone(),
        Arc::new(HostAccess {
            base: s.base.to_string(),
            access: host.access.clone(),
        }),
        Arc::clone(&here) as Arc<dyn Serving>,
    );
    until("the host is connected", || s.relay.connected(&host_id)).await;

    // A phone types through the relay; the program answers, and the person here sees it too.
    let (credentials, grant) = approved_phone(&s, "alice", &host, &host_id).await;
    let_in(&credentials, &grant);
    let mut phone = Phone::connect(&s, &credentials, &host_id).await;
    let lease = phone.take_and_type(&terminal, "from-the-phone").await;
    until("the host's own view shows it", || {
        at_host.lock().unwrap().contains("got:from-the-phone")
    })
    .await;
    // Typed and let go at once, as the pairing tool does: the line still reaches the program.
    phone
        .type_and_release(&terminal, &lease, 2, "then-let-go")
        .await;
    until(
        "the program answers the line typed before letting go",
        || at_host.lock().unwrap().contains("got:then-let-go"),
    )
    .await;

    // Its grant is revoked at the server: the live stream closes within seconds, and what it
    // controlled is let go.
    let revoked = Instant::now();
    let response = s
        .post_as(&host.access, &format!("v1/grants/{grant}/revoke"), None)
        .await;
    assert!(response.status().is_success());
    assert_eq!(phone.closed().await, CloseReason::Revoked);
    assert!(revoked.elapsed() < Duration::from_secs(5));
    let terminals = Arc::clone(&here.terminals);
    let id = program.id.clone();
    until("its control is let go", || {
        terminals.controller(&id).unwrap() == Controller::Nobody
    })
    .await;

    // Another phone takes control; then the relay fails.
    let (credentials, grant) = approved_phone(&s, "alice", &host, &host_id).await;
    let_in(&credentials, &grant);
    let mut phone = Phone::connect(&s, &credentials, &host_id).await;
    phone.take_and_type(&terminal, "before-the-failure").await;
    s.fail();
    assert_eq!(phone.closed().await, CloseReason::Restarting);

    // The program runs on, nothing holds control, and the person here types into it as before.
    until("control is let go", || {
        terminals.controller(&id).unwrap() == Controller::Nobody
    })
    .await;
    assert!(matches!(
        here.terminals.info(&program.id).unwrap().state,
        SessionState::Running
    ));
    here.terminals.write(&program.id, b"from-here\r").unwrap();
    until("the program answers the person here", || {
        at_host.lock().unwrap().contains("got:from-here")
    })
    .await;
    // And the connector says so, and keeps trying.
    until("the connector notices", || {
        matches!(
            connector.status(),
            Status::Waiting { .. } | Status::Connecting
        )
    })
    .await;
    connector.stop().await;
    let _ = here.terminals.kill(&program.id);
}

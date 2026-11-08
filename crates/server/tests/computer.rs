//! A computer as a client of another host, as the desktop is one (35 §3, "The desktop as a
//! client"): the real server and relay, a host with the app's connector and a program in a real
//! PTY, and `remote_host::computer` on the other side. It signs in as a device of its own, pairs by
//! the host's code, waits for the host's person, shows what the host shows once let in, and loses
//! it again when the host takes the grant back.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::served::*;
use pty_host::{LaunchPlan, PtyHost};
use remote_client::Client;
use remote_host::account::Failure;
use remote_host::computer::{self, Computer, ComputerHome, Link, ASKED};
use remote_host::{Access, AccessFuture, Connector, Serving};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::{GrantState, HostSummary, Offer};
use remote_proto::{DeviceId, ProjectGrant};
use serde::de::DeserializeOwned;
use serde_json::json;
use yardsort_core::remote::host::{keep_approved, CredentialStore};
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::store::Store;
use yardsort_server::records::MemoryRecords;

const SOON: Duration = Duration::from_secs(20);

async fn ok<T: DeserializeOwned>(response: reqwest::Response) -> T {
    let status = response.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < SOON, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

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
    git: yardsort_core::git::Git,
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
            starting: None,
        });
    }
}

/// A program that runs until it is stopped, and prints nothing.
fn waits(workspace: &str) -> LaunchPlan {
    let (program, args): (&str, Vec<String>) = if cfg!(windows) {
        (
            "powershell.exe",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "Start-Sleep 600".into(),
            ],
        )
    } else {
        ("/bin/sh", vec!["-c".into(), "sleep 600".into()])
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

#[derive(Default)]
struct Kept(Mutex<HashMap<String, String>>);

impl CredentialStore for Kept {
    fn get(&self, name: &str) -> Result<Option<Secret>, String> {
        Ok(self.0.lock().unwrap().get(name).cloned().map(Secret::new))
    }
    fn set(&self, name: &str, secret: &Secret) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(name.to_owned(), secret.expose().to_owned());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// The other computer: its own records, credentials and session.
struct Desk {
    computer: Computer,
    store: Store,
    kept: Arc<Kept>,
    changes: AtomicUsize,
}

struct Shared(Arc<Kept>);

impl CredentialStore for Shared {
    fn get(&self, name: &str) -> Result<Option<Secret>, String> {
        self.0.get(name)
    }
    fn set(&self, name: &str, secret: &Secret) -> Result<(), String> {
        self.0.set(name, secret)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.delete(name)
    }
}

impl ComputerHome for Desk {
    fn computer(&self) -> &Computer {
        &self.computer
    }
    fn store(&self) -> &Store {
        &self.store
    }
    fn credentials(&self) -> Result<Box<dyn CredentialStore>, Failure> {
        Ok(Box::new(Shared(Arc::clone(&self.kept))))
    }
    fn client_name(&self) -> String {
        "computer-test/1".into()
    }
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_pairs_by_code_sees_what_the_host_shows_once_let_in_and_loses_it_when_revoked() {
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

    // The host: a workspace with a program running in it.
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
    let program = here.terminals.spawn(waits(&workspace.id)).unwrap();
    let _connector = Connector::start(
        host_id.clone(),
        Arc::new(HostAccess {
            base: s.base.to_string(),
            access: host.access.clone(),
        }),
        Arc::clone(&here) as Arc<dyn Serving>,
    );
    until("the host is connected", || s.relay.connected(&host_id)).await;

    // The other computer signs in as a client, a device apart from any host.
    let desk_dir = tempfile::tempdir().unwrap();
    let desk = Arc::new(Desk {
        computer: Computer::default(),
        store: Store::open(&desk_dir.path().join("yardsort.db")).unwrap(),
        kept: Arc::default(),
        changes: AtomicUsize::new(0),
    });
    let signed_in = s.sign_in("alice", "Laptop", DeviceKind::Computer).await;
    let device = signed_in.device.clone();
    let client = Client::new(s.base.as_str()).unwrap();
    let server = client.describe().await.unwrap();
    computer::sign_in(&desk, client, &server, "Laptop", signed_in)
        .await
        .unwrap();
    assert_eq!(
        desk.store.remote_computer().unwrap().unwrap().device_id,
        device.to_string()
    );
    assert!(desk
        .kept
        .get(&computer::refresh_entry(device.as_str()))
        .unwrap()
        .is_some());

    // A wrong code says so; the host's code asks, and waits for its person.
    let wrong = computer::pair(&desk, "AAAA-AAAA").await.unwrap_err();
    assert_eq!(wrong.code, "remote_code");
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let asked = computer::pair(&desk, offer.code.expose()).await.unwrap();
    assert_eq!(asked.state, GrantState::Pending);
    assert_eq!(asked.requested, ASKED.into_iter().collect());
    let (paired, problem) = computer::paired(&desk);
    assert!(problem.is_none(), "{problem:?}");
    assert_eq!(paired.len(), 1);
    assert_eq!(paired[0].grant.host_name, "Workstation");
    assert!(
        paired[0].view.is_none(),
        "nothing is reached before it is let in"
    );

    // The host's person lets it watch.
    let _: serde_json::Value = ok(s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/approve", asked.id),
            Some(&json!({"scopes": ASKED, "projects": {"kind": "all"}})),
        )
        .await)
    .await;
    keep_approved(
        &here.store,
        &asked.id,
        &DeviceId::new(device.to_string()).unwrap(),
        "Laptop",
        &ASKED.into_iter().collect(),
        &ProjectGrant::All,
        1,
    )
    .unwrap();
    // Nothing tells the computer of the answer: it asks again by itself while it waits.
    until("the host is reached and read", || {
        computer::view(desk.as_ref(), host_id.as_str()).is_some_and(|v| {
            v.link == Link::Online
                && v.shown.as_ref().is_some_and(|shown| {
                    shown.workspaces.iter().any(|(w, sessions)| {
                        w.name == "work"
                            && sessions
                                .terminals
                                .iter()
                                .any(|t| t.terminal.id.as_str() == program.id.0)
                    })
                })
        })
    })
    .await;
    let shown = computer::view(desk.as_ref(), host_id.as_str())
        .unwrap()
        .shown
        .unwrap();
    assert_eq!(shown.host.name, "Workstation");
    assert_eq!(shown.projects[0].name, "alpha");
    assert!(
        desk.changes.load(Ordering::SeqCst) > 0,
        "the window was told"
    );

    // The host takes it back: the computer reaches it no more, and the list says why.
    let revoked = s
        .post_as(
            &host.access,
            &format!("v1/grants/{}/revoke", asked.id),
            None,
        )
        .await;
    assert!(revoked.status().is_success(), "{}", revoked.status());
    until("the host is no longer reached", || {
        computer::view(desk.as_ref(), host_id.as_str()).is_none_or(|v| v.link != Link::Online)
    })
    .await;
    computer::refresh(&desk).await.unwrap();
    let (paired, _) = computer::paired(&desk);
    assert_eq!(paired[0].grant.state, GrantState::Revoked);
    assert!(computer::view(desk.as_ref(), host_id.as_str()).is_none());
    // The program on the host never noticed.
    assert!(here
        .terminals
        .list()
        .iter()
        .any(|t| t.id == program.id && t.state == pty_host::SessionState::Running));

    // Forgotten here: the server still lists what it took back, but it shows no more.
    computer::forget(&desk, asked.id.as_str()).await.unwrap();
    assert!(computer::paired(&desk).0.is_empty());
    computer::refresh(&desk).await.unwrap();
    assert!(computer::paired(&desk).0.is_empty(), "listed again");

    // Signing out forgets it here and at the server.
    computer::sign_out(desk.as_ref()).await.unwrap();
    assert!(desk.store.remote_computer().unwrap().is_none());
    assert!(desk
        .kept
        .get(&computer::refresh_entry(device.as_str()))
        .unwrap()
        .is_none());
}

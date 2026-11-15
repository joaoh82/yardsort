//! A computer as a client of another host, as the desktop is one (35 §3, "The desktop as a
//! client"): the real server and relay, a host with the app's connector and a program in a real
//! PTY, and `remote_host::computer` on the other side. It signs in as a device of its own, pairs by
//! the host's code, waits for the host's person, shows what the host shows once let in, and loses
//! it again when the host takes the grant back. A terminal there is watched, typed into once this
//! computer takes control, and watched again after the link drops. Work is started there, and an
//! ended conversation resumed, by the host's ids.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::served::*;
use pty_host::{LaunchPlan, PtyHost};
use remote_client::Client;
use remote_host::account::Failure;
use remote_host::computer::start;
use remote_host::computer::terminal::{self, Seen};
use remote_host::computer::{self, Computer, ComputerHome, Link, ASKED};
use remote_host::{Access, AccessFuture, Connector, Serving};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::{GrantState, HostSummary, Offer};
use remote_proto::{Controller, DeviceId, ProjectGrant, TermSize};
use serde::de::DeserializeOwned;
use serde_json::json;
use yardsort_core::error::{IpcError, IpcResult};
use yardsort_core::harness::{HarnessDef, HarnessOverride, SessionIdMode};
use yardsort_core::remote::host::{keep_approved, CredentialStore};
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::start::Starting;
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

/// The host's records and terminals, and what it starts agents with, if anything.
struct Here {
    _dir: tempfile::TempDir,
    store: Store,
    git: yardsort_core::git::Git,
    terminals: Arc<PtyHost>,
    identity: HostIdentity,
    starting: Option<Arc<dyn Starting>>,
}

impl Serving for Here {
    fn with(&self, f: &mut dyn FnMut(&Context<'_>)) {
        f(&Context {
            store: &self.store,
            git: &self.git,
            terminals: self.terminals.as_ref(),
            host: &self.identity,
            interactive: true,
            starting: self.starting.clone(),
            workflow: None,
            conversations: None,
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
        starting: None,
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

/// A program that prints back what is typed into it.
fn echoes(workspace: &str) -> LaunchPlan {
    let (program, args): (&str, Vec<String>) = if cfg!(windows) {
        ("cmd.exe", vec!["/Q".into()])
    } else {
        ("/bin/cat", vec![])
    };
    LaunchPlan {
        program: program.into(),
        args,
        ..waits(workspace)
    }
}

/// What a watched terminal shows next, within [`SOON`].
async fn seen(watching: &mut terminal::Watching) -> Seen {
    tokio::time::timeout(SOON, watching.next())
        .await
        .expect("something seen in time")
        .expect("still watched")
}

/// Everything seen until `done` says so, the output joined.
async fn seen_until(
    watching: &mut terminal::Watching,
    what: &str,
    mut done: impl FnMut(&Seen, &str) -> bool,
) -> String {
    let mut output = String::new();
    loop {
        let item = tokio::time::timeout(SOON, watching.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {output:?}"))
            .unwrap_or_else(|| panic!("gone before {what}"));
        if let Seen::Output(bytes) = &item {
            output.push_str(&String::from_utf8_lossy(bytes));
        }
        if done(&item, &output) {
            return output;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_watches_a_terminal_types_once_it_takes_control_and_loses_it_to_the_host() {
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
        starting: None,
    });
    let program = here.terminals.spawn(echoes(&workspace.id)).unwrap();
    let start_connector = || {
        Connector::start(
            host_id.clone(),
            Arc::new(HostAccess {
                base: s.base.to_string(),
                access: host.access.clone(),
            }),
            Arc::clone(&here) as Arc<dyn Serving>,
        )
    };
    let connector = start_connector();
    until("the host is connected", || s.relay.connected(&host_id)).await;

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
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let asked = computer::pair(&desk, offer.code.expose()).await.unwrap();
    // Let in to watch and to type: all it asked for.
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
    until("the host is reached", || {
        computer::view(desk.as_ref(), host_id.as_str()).is_some_and(|v| v.link == Link::Online)
    })
    .await;

    // Watching starts from a snapshot at the host's size, and takes nothing.
    let mut watching = terminal::watch(&desk, host_id.as_str(), &program.id.0).unwrap();
    assert_eq!(
        seen(&mut watching).await,
        Seen::Reset {
            size: TermSize { cols: 80, rows: 24 }
        }
    );
    assert_eq!(
        seen(&mut watching).await,
        Seen::Controller(Controller::Nobody)
    );
    // Typed while watching only: it goes nowhere.
    watching.asker().input("unsent\r".into());

    // Taking control is a press; then typing reaches the program, and the view's size stands.
    watching.asker().take_control();
    seen_until(&mut watching, "control", |item, _| item == &Seen::Held).await;
    watching.asker().input("hello there\r".into());
    let output = seen_until(&mut watching, "the echo", |_, output| {
        output.contains("hello there")
    })
    .await;
    assert!(!output.contains("unsent"), "{output:?}");
    watching.asker().resize(TermSize {
        cols: 100,
        rows: 30,
    });
    seen_until(&mut watching, "the new size", |item, _| {
        item == &Seen::Resized(TermSize {
            cols: 100,
            rows: 30,
        })
    })
    .await;

    // The person at the host types: control is theirs, and this computer is told so.
    here.terminals.write(&program.id, b"\r").unwrap();
    seen_until(&mut watching, "control lost", |item, _| {
        matches!(item, Seen::NotHeld(Some(why)) if why == "The person at that computer took control.")
    })
    .await;

    // The link drops and comes back: watching resumes where it was, with no new snapshot.
    drop(connector);
    seen_until(&mut watching, "the drop", |item, _| {
        matches!(item, Seen::Away(_))
    })
    .await;
    let _connector = start_connector();
    let after = seen_until(&mut watching, "watching again", |item, _| {
        matches!(item, Seen::Reset { .. } | Seen::Resized(_))
    })
    .await;
    assert!(after.is_empty(), "nothing drawn twice: {after:?}");
    here.terminals.write(&program.id, b"after\r").unwrap();
    let mut reset = false;
    seen_until(&mut watching, "output after the drop", |item, output| {
        reset |= matches!(item, Seen::Reset { .. });
        output.contains("after")
    })
    .await;
    assert!(!reset, "it resumed rather than starting over");

    // The program never noticed any of it.
    assert!(here
        .terminals
        .list()
        .iter()
        .any(|t| t.id == program.id && t.state == pty_host::SessionState::Running));
}

/// What the host starts agents with: a program that waits, in its one workspace, and a note of
/// what it was asked. Its one harness can resume, so an ended conversation of it is resumable.
struct Stand {
    terminals: Arc<PtyHost>,
    workspace: String,
    asked: Mutex<Vec<String>>,
    /// The project a clone is added as.
    cloned: String,
    /// Whether its workspace has uncommitted work, which archiving or deleting would lose.
    dirty: std::sync::atomic::AtomicBool,
}

impl Stand {
    fn note(&self, asked: String) {
        self.asked.lock().unwrap().push(asked);
    }

    fn unless_dirty(&self, force: bool) -> IpcResult<()> {
        if self.dirty.load(std::sync::atomic::Ordering::SeqCst) && !force {
            // As the core says it, with a folder in it that must not reach the other computer.
            return Err(IpcError::new(
                "worktree_dirty",
                "/home/someone/worktrees/w has uncommitted changes.",
            ));
        }
        Ok(())
    }

    fn run(&self, asked: String) -> IpcResult<pty_host::SessionInfo> {
        self.asked.lock().unwrap().push(asked);
        self.terminals
            .spawn(waits(&self.workspace))
            .map_err(|e| IpcError::new("spawn", e.to_string()))
    }
}

impl Starting for Stand {
    fn harnesses(&self) -> Vec<remote_proto::HarnessSummary> {
        vec![remote_proto::HarnessSummary {
            id: "stand-in".into(),
            name: "Stand-in".into(),
            models: vec!["large".into()],
            efforts: vec!["high".into()],
        }]
    }
    fn workspace(
        &self,
        new: &yardsort_core::remote::start::NewWorkspace,
    ) -> IpcResult<(String, pty_host::SessionInfo)> {
        let said = [
            &new.name,
            &new.model,
            &new.effort,
            &new.base_branch,
            &new.existing_branch,
        ]
        .map(|part| part.as_deref().unwrap_or("-"))
        .join(" ");
        let extra = if said == "- - - - -" {
            String::new()
        } else {
            format!(" [{said}]")
        };
        let info = self.run(format!(
            "workspace {} {} {}{extra}",
            new.project, new.harness, new.prompt
        ))?;
        Ok((self.workspace.clone(), info))
    }
    fn shell(&self, workspace: &str) -> IpcResult<pty_host::SessionInfo> {
        self.run(format!("shell {workspace}"))
    }
    fn branches(&self, project: &str) -> IpcResult<remote_proto::ProjectBranches> {
        self.note(format!("branches {project}"));
        Ok(remote_proto::manage::bounded_branches(
            vec!["main".into(), "develop".into()],
            Some("main".into()),
            vec!["main".into()],
        ))
    }
    fn clone_project(&self, remote: &str, name: Option<&str>) -> IpcResult<(String, String)> {
        self.note(format!("clone {remote} {name:?}"));
        if remote.contains("private") {
            return Err(IpcError::new(
                "clone_failed",
                "Could not clone /secret/path",
            ));
        }
        Ok((self.cloned.clone(), "cloned".into()))
    }
    fn rename(&self, workspace: &str, name: &str) -> IpcResult<()> {
        self.note(format!("rename {workspace} {name}"));
        Ok(())
    }
    fn archive(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.note(format!("archive {workspace} {force}"));
        self.unless_dirty(force)
    }
    fn delete(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.note(format!("delete {workspace} {force}"));
        self.unless_dirty(force)
    }
    fn changes(&self, workspace: &str) -> IpcResult<yardsort_core::changes::ChangeSet> {
        self.note(format!("changes {workspace}"));
        Ok(yardsort_core::changes::ChangeSet {
            uncommitted: vec![yardsort_core::changes::FileChange {
                path: "src/big.txt".into(),
                old_path: None,
                kind: yardsort_core::changes::ChangeKind::Modified,
                additions: Some(3),
                deletions: Some(1),
            }],
            committed: vec![],
            base: Some("main".into()),
        })
    }
    fn files(
        &self,
        workspace: &str,
        dir: &str,
    ) -> IpcResult<Vec<yardsort_core::file_tree::FileEntry>> {
        self.note(format!("files {workspace} {dir:?}"));
        Ok(vec![yardsort_core::file_tree::FileEntry {
            name: "big.txt".into(),
            path: "src/big.txt".into(),
            is_dir: false,
            ignored: false,
        }])
    }
    fn content(
        &self,
        _workspace: &str,
        path: &str,
        _old_path: Option<&str>,
        of: remote_proto::FileOf,
    ) -> IpcResult<yardsort_core::changes::Content> {
        use yardsort_core::changes::Content;
        Ok(match (path, of) {
            // Larger than one piece carries, and with a character that a piece may split.
            ("src/big.txt", remote_proto::FileOf::Working) => Content::Text {
                text: "é".repeat(40_000),
            },
            ("src/big.txt", _) => Content::Absent,
            ("logo.png", _) => Content::Binary,
            ("huge.log", _) => Content::TooLarge { bytes: 5_000_000 },
            _ => return Err(IpcError::new("bad_path", "/home/someone/w is outside")),
        })
    }
    fn publish(
        &self,
        workspace: &str,
        forge: bool,
    ) -> IpcResult<remote_proto::publish::PublishState> {
        self.note(format!("publish {workspace} {forge}"));
        Ok(remote_proto::publish::PublishState {
            branch: Some("ys/tidy".into()),
            base: Some("main".into()),
            remote: true,
            ahead: 1,
            unpushed: vec!["Tidy it".into()],
            can_open: true,
            gh: true,
            ..Default::default()
        })
    }
    fn commit(
        &self,
        workspace: &str,
        message: &str,
    ) -> IpcResult<remote_proto::publish::PublishState> {
        self.note(format!("commit {workspace} {message}"));
        self.publish(workspace, false)
    }
    fn push(&self, workspace: &str) -> IpcResult<remote_proto::publish::PublishState> {
        self.note(format!("push {workspace}"));
        // As the core says a refused push: in its own words, with nothing of Git's.
        Err(IpcError::new(
            "push_rejected",
            "The remote has commits this branch does not have, so it refused the push.",
        ))
    }
    fn open_pull_request(
        &self,
        workspace: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> IpcResult<remote_proto::publish::PullRequestOpened> {
        self.note(format!("open {workspace} {title} {body:?} {draft}"));
        Ok(remote_proto::publish::PullRequestOpened {
            url: "https://forge.example/acme/app/pull/7".into(),
            created: true,
        })
    }
    fn merge_pull_request(
        &self,
        workspace: &str,
        number: u32,
        head_oid: &str,
        method: remote_proto::publish::MergeMethod,
    ) -> IpcResult<()> {
        self.note(format!("merge {workspace} {number} {head_oid} {method:?}"));
        Ok(())
    }
    fn session(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
    ) -> IpcResult<pty_host::SessionInfo> {
        self.run(format!("session {workspace} {harness} {prompt:?}"))
    }
    fn resume(&self, session: &str, prompt: Option<&str>) -> IpcResult<pty_host::SessionInfo> {
        self.run(format!("resume {session} {prompt:?}"))
    }
    fn definitions(&self) -> Vec<HarnessDef> {
        let program = if cfg!(windows) { "cmd.exe" } else { "sh" };
        yardsort_core::harness::resolve_all(&[HarnessOverride {
            id: "stand-in".into(),
            builtin: Some(false),
            command: Some(program.into()),
            resume_args: Some(vec!["{session_id}".into()]),
            session_id_mode: Some(SessionIdMode::Assigned),
            ..Default::default()
        }])
        .into_iter()
        .map(|h| h.def)
        .filter(|def| def.id == "stand-in")
        .collect()
    }
    fn data_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::new()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_starts_agents_there_and_resumes_an_ended_one_by_that_hosts_ids() {
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
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("alpha", "/code/alpha").unwrap();
    let workspace = store
        .add_worktree(&project.id, "work", "/code/alpha-wt", None, None)
        .unwrap();
    // A conversation that ended, which the host's harness can take up again.
    store
        .add_session(&yardsort_core::store::NewSession {
            id: "an-ended-one",
            workspace_id: &workspace.id,
            harness_id: "stand-in",
            harness_session_id: Some("its-own-id"),
            title: "Write the migration",
            pty_session_id: "long-gone",
            ..Default::default()
        })
        .unwrap();
    store.end_session_by_pty("long-gone", Some(0)).unwrap();
    let terminals = Arc::new(PtyHost::new(Arc::new(|_| {})));
    let stand = Arc::new(Stand {
        terminals: Arc::clone(&terminals),
        workspace: workspace.id.clone(),
        asked: Mutex::default(),
        cloned: project.id.clone(),
        dirty: std::sync::atomic::AtomicBool::new(false),
    });
    let here = Arc::new(Here {
        _dir: dir,
        store,
        git: yardsort_core::git::testing::git(),
        terminals,
        identity: HostIdentity {
            id: host_id.clone(),
            name: "Workstation".into(),
        },
        starting: Some(Arc::clone(&stand) as Arc<dyn Starting>),
    });
    let _connector = Connector::start(
        host_id.clone(),
        Arc::new(HostAccess {
            base: s.base.to_string(),
            access: host.access.clone(),
        }),
        Arc::clone(&here) as Arc<dyn Serving>,
    );
    until("the host is connected", || s.relay.connected(&host_id)).await;

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
    let offer: Offer = ok(s
        .post_as(&host.access, &format!("v1/hosts/{host_id}/offers"), None)
        .await)
    .await;
    let asked = computer::pair(&desk, offer.code.expose()).await.unwrap();
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
    let host = host_id.as_str();
    let resumable = || {
        computer::view(desk.as_ref(), host).is_some_and(|v| {
            v.shown.as_ref().is_some_and(|shown| {
                shown.workspaces.iter().any(|(_, page)| {
                    page.sessions
                        .iter()
                        .any(|s| s.id.as_str() == "an-ended-one" && s.resumable)
                })
            })
        })
    };
    until("the ended conversation shows as one to resume", resumable).await;

    let agents = start::harnesses(&desk, host).await.unwrap();
    assert_eq!(agents[0].id, "stand-in");
    let made = start::workspace(
        &desk,
        host,
        &project.id,
        "Fix the spacing",
        start::Said {
            harness: Some("stand-in".into()),
            ..start::Said::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(made.workspace.as_str(), workspace.id);
    assert_eq!(made.harness, "stand-in");
    let another = start::session(&desk, host, &workspace.id, None, Some("  "))
        .await
        .unwrap();
    assert_ne!(another.terminal, made.terminal);
    let resumed = start::resume(&desk, host, "an-ended-one", Some("Carry on"))
        .await
        .unwrap();
    assert_eq!(resumed.workspace.as_str(), workspace.id);
    assert_eq!(resumed.harness, "stand-in");
    assert_eq!(
        *stand.asked.lock().unwrap(),
        [
            format!("workspace {} stand-in Fix the spacing", project.id),
            format!("session {} stand-in None", workspace.id),
            "resume an-ended-one Some(\"Carry on\")".to_owned(),
        ]
    );
    // What the host has no record of is refused there, and nothing is started for it.
    let refused = start::resume(&desk, host, "not-one-of-its", None)
        .await
        .unwrap_err();
    assert_eq!(refused.code, "remote_refused");
    assert_eq!(stand.asked.lock().unwrap().len(), 3);

    // Since minor 10: what a start says beyond its message reaches the host, each part as said.
    let said = |effort: &str| start::Said {
        harness: Some("stand-in".into()),
        name: Some("tidy".into()),
        model: Some("large".into()),
        effort: Some(effort.into()),
        base_branch: Some("develop".into()),
        existing_branch: None,
    };
    start::workspace(&desk, host, &project.id, "Tidy it", said("high"))
        .await
        .unwrap();
    // An effort that agent does not have there is refused, and nothing is made for it.
    let odd = start::workspace(&desk, host, &project.id, "Tidy it", said("ludicrous"))
        .await
        .unwrap_err();
    assert_eq!(odd.code, "remote_refused");
    let branches = start::branches(&desk, host, &project.id).await.unwrap();
    assert_eq!(branches.branches, ["main", "develop"]);
    assert_eq!(branches.default.as_deref(), Some("main"));
    assert_eq!(branches.checked_out, ["main"]);
    let shell = start::shell(&desk, host, &workspace.id).await.unwrap();
    assert_eq!(shell.workspace.as_str(), workspace.id);
    assert_eq!(shell.harness, "", "a shell is no harness");
    start::rename(&desk, host, &workspace.id, " Login spacing ")
        .await
        .unwrap();
    // Uncommitted work there: archiving and deleting are refused until asked a second time, and
    // the host's own words, which name its folder, do not come over.
    stand.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    for delete in [false, true] {
        let dirty = start::remove(&desk, host, &workspace.id, delete, false)
            .await
            .unwrap_err();
        assert_eq!(dirty.code, "remote_uncommitted");
        assert!(!dirty.message.contains("/home"), "{}", dirty.message);
        start::remove(&desk, host, &workspace.id, delete, true)
            .await
            .unwrap();
    }
    // A clone is asked for by its address, and the project it was added as comes back. One the
    // host cannot clone says so without its folders.
    let cloned = start::clone(&desk, host, " https://example.com/acme/app.git ")
        .await
        .unwrap();
    assert_eq!(cloned.project.as_str(), project.id);
    assert_eq!(cloned.name, "cloned");
    let failed = start::clone(&desk, host, "https://example.com/acme/private.git")
        .await
        .unwrap_err();
    assert_eq!(failed.code, "remote_refused");
    assert!(!failed.message.contains("/secret"), "{}", failed.message);
    assert_eq!(
        stand.asked.lock().unwrap()[3..],
        [
            format!(
                "workspace {} stand-in Tidy it [tidy large high develop -]",
                project.id
            ),
            format!("branches {}", project.id),
            format!("shell {}", workspace.id),
            format!("rename {} Login spacing", workspace.id),
            format!("archive {} false", workspace.id),
            format!("archive {} true", workspace.id),
            format!("delete {} false", workspace.id),
            format!("delete {} true", workspace.id),
            "clone https://example.com/acme/app.git None".to_owned(),
            "clone https://example.com/acme/private.git None".to_owned(),
        ]
    );
    // Since minor 11: what changed there, its tree, and a file put together from its pieces.
    use remote_host::computer::look;
    let changed = look::changes(&desk, host, &workspace.id).await.unwrap();
    assert_eq!(changed.uncommitted[0].path, "src/big.txt");
    assert_eq!(changed.base.as_deref(), Some("main"));
    let tree = look::files(&desk, host, &workspace.id, "src")
        .await
        .unwrap();
    assert_eq!(tree.entries[0].name, "big.txt");
    let read = |path: &'static str, of| look::file(&desk, host, &workspace.id, path, None, of);
    let big = read("src/big.txt", remote_proto::FileOf::Working)
        .await
        .unwrap();
    assert_eq!(big.state, remote_proto::FileState::Text);
    // 80 000 bytes: three pieces, the first two ending inside a character.
    assert_eq!(String::from_utf8(big.bytes).unwrap(), "é".repeat(40_000));
    let before = read("src/big.txt", remote_proto::FileOf::UncommittedOld)
        .await
        .unwrap();
    assert_eq!(before.state, remote_proto::FileState::Absent);
    let picture = read("logo.png", remote_proto::FileOf::Working)
        .await
        .unwrap();
    assert_eq!(picture.state, remote_proto::FileState::Binary);
    assert!(picture.bytes.is_empty());
    let huge = read("huge.log", remote_proto::FileOf::Working)
        .await
        .unwrap();
    assert_eq!(
        (huge.state, huge.size),
        (remote_proto::FileState::TooLarge, 5_000_000)
    );
    // A path the host refuses is refused in its own words, without its folders; one that climbs
    // out is refused by this computer's client, and never sent.
    let climbing = read("../up.txt", remote_proto::FileOf::Working)
        .await
        .unwrap_err();
    assert_eq!(climbing.code, "remote_failed");
    let outside = read("elsewhere.txt", remote_proto::FileOf::Working)
        .await
        .unwrap_err();
    assert_eq!(outside.code, "remote_refused");
    assert!(!outside.message.contains("/home"), "{}", outside.message);
    assert!(read("../up.txt", remote_proto::FileOf::Working)
        .await
        .is_err());
    // Since minor 12: commit, push and the pull request, each done by the host as itself.
    use remote_host::computer::publish;
    let asked_before = stand.asked.lock().unwrap().len();
    let stands = publish::state(&desk, host, &workspace.id, true)
        .await
        .unwrap();
    assert_eq!(stands.branch.as_deref(), Some("ys/tidy"));
    assert!(stands.can_open && stands.gh);
    let committed = publish::commit(&desk, host, &workspace.id, "  Tidy it\n\nA body.  ")
        .await
        .unwrap();
    assert_eq!(committed.unpushed, ["Tidy it"]);
    // A push the remote refuses is said in the host's own words.
    let refused = publish::push(&desk, host, &workspace.id).await.unwrap_err();
    assert_eq!(refused.code, "remote_refused");
    assert!(
        refused.message.contains("refused the push"),
        "{}",
        refused.message
    );
    let opened = publish::open_pull_request(&desk, host, &workspace.id, " Tidy it ", "Why.", true)
        .await
        .unwrap();
    assert!(opened.created);
    assert_eq!(opened.url, "https://forge.example/acme/app/pull/7");
    publish::merge_pull_request(
        &desk,
        host,
        &workspace.id,
        7,
        "0123456789abcdef",
        remote_proto::publish::MergeMethod::Rebase,
    )
    .await
    .unwrap();
    // What no host would take is refused by this computer's client, and never sent.
    assert!(publish::commit(&desk, host, &workspace.id, "  ")
        .await
        .is_err());
    assert!(publish::merge_pull_request(
        &desk,
        host,
        &workspace.id,
        7,
        "HEAD",
        remote_proto::publish::MergeMethod::Squash
    )
    .await
    .is_err());
    assert_eq!(
        stand.asked.lock().unwrap()[asked_before..],
        [
            format!("publish {} true", workspace.id),
            format!("commit {} Tidy it\n\nA body.", workspace.id),
            format!("publish {} false", workspace.id),
            format!("push {}", workspace.id),
            format!("open {} Tidy it \"Why.\" true", workspace.id),
            format!("merge {} 7 0123456789abcdef Rebase", workspace.id),
        ]
    );
    for terminal in here.terminals.list() {
        let _ = here.terminals.kill(&terminal.id);
    }
}

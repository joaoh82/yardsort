//! Starting agents and uploading files on a device's channel, against a real repository, a real
//! agent in a real PTY, and real files.

use std::path::{Path, PathBuf};

use crate::env::{EnvSource, ShellEnv};
use crate::harness::{HarnessOverride, SessionIdMode};
use crate::remote::start::{Starter, Starting};
use crate::settings::Settings;

use super::*;

/// A harness that is really the shell, so an agent can be started on any machine. Its first
/// message is the script it runs; resumed, it says which conversation it was.
fn shell_harness() -> HarnessOverride {
    let (command, flag) = if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("sh", "-c")
    };
    HarnessOverride {
        id: "sh-agent".into(),
        label: Some("Shell agent".into()),
        builtin: Some(false),
        command: Some(command.into()),
        prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
        // It lingers after saying so: a resume settles its record at once should the program
        // have exited already, which on a busy machine an instant `echo` sometimes has.
        resume_args: Some(vec![
            flag.into(),
            if cfg!(windows) {
                "echo resumed-{session_id} & ping -n 3 127.0.0.1 >nul".into()
            } else {
                "echo resumed-{session_id}; sleep 2".into()
            },
        ]),
        session_id_mode: Some(SessionIdMode::Assigned),
        ..Default::default()
    }
}

/// What the app lends a channel, made of this test's own store, terminals and folders.
struct Lent {
    store: Arc<Store>,
    host: Arc<PtyHost>,
    env: Arc<ShellEnv>,
    settings: Settings,
    worktrees: PathBuf,
    data: PathBuf,
}

impl Lent {
    fn with<T>(&self, f: impl FnOnce(&Starter<'_>) -> T) -> T {
        f(&Starter {
            store: &self.store,
            git: &git(),
            terminals: self.host.as_ref(),
            env: &self.env,
            settings: &self.settings,
            worktree_root: &self.worktrees,
            data_dir: &self.data,
        })
    }
}

impl Starting for Lent {
    fn harnesses(&self) -> Vec<remote_proto::HarnessSummary> {
        self.with(|s| s.harnesses())
    }
    fn workspace(
        &self,
        project: &str,
        prompt: &str,
        harness: &str,
    ) -> crate::error::IpcResult<(String, SessionInfo)> {
        self.with(|s| s.workspace(project, prompt, harness))
    }
    fn session(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
    ) -> crate::error::IpcResult<SessionInfo> {
        self.with(|s| s.session(workspace, harness, prompt))
    }
    fn resume(&self, session: &str, prompt: Option<&str>) -> crate::error::IpcResult<SessionInfo> {
        self.with(|s| s.resume(session, prompt))
    }
    fn definitions(&self) -> Vec<crate::harness::HarnessDef> {
        self.with(|s| s.definitions())
    }
    fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }
}

/// A real repository as a project, another project the phone is not let into, and a phone.
struct Started {
    lent: Arc<Lent>,
    git: Git,
    identity: HostIdentity,
    project: String,
    elsewhere: String,
    _dirs: Vec<tempfile::TempDir>,
}

impl Started {
    fn new() -> Self {
        let git = git();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        git.run(repo.path(), &["checkout", "-b", "trunk"]).unwrap();
        git.initial_commit(repo.path()).unwrap();
        let worktrees = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let project = store
            .add_project("alpha", &repo.path().to_string_lossy())
            .unwrap()
            .id;
        let elsewhere = store.add_project("beta", "/code/beta").unwrap().id;
        // Built-ins are left out, so that what is found here is the same on every machine.
        let builtins_off = crate::harness::resolve_all(&[])
            .into_iter()
            .map(|h| HarnessOverride {
                id: h.def.id,
                enabled: Some(false),
                ..Default::default()
            });
        let settings = Settings {
            harnesses: std::iter::once(shell_harness())
                .chain(builtins_off)
                .collect(),
            ..Default::default()
        };
        Self {
            lent: Arc::new(Lent {
                store: Arc::new(store),
                host: Arc::new(PtyHost::new(Arc::new(|_| {}))),
                env: Arc::new(ShellEnv {
                    vars: std::env::vars().collect(),
                    source: EnvSource::Process,
                    warning: None,
                }),
                settings,
                worktrees: worktrees.path().to_owned(),
                data: data.path().to_owned(),
            }),
            git,
            identity: HostIdentity {
                id: HostId::new("host-test").unwrap(),
                name: "Test host".into(),
            },
            project,
            elsewhere,
            _dirs: vec![repo, worktrees, data],
        }
    }

    fn approve(&self, scopes: &[Scope]) {
        host::keep_approved(
            &self.lent.store,
            &GrantId::new("g-phone").unwrap(),
            &DeviceId::new(PHONE).unwrap(),
            "Phone",
            &scopes.iter().copied().collect::<BTreeSet<_>>(),
            &ProjectGrant::Only(BTreeSet::from([ProjectId::new(&self.project).unwrap()])),
            1,
        )
        .unwrap();
    }

    fn context(&self, starting: bool) -> Context<'_> {
        Context {
            store: &self.lent.store,
            git: &self.git,
            terminals: self.lent.host.as_ref(),
            host: &self.identity,
            interactive: true,
            starting: starting.then(|| Arc::clone(&self.lent) as Arc<dyn Starting>),
        }
    }

    fn visitor(&self, out: &Arc<Recorded>) -> Visitor {
        let visitor = Visitor::open(
            &self.lent.store,
            DeviceId::new(PHONE).unwrap(),
            GrantId::new("g-phone").unwrap(),
            Arc::clone(out) as Arc<dyn Outbound>,
        )
        .unwrap();
        let hello = json!({"client": "test-phone/1", "protocols": [1]}).to_string();
        visitor.handle(&self.context(true), &hello).unwrap();
        visitor
    }

    /// Send one request, and wait for its reply: a start's comes later, from its own thread.
    fn ask(&self, visitor: &Visitor, out: &Recorded, id: &str, op: &str, params: Value) -> Value {
        let request = json!({
            "requestId": id,
            "protocol": ProtocolVersion::CURRENT,
            "op": op,
            "params": params,
        });
        visitor
            .handle(&self.context(true), &request.to_string())
            .unwrap();
        let reply = |out: &Recorded| out.texts().into_iter().find(|t| t["requestId"] == id);
        out.wait_for(op, |out| reply(out).is_some());
        reply(out).unwrap()
    }

    fn workspaces(&self) -> Vec<crate::store::WorkspaceRow> {
        self.lent
            .store
            .workspaces()
            .unwrap()
            .into_iter()
            .filter(|w| w.project_id == self.project && w.kind == "worktree")
            .collect()
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        for session in self.lent.host.list() {
            let _ = self.lent.host.kill(&session.id);
        }
    }
}

/// What a device with control and nothing more is let in to.
const CONTROL: [Scope; 3] = [
    Scope::WorkspaceRead,
    Scope::TerminalRead,
    Scope::TerminalControl,
];

#[test]
fn a_phone_with_control_starts_a_workspace_and_its_agent_as_the_window_would() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);

    let welcome = &out.texts()[0]["welcome"]["host"]["capabilities"];
    assert!(welcome
        .as_array()
        .unwrap()
        .contains(&json!("workspace.start")));
    let listed = fx.ask(&visitor, &out, "r1", "harness.list", json!(null));
    assert_eq!(
        listed["result"]["harnesses"],
        json!([{"id": "sh-agent", "name": "Shell agent"}])
    );

    let reply = fx.ask(
        &visitor,
        &out,
        "r2",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo hello-from-the-phone"}),
    );
    let started = &reply["result"];
    assert_eq!(started["kind"], "started", "{reply}");
    assert_eq!(started["harness"], "sh-agent");

    // A branch and a worktree, named after the message, as the composer makes them.
    let made = fx.workspaces();
    assert_eq!(made.len(), 1);
    assert_eq!(started["workspace"], made[0].id);
    assert!(Path::new(&made[0].path).is_dir());
    assert!(made[0].branch.as_deref().unwrap().contains("echo-hello"));
    // The agent runs in it, recorded as a conversation, and was given the message.
    let terminal = SessionId(started["terminal"].as_str().unwrap().to_owned());
    let info = fx.lent.host.info(&terminal).unwrap();
    assert_eq!(info.labels[WORKSPACE_LABEL], made[0].id);
    assert_eq!(
        fx.lent.store.session_prompts(&made[0].id).unwrap(),
        ["echo hello-from-the-phone"]
    );
    wait_until_drawn(&fx.lent.host, &terminal, "hello-from-the-phone");

    // Another agent in the same workspace, waiting for its first message.
    let reply = fx.ask(
        &visitor,
        &out,
        "r3",
        "session.start",
        json!({"workspace": made[0].id, "harness": "sh-agent", "prompt": "echo second"}),
    );
    assert_eq!(reply["result"]["workspace"], made[0].id, "{reply}");
    assert_ne!(reply["result"]["terminal"], started["terminal"]);
    wait_until_drawn(
        &fx.lent.host,
        &SessionId(reply["result"]["terminal"].as_str().unwrap().to_owned()),
        "second",
    );
}

#[test]
fn starting_is_refused_without_control_outside_the_grant_or_for_an_unknown_agent() {
    let fx = Started::new();
    // Watching only.
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let start = |id: &str, params: Value| fx.ask(&visitor, &out, id, "workspace.start", params);
    let asked = json!({"project": fx.project, "prompt": "echo hi"});
    assert_eq!(code(&start("r1", asked.clone())), "forbidden");
    let upload = fx.ask(
        &visitor,
        &out,
        "r2",
        "upload.begin",
        json!({"workspace": fx.project, "name": "a.png", "size": 3}),
    );
    assert_eq!(code(&upload), "forbidden");

    fx.approve(&CONTROL);
    // A project the grant does not reach looks like one that does not exist.
    assert_eq!(
        code(&start(
            "r3",
            json!({"project": fx.elsewhere, "prompt": "echo hi"})
        )),
        "not_found"
    );
    // A harness that is configured but turned off is not one it can start.
    assert_eq!(
        code(&start(
            "r4",
            json!({"project": fx.project, "prompt": "echo hi", "harness": "claude"})
        )),
        "invalid_request"
    );
    // Nothing was made for any of them.
    assert!(fx.workspaces().is_empty());
}

/// A first message that keeps the shell agent running.
const LONG: &str = if cfg!(windows) {
    "ping -n 30 127.0.0.1"
} else {
    "sleep 30"
};

/// The workspace's conversations, ended ones too, as the phone reads them.
fn sessions(fx: &Started, visitor: &Visitor, out: &Recorded, id: &str, workspace: &Value) -> Value {
    let reply = fx.ask(
        visitor,
        out,
        id,
        "session.list",
        json!({"workspace": workspace, "includeEnded": true}),
    );
    reply["result"]["sessions"].clone()
}

/// Wait until the daemon says the terminal's program ended.
fn wait_until_exited(host: &PtyHost, terminal: &SessionId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while host.info(terminal).unwrap().state == SessionState::Running {
        assert!(std::time::Instant::now() < deadline, "it never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_phone_with_control_resumes_an_ended_conversation_under_its_record() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let welcome = &out.texts()[0]["welcome"]["host"]["capabilities"];
    assert!(welcome
        .as_array()
        .unwrap()
        .contains(&json!("session.resume")));

    let reply = fx.ask(
        &visitor,
        &out,
        "r1",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo first-turn"}),
    );
    let workspace = reply["result"]["workspace"].clone();
    let first = SessionId(reply["result"]["terminal"].as_str().unwrap().to_owned());
    // Running, it is not one to resume.
    let listed = sessions(&fx, &visitor, &out, "r2", &workspace);
    let record = listed[0]["id"].clone();
    wait_until_drawn(&fx.lent.host, &first, "first-turn");
    wait_until_exited(&fx.lent.host, &first);
    // Ended, though nobody settled its record yet, it is.
    let listed = sessions(&fx, &visitor, &out, "r3", &workspace);
    assert_eq!(listed[0]["id"], record);
    assert_eq!(listed[0]["resumable"], true, "{listed}");
    // A device let only watch is told of nothing to resume.
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    let watching = sessions(&fx, &visitor, &out, "r3w", &workspace);
    assert_eq!(watching[0]["resumable"], false, "{watching}");
    fx.approve(&CONTROL);

    let reply = fx.ask(
        &visitor,
        &out,
        "r4",
        "session.resume",
        json!({"session": record}),
    );
    let resumed = &reply["result"];
    assert_eq!(resumed["kind"], "started", "{reply}");
    assert_eq!(resumed["workspace"], workspace);
    assert_eq!(resumed["harness"], "sh-agent");
    let terminal = SessionId(resumed["terminal"].as_str().unwrap().to_owned());
    assert_ne!(terminal, first);
    // The same conversation, by the harness's own id for it, under the same record.
    let info = fx.lent.host.info(&terminal).unwrap();
    assert_eq!(
        info.labels[crate::launch::RECORD_LABEL],
        record.as_str().unwrap()
    );
    wait_until_drawn(&fx.lent.host, &terminal, "resumed-");
    let records = fx.lent.store.sessions(workspace.as_str().unwrap()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].pty_session_id.as_deref(),
        Some(terminal.0.as_str())
    );
}

#[test]
fn resuming_is_refused_without_control_outside_the_grant_or_while_it_runs() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let reply = fx.ask(
        &visitor,
        &out,
        "r1",
        "workspace.start",
        json!({"project": fx.project, "prompt": LONG}),
    );
    let workspace = reply["result"]["workspace"].clone();
    let record = sessions(&fx, &visitor, &out, "r2", &workspace)[0]["id"].clone();
    let resume = |id: &str, session: &Value| {
        fx.ask(
            &visitor,
            &out,
            id,
            "session.resume",
            json!({"session": session}),
        )
    };
    // Still running.
    let refused = resume("r3", &record);
    assert_eq!(code(&refused), "invalid_request");
    assert_eq!(
        refused["error"]["message"],
        "This session is already running."
    );
    // No such conversation.
    assert_eq!(code(&resume("r4", &json!("no-such-record"))), "not_found");
    // One in a project the grant does not reach looks like one that does not exist.
    let theirs = fx
        .lent
        .store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.project_id == fx.elsewhere)
        .unwrap();
    fx.lent
        .store
        .add_session(&crate::store::NewSession {
            id: "theirs",
            workspace_id: &theirs.id,
            harness_id: "sh-agent",
            harness_session_id: Some("their-own-id"),
            title: "theirs",
            pty_session_id: "gone",
            ..Default::default()
        })
        .unwrap();
    fx.lent.store.end_session_by_pty("gone", Some(0)).unwrap();
    assert_eq!(code(&resume("r5", &json!("theirs"))), "not_found");

    // Watching only.
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    assert_eq!(code(&resume("r6", &record)), "forbidden");
}

#[test]
fn a_host_that_lends_nothing_to_start_with_neither_offers_nor_answers_it() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = Visitor::open(
        &fx.lent.store,
        DeviceId::new(PHONE).unwrap(),
        GrantId::new("g-phone").unwrap(),
        Arc::clone(&out) as Arc<dyn Outbound>,
    )
    .unwrap();
    let hello = json!({"client": "test-phone/1", "protocols": [1]}).to_string();
    visitor.handle(&fx.context(false), &hello).unwrap();
    let capabilities = out.texts()[0]["welcome"]["host"]["capabilities"].clone();
    assert!(!capabilities
        .as_array()
        .unwrap()
        .contains(&json!("harness.list")));
    let request = json!({
        "requestId": "r1",
        "protocol": ProtocolVersion::CURRENT,
        "op": "workspace.start",
        "params": {"project": fx.project, "prompt": "echo hi"},
    });
    visitor
        .handle(&fx.context(false), &request.to_string())
        .unwrap();
    let reply = out.texts().into_iter().last().unwrap();
    assert_eq!(code(&reply), "unsupported");
    assert!(fx.workspaces().is_empty());
}

#[test]
fn an_upload_lands_whole_outside_the_worktree_or_not_at_all() {
    use base64::Engine;
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let fx = Started::new();
    fx.approve(&CONTROL);
    let workspace = fx
        .lent
        .store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.project_id == fx.project)
        .unwrap();
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let begin = |id: &str, size: usize| {
        let reply = fx.ask(
            &visitor,
            &out,
            id,
            "upload.begin",
            json!({"workspace": workspace.id, "name": "IMG 4141.HEIC.jpg", "size": size}),
        );
        reply["result"]["upload"]
            .as_u64()
            .unwrap_or_else(|| panic!("{reply}"))
    };
    let chunk = |id: &str, upload: u64, offset: usize, bytes: &[u8]| {
        fx.ask(
            &visitor,
            &out,
            id,
            "upload.chunk",
            json!({"upload": upload, "offset": offset, "data": b64(bytes)}),
        )
    };
    let photo: Vec<u8> = (0..70_000u32).map(|n| (n % 251) as u8).collect();
    let uploads = fx.lent.data.join("uploads").join(&workspace.id);

    // In pieces, in order.
    let upload = begin("r1", photo.len());
    let pieces: Vec<&[u8]> = photo.chunks(remote_proto::MAX_UPLOAD_CHUNK).collect();
    let mut offset = 0;
    let mut last = Value::Null;
    for (n, piece) in pieces.iter().enumerate() {
        last = chunk(&format!("c{n}"), upload, offset, piece);
        offset += piece.len();
        if n + 1 < pieces.len() {
            assert_eq!(last["result"]["kind"], "done", "{last}");
        }
    }
    assert_eq!(last["result"]["kind"], "uploaded", "{last}");
    let path = PathBuf::from(last["result"]["path"].as_str().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), photo);
    assert_eq!(path.parent().unwrap(), uploads);
    assert!(
        path.to_string_lossy().ends_with("-IMG4141HEIC.jpg"),
        "{path:?}"
    );
    assert!(!path.starts_with(&workspace.path));

    // A piece out of order ends the upload, and what came of it goes.
    let upload = begin("r2", photo.len());
    assert_eq!(
        code(&chunk("d1", upload, 10, &photo[..10])),
        "invalid_request"
    );
    assert_eq!(code(&chunk("d2", upload, 0, &photo[..10])), "not_found");
    // So does more than it said it was.
    let upload = begin("r3", 4);
    assert_eq!(code(&chunk("e1", upload, 0, b"12345")), "invalid_request");

    // One the channel closes on is deleted.
    let upload = begin("r4", photo.len());
    chunk("f1", upload, 0, &photo[..100]);
    visitor.close(fx.lent.host.as_ref());
    let left: Vec<_> = std::fs::read_dir(&uploads)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(left, [path]);
}

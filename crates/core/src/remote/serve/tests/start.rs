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
        // It stays after saying so, waiting for a line nobody sends: a resume settles its record
        // at once should the program have exited already, which on a busy machine an instant
        // `echo` sometimes had. The host ends it when the test does.
        resume_args: Some(vec![
            flag.into(),
            if cfg!(windows) {
                "echo resumed-{session_id} & set /p line=".into()
            } else {
                "echo resumed-{session_id}; read line".into()
            },
        ]),
        session_id_mode: Some(SessionIdMode::Assigned),
        ..Default::default()
    }
}

/// A harness that says what it was started with, in whatever order its arguments come: a model,
/// an effort and the first message. It has one effort and suggests one model.
fn telling_harness() -> HarnessOverride {
    let strings = |all: &[&str]| Some(all.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    HarnessOverride {
        id: "sh-teller".into(),
        label: Some("Telling agent".into()),
        builtin: Some(false),
        command: Some(if cfg!(windows) { "cmd.exe" } else { "sh" }.into()),
        base_args: if cfg!(windows) {
            strings(&["/C", "echo", "told"])
        } else {
            strings(&["-c", "echo told \"$@\"; read line", "sh"])
        },
        model_args: strings(&["model={model}"]),
        effort_args: strings(&["effort={effort}"]),
        prompt_args: strings(&["{prompt}"]),
        models: strings(&["large"]),
        efforts: strings(&["high"]),
        ..Default::default()
    }
}

/// What the app lends a channel, made of this test's own store, terminals and folders.
struct Lent {
    git: Git,
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
            git: &self.git,
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
        new: &crate::remote::start::NewWorkspace,
    ) -> crate::error::IpcResult<(String, SessionInfo)> {
        self.with(|s| s.workspace(new))
    }
    fn shell(&self, workspace: &str) -> crate::error::IpcResult<SessionInfo> {
        self.with(|s| s.shell(workspace))
    }
    fn branches(&self, project: &str) -> crate::error::IpcResult<remote_proto::ProjectBranches> {
        self.with(|s| s.branches(project))
    }
    fn clone_project(
        &self,
        remote: &str,
        name: Option<&str>,
    ) -> crate::error::IpcResult<(String, String)> {
        self.with(|s| s.clone_project(remote, name))
    }
    fn rename(&self, workspace: &str, name: &str) -> crate::error::IpcResult<()> {
        self.with(|s| s.rename(workspace, name))
    }
    fn archive(&self, workspace: &str, force: bool) -> crate::error::IpcResult<()> {
        self.with(|s| s.archive(workspace, force))
    }
    fn delete(&self, workspace: &str, force: bool) -> crate::error::IpcResult<()> {
        self.with(|s| s.delete(workspace, force))
    }
    fn changes(&self, workspace: &str) -> crate::error::IpcResult<crate::changes::ChangeSet> {
        self.with(|s| s.changes(workspace))
    }
    fn files(
        &self,
        workspace: &str,
        dir: &str,
    ) -> crate::error::IpcResult<Vec<crate::file_tree::FileEntry>> {
        self.with(|s| s.files(workspace, dir))
    }
    fn content(
        &self,
        workspace: &str,
        path: &str,
        old_path: Option<&str>,
        of: remote_proto::FileOf,
    ) -> crate::error::IpcResult<crate::changes::Content> {
        self.with(|s| s.content(workspace, path, old_path, of))
    }
    fn publish(
        &self,
        workspace: &str,
        forge: bool,
    ) -> crate::error::IpcResult<remote_proto::publish::PublishState> {
        self.with(|s| s.publish(workspace, forge))
    }
    fn commit(
        &self,
        workspace: &str,
        message: &str,
    ) -> crate::error::IpcResult<remote_proto::publish::PublishState> {
        self.with(|s| s.commit(workspace, message))
    }
    fn push(
        &self,
        workspace: &str,
    ) -> crate::error::IpcResult<remote_proto::publish::PublishState> {
        self.with(|s| s.push(workspace))
    }
    fn open_pull_request(
        &self,
        workspace: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> crate::error::IpcResult<remote_proto::publish::PullRequestOpened> {
        self.with(|s| s.open_pull_request(workspace, title, body, draft))
    }
    fn merge_pull_request(
        &self,
        workspace: &str,
        number: u32,
        head_oid: &str,
        method: remote_proto::publish::MergeMethod,
    ) -> crate::error::IpcResult<()> {
        self.with(|s| s.merge_pull_request(workspace, number, head_oid, method))
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
        Self::with_git(git)
    }

    /// As [`Self::new`], with the host's git made by `make`: one whose configuration a test set.
    fn with_git(make: impl Fn() -> Git) -> Self {
        Self::with(make, None)
    }

    /// As [`Self::with_git`], with `bin` first on the host's `PATH`: where a test put a stand-in
    /// for a program the host looks for.
    fn with(make: impl Fn() -> Git, bin: Option<&Path>) -> Self {
        let git = make();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        git.run(repo.path(), &["checkout", "-b", "trunk"]).unwrap();
        git.initial_commit(repo.path()).unwrap();
        let worktrees = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        // Where a clone a device asks for goes: never this user's own folder for them.
        let cloned = tempfile::tempdir().unwrap();
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
            harnesses: [shell_harness(), telling_harness()]
                .into_iter()
                .chain(builtins_off)
                .collect(),
            remote: crate::settings::RemoteSettings {
                projects_dir: Some(
                    cloned
                        .path()
                        .join("projects")
                        .to_string_lossy()
                        .into_owned(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };
        Self {
            lent: Arc::new(Lent {
                git: make(),
                store: Arc::new(store),
                host: Arc::new(PtyHost::new(Arc::new(|_| {}))),
                env: Arc::new(ShellEnv {
                    vars: std::env::vars()
                        .map(|(key, value)| match bin {
                            Some(bin) if key == "PATH" => {
                                let joined = std::env::join_paths(
                                    std::iter::once(bin.to_owned())
                                        .chain(std::env::split_paths(&value)),
                                )
                                .unwrap();
                                (key, joined.to_string_lossy().into_owned())
                            }
                            _ => (key, value),
                        })
                        .collect(),
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
            _dirs: vec![repo, worktrees, data, cloned],
        }
    }

    fn approve(&self, scopes: &[Scope]) {
        self.approve_for(
            scopes,
            &ProjectGrant::Only(BTreeSet::from([ProjectId::new(&self.project).unwrap()])),
        );
    }

    fn approve_for(&self, scopes: &[Scope], projects: &ProjectGrant) {
        host::keep_approved(
            &self.lent.store,
            &GrantId::new("g-phone").unwrap(),
            &DeviceId::new(PHONE).unwrap(),
            "Phone",
            &scopes.iter().copied().collect::<BTreeSet<_>>(),
            projects,
            1,
        )
        .unwrap();
    }

    /// The project's repository, where its branches are.
    fn repo(&self) -> PathBuf {
        PathBuf::from(
            self.lent
                .store
                .project(&self.project)
                .unwrap()
                .unwrap()
                .root_path,
        )
    }

    fn context(&self, starting: bool) -> Context<'_> {
        Context {
            store: &self.lent.store,
            git: &self.git,
            terminals: self.lent.host.as_ref(),
            host: &self.identity,
            interactive: true,
            starting: starting.then(|| Arc::clone(&self.lent) as Arc<dyn Starting>),
            workflow: None,
            conversations: None,
            tests: None,
            pulls: None,
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
        json!([
            {"id": "sh-agent", "name": "Shell agent"},
            // Since minor 12: what its composer offers for it.
            {"id": "sh-teller", "name": "Telling agent", "models": ["large"], "efforts": ["high"]},
        ])
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

#[test]
fn a_start_says_a_name_a_model_an_effort_and_the_branch_to_start_from_or_to_open() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let repo = fx.repo();
    // A branch ahead of the default, and one to open as it is.
    fx.git.run(&repo, &["branch", "develop"]).unwrap();
    fx.git.run(&repo, &["branch", "kept"]).unwrap();
    let ahead = tempfile::tempdir().unwrap();
    let side = ahead.path().join("side");
    fx.git
        .run(
            &repo,
            &["worktree", "add", &side.to_string_lossy(), "develop"],
        )
        .unwrap();
    fx.git
        .run(&side, &["commit", "--allow-empty", "-m", "only on develop"])
        .unwrap();
    let develop = fx.git.run(&repo, &["rev-parse", "develop"]).unwrap();
    fx.git
        .run(&repo, &["worktree", "remove", &side.to_string_lossy()])
        .unwrap();

    let listed = fx.ask(
        &visitor,
        &out,
        "b1",
        "project.branches",
        json!({"project": fx.project}),
    );
    assert_eq!(listed["result"]["kind"], "branches", "{listed}");
    assert_eq!(listed["result"]["default"], "trunk");
    assert_eq!(
        listed["result"]["branches"],
        json!(["trunk", "develop", "kept"])
    );
    assert_eq!(listed["result"]["checkedOut"], json!(["trunk"]));

    let reply = fx.ask(
        &visitor,
        &out,
        "s1",
        "workspace.start",
        json!({
            "project": fx.project,
            "prompt": "tidy-the-notes",
            "harness": "sh-teller",
            "name": "Tidy Up!",
            "model": "large",
            "effort": "high",
            "baseBranch": "develop",
        }),
    );
    assert_eq!(reply["result"]["kind"], "started", "{reply}");
    let made = fx.workspaces();
    assert_eq!(made.len(), 1);
    // Named as said, made a slug; its branch starts where the one said does.
    assert_eq!(made[0].name, "tidy-up");
    assert!(made[0].branch.as_deref().unwrap().ends_with("tidy-up"));
    assert_eq!(made[0].base_branch.as_deref(), Some("develop"));
    assert_eq!(
        fx.git
            .run(Path::new(&made[0].path), &["rev-parse", "HEAD"])
            .unwrap(),
        develop
    );
    // The agent was told the model and the effort, as its settings pass them.
    let terminal = SessionId(reply["result"]["terminal"].as_str().unwrap().to_owned());
    wait_until_drawn(&fx.lent.host, &terminal, "model=large");
    wait_until_drawn(&fx.lent.host, &terminal, "effort=high");
    wait_until_drawn(&fx.lent.host, &terminal, "tidy-the-notes");

    // An existing branch is opened as it is: no new branch, and the workspace named after it.
    let before = fx.git.branches(&repo).unwrap();
    let reply = fx.ask(
        &visitor,
        &out,
        "s2",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo on-kept", "existingBranch": "kept"}),
    );
    assert_eq!(reply["result"]["kind"], "started", "{reply}");
    assert_eq!(fx.git.branches(&repo).unwrap(), before);
    let opened: Vec<_> = fx
        .workspaces()
        .into_iter()
        .filter(|w| w.branch.as_deref() == Some("kept"))
        .collect();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].base_branch, None);

    // What cannot be: an effort that agent has not, a branch that is not there, and an existing
    // branch with a name or a base. Nothing is made for any of them.
    let count = fx.workspaces().len();
    for (id, params) in [
        (
            "e1",
            json!({"prompt": "x", "harness": "sh-teller", "effort": "ludicrous"}),
        ),
        (
            "e2",
            json!({"prompt": "x", "harness": "sh-agent", "effort": "high"}),
        ),
        ("e3", json!({"prompt": "x", "baseBranch": "not-there"})),
        ("e4", json!({"prompt": "x", "existingBranch": "not-there"})),
        (
            "e5",
            json!({"prompt": "x", "existingBranch": "develop", "name": "n"}),
        ),
    ] {
        let mut params = params;
        params["project"] = json!(fx.project);
        let reply = fx.ask(&visitor, &out, id, "workspace.start", params);
        assert_eq!(reply["error"]["code"], "invalid_request", "{id}: {reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(
            !message.contains(&*repo.to_string_lossy()),
            "{id}: {message}"
        );
    }
    assert_eq!(fx.workspaces().len(), count);
    // Branches of a project the grant does not reach are not told.
    let reply = fx.ask(
        &visitor,
        &out,
        "b2",
        "project.branches",
        json!({"project": fx.elsewhere}),
    );
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
}

#[test]
fn a_shell_starts_in_a_workspace_and_a_workspace_is_renamed() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let reply = fx.ask(
        &visitor,
        &out,
        "s1",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo first"}),
    );
    let workspace = reply["result"]["workspace"].as_str().unwrap().to_owned();

    let shell = fx.ask(
        &visitor,
        &out,
        "sh",
        "shell.start",
        json!({"workspace": workspace}),
    );
    assert_eq!(shell["result"]["kind"], "started", "{shell}");
    assert_eq!(shell["result"]["harness"], "");
    assert_eq!(shell["result"]["workspace"], workspace);
    let terminal = SessionId(shell["result"]["terminal"].as_str().unwrap().to_owned());
    let info = fx.lent.host.info(&terminal).unwrap();
    assert_eq!(info.labels[WORKSPACE_LABEL], workspace);
    assert!(!info.labels.contains_key(crate::launch::HARNESS_LABEL));

    let renamed = fx.ask(
        &visitor,
        &out,
        "rn",
        "workspace.rename",
        json!({"workspace": workspace, "name": "Login spacing"}),
    );
    assert_eq!(renamed["result"]["kind"], "done", "{renamed}");
    let row = fx.lent.store.workspace(&workspace).unwrap().unwrap();
    assert_eq!(row.name, "Login spacing");
    // Its folder and its branch keep their names.
    assert!(Path::new(&row.path).is_dir());
    assert!(row.branch.as_deref().unwrap().contains("echo-first"));

    // The project's own checkout is not one to rename, archive or delete.
    let local = fx
        .lent
        .store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.project_id == fx.project && w.kind == "local");
    if let Some(local) = local {
        for op in ["workspace.archive", "workspace.delete"] {
            let reply = fx.ask(
                &visitor,
                &out,
                op,
                op,
                json!({"workspace": local.id, "force": true}),
            );
            assert_eq!(reply["error"]["code"], "invalid_request", "{op}: {reply}");
        }
        assert!(fx.repo().is_dir());
    }
    // Watching only: none of it.
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    for (op, params) in [
        ("shell.start", json!({"workspace": workspace})),
        (
            "workspace.rename",
            json!({"workspace": workspace, "name": "x"}),
        ),
        (
            "workspace.delete",
            json!({"workspace": workspace, "force": true}),
        ),
    ] {
        let reply = fx.ask(&visitor, &out, &format!("w-{op}"), op, params);
        assert_eq!(reply["error"]["code"], "forbidden", "{op}: {reply}");
    }
    assert!(Path::new(&row.path).is_dir());
}

#[test]
fn uncommitted_work_is_kept_until_archiving_or_deleting_is_asked_a_second_time() {
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let mut made = Vec::new();
    for (id, prompt) in [("s1", "echo to-archive"), ("s2", "echo to-delete")] {
        let reply = fx.ask(
            &visitor,
            &out,
            id,
            "workspace.start",
            json!({"project": fx.project, "prompt": prompt}),
        );
        let id = reply["result"]["workspace"].as_str().unwrap().to_owned();
        let row = fx.lent.store.workspace(&id).unwrap().unwrap();
        std::fs::write(Path::new(&row.path).join("notes.txt"), "not committed").unwrap();
        made.push(row);
    }
    for (row, op) in made.iter().zip(["workspace.archive", "workspace.delete"]) {
        let notes = Path::new(&row.path).join("notes.txt");
        // Without the second yes it is refused, in words that name no folder, and nothing goes.
        let reply = fx.ask(
            &visitor,
            &out,
            &format!("no-{op}"),
            op,
            json!({"workspace": row.id}),
        );
        assert_eq!(reply["error"]["code"], "uncommitted_work", "{op}: {reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(!message.contains(&row.path), "{message}");
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "not committed");
        assert!(fx.lent.store.workspace(&row.id).unwrap().is_some());
        // With it, the folder goes and the branch stays.
        let reply = fx.ask(
            &visitor,
            &out,
            &format!("yes-{op}"),
            op,
            json!({"workspace": row.id, "force": true}),
        );
        assert_eq!(reply["result"]["kind"], "done", "{op}: {reply}");
        assert!(!Path::new(&row.path).exists());
        assert!(fx
            .git
            .branch_exists(&fx.repo(), row.branch.as_deref().unwrap())
            .unwrap());
    }
    // Archived, its record is kept to restore it; deleted, it is forgotten.
    assert!(
        fx.lent
            .store
            .workspace(&made[0].id)
            .unwrap()
            .unwrap()
            .archived
    );
    assert!(fx.lent.store.workspace(&made[1].id).unwrap().is_none());
}

#[test]
fn a_repository_is_cloned_into_the_hosts_projects_folder_and_added_as_a_project() {
    // A repository on this disk, reached by an address a device may give: git rewrites it.
    let origin = tempfile::tempdir().unwrap();
    let setup = git();
    let source = origin.path().join("source");
    std::fs::create_dir(&source).unwrap();
    setup.init(&source).unwrap();
    setup.initial_commit(&source).unwrap();
    let bare = origin.path().join("weather.git");
    setup
        .run(origin.path(), &["clone", "--bare", "source", "weather.git"])
        .unwrap();
    assert!(bare.is_dir());
    let base = format!(
        "file:///{}/",
        origin
            .path()
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let key = format!("url.{base}.insteadOf");
    let rewritten = move || {
        crate::git::testing::git_with(&[
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", &key),
            ("GIT_CONFIG_VALUE_0", "https://git.example.test/acme/"),
        ])
    };
    let fx = Started::with_git(rewritten);
    let out = Recorded::new(usize::MAX);

    // A device let into some projects cannot add one: it could not reach what it cloned.
    fx.approve(&CONTROL);
    let visitor = fx.visitor(&out);
    let clone = json!({"remote": "https://git.example.test/acme/weather.git"});
    let reply = fx.ask(&visitor, &out, "c0", "project.clone", clone.clone());
    assert_eq!(reply["error"]["code"], "forbidden", "{reply}");

    fx.approve_for(&CONTROL, &ProjectGrant::All);
    let reply = fx.ask(&visitor, &out, "c1", "project.clone", clone.clone());
    assert_eq!(reply["result"]["kind"], "cloned", "{reply}");
    assert_eq!(reply["result"]["name"], "weather");
    let id = reply["result"]["project"].as_str().unwrap();
    let row = fx.lent.store.project(id).unwrap().unwrap();
    // In the folder the host's settings name, made for it, and nowhere a device said.
    let projects = PathBuf::from(fx.lent.settings.remote.projects_dir.clone().unwrap());
    assert_eq!(
        crate::git::normalize(Path::new(&row.root_path)),
        crate::git::normalize(&projects.join("weather"))
    );
    assert!(Path::new(&row.root_path).join(".git").exists());
    // It is a project like any other: a workspace starts in it, and its summary says where it
    // is fetched from, as the device asked for it.
    let listed = fx.ask(&visitor, &out, "l1", "workspace.list", json!({}));
    let summary = listed["result"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(
        summary["remote"],
        "https://git.example.test/acme/weather.git"
    );
    let started = fx.ask(
        &visitor,
        &out,
        "s1",
        "workspace.start",
        json!({"project": id, "prompt": "echo in-the-clone"}),
    );
    assert_eq!(started["result"]["kind"], "started", "{started}");

    // The same again is refused, since the folder is there, and nothing of it is touched.
    let again = fx.ask(&visitor, &out, "c2", "project.clone", clone);
    assert_eq!(again["error"]["code"], "invalid_request", "{again}");
    assert!(Path::new(&row.root_path).join(".git").exists());
    // One that cannot be cloned says so without the host's folders, and leaves nothing behind.
    let missing = fx.ask(
        &visitor,
        &out,
        "c3",
        "project.clone",
        json!({"remote": "https://git.example.test/acme/not-there.git"}),
    );
    assert_eq!(missing["error"]["code"], "invalid_request", "{missing}");
    let message = missing["error"]["message"].as_str().unwrap();
    assert!(!message.contains(&*projects.to_string_lossy()), "{message}");
    assert!(!projects.join("not-there").exists());
    // What is not an address is not passed to git at all.
    for (n, remote) in ["/etc", "file:///etc", "--upload-pack=x", "ext::sh -c x"]
        .into_iter()
        .enumerate()
    {
        let reply = fx.ask(
            &visitor,
            &out,
            &format!("x{n}"),
            "project.clone",
            json!({"remote": remote}),
        );
        assert_eq!(
            reply["error"]["code"], "invalid_request",
            "{remote}: {reply}"
        );
    }
    assert_eq!(
        std::fs::read_dir(&projects).unwrap().count(),
        1,
        "only the one clone"
    );
}

#[test]
fn a_remote_is_told_only_when_another_computer_could_clone_it_and_never_with_credentials() {
    use crate::remote::start::{name_from_remote, repository_key, shareable_remote};
    assert_eq!(
        shareable_remote("https://user:secret@github.com/acme/app.git").as_deref(),
        Some("https://github.com/acme/app.git")
    );
    assert_eq!(
        shareable_remote("git@github.com:acme/app.git").as_deref(),
        Some("git@github.com:acme/app.git")
    );
    for folder in [
        "/srv/git/app.git",
        "../app",
        "file:///srv/app.git",
        "git://host/app",
        "acme/app",
    ] {
        assert_eq!(shareable_remote(folder), None, "{folder}");
    }
    // The same repository, however each computer reaches it.
    for same in [
        "https://github.com/acme/app.git",
        "https://GitHub.com/acme/app",
        "git@github.com:acme/app.git",
        "ssh://git@github.com:22/acme/app.git",
    ] {
        assert_eq!(
            repository_key(same).as_deref(),
            Some("github.com/acme/app"),
            "{same}"
        );
    }
    assert_ne!(
        repository_key("https://github.com/acme/app"),
        repository_key("https://gitlab.com/acme/app")
    );
    assert_eq!(repository_key("/srv/git/app.git"), None);
    assert_eq!(
        name_from_remote("git@github.com:acme/app.git").as_deref(),
        Some("app")
    );
    assert_eq!(
        name_from_remote("https://example.com/group/sub/site/").as_deref(),
        Some("site")
    );
    assert_eq!(name_from_remote("https://example.com/"), None);
}

#[test]
fn a_device_with_control_reads_a_workspaces_changes_its_tree_and_its_files_and_nothing_outside() {
    use base64::Engine;
    let fx = Started::new();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let reply = fx.ask(
        &visitor,
        &out,
        "s1",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo reading"}),
    );
    let workspace = reply["result"]["workspace"].as_str().unwrap().to_owned();
    let root = PathBuf::from(fx.lent.store.workspace(&workspace).unwrap().unwrap().path);
    let write = |path: &str, bytes: &[u8]| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    // One file committed on the branch and changed since, one new, one ignored, one too big for
    // a piece, one that is not text, and a secret beside the workspace.
    write(".gitignore", b"secret.env\n");
    write("src/app.txt", b"first\n");
    fx.git.run(&root, &["add", "."]).unwrap();
    fx.git.run(&root, &["commit", "-m", "add the app"]).unwrap();
    write("src/app.txt", b"first\nsecond\n");
    write("notes.txt", b"not committed\n");
    write("secret.env", b"TOKEN=1\n");
    let long = "\u{e9}".repeat(remote_proto::MAX_FILE_CHUNK);
    write("long.txt", long.as_bytes());
    write("logo.bin", &[0, 159, 146, 150, 0, 1, 2]);
    std::fs::write(root.parent().unwrap().join("outside.txt"), b"not yours").unwrap();

    let ask = |id: &str, op: &str, mut params: Value| {
        params["workspace"] = json!(workspace);
        fx.ask(&visitor, &out, id, op, params)
    };
    let changes = ask("c1", "workspace.changes", json!({}));
    assert_eq!(changes["result"]["kind"], "changes", "{changes}");
    let paths = |group: &str| -> Vec<String> {
        changes["result"][group]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                format!(
                    "{} {}",
                    f["kind"].as_str().unwrap(),
                    f["path"].as_str().unwrap()
                )
            })
            .collect()
    };
    let uncommitted = paths("uncommitted");
    assert!(
        uncommitted.contains(&"modified src/app.txt".to_owned()),
        "{uncommitted:?}"
    );
    assert!(
        uncommitted.contains(&"untracked notes.txt".to_owned()),
        "{uncommitted:?}"
    );
    assert!(
        !uncommitted.iter().any(|p| p.contains("secret.env")),
        "{uncommitted:?}"
    );
    assert_eq!(
        paths("committed"),
        ["added .gitignore", "added src/app.txt"]
    );
    assert_eq!(changes["result"]["base"], "trunk");

    // The tree: folders first, and neither `.git` nor what Git ignores.
    let tree = ask("f1", "workspace.files", json!({}));
    let names: Vec<&str> = tree["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[0], "src", "{tree}");
    assert_eq!(tree["result"]["entries"][0]["dir"], true);
    assert!(
        names.contains(&"notes.txt") && names.contains(&".gitignore"),
        "{names:?}"
    );
    assert!(
        !names.contains(&".git") && !names.contains(&"secret.env"),
        "{names:?}"
    );
    let inside = ask("f2", "workspace.files", json!({"dir": "src"}));
    assert_eq!(inside["result"]["entries"], json!([{"name": "app.txt"}]));

    let decode = |reply: &Value| {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(reply["result"]["data"].as_str().unwrap_or(""))
                .unwrap(),
        )
    };
    // A file on disk, and both sides of each kind of change.
    let read = |id: &str, path: &str, of: &str, offset: u64| {
        ask(
            id,
            "file.read",
            json!({"path": path, "of": of, "offset": offset}),
        )
    };
    let now = read("r1", "src/app.txt", "working", 0);
    assert_eq!(now["result"]["state"], "text", "{now}");
    assert_eq!(decode(&now).unwrap(), "first\nsecond\n");
    assert_eq!(now["result"]["done"], true);
    assert_eq!(
        decode(&read("r2", "src/app.txt", "uncommittedOld", 0)).unwrap(),
        "first\n"
    );
    assert_eq!(
        read("r3", "src/app.txt", "committedOld", 0)["result"]["state"],
        "absent"
    );
    assert_eq!(
        decode(&read("r4", "src/app.txt", "committedNew", 0)).unwrap(),
        "first\n"
    );
    assert_eq!(
        read("r5", "notes.txt", "uncommittedOld", 0)["result"]["state"],
        "absent"
    );

    // A file larger than a piece comes in pieces, each from where the last ended. A piece may
    // end inside a character, and the pieces together are the file.
    let first = read("l1", "long.txt", "working", 0);
    assert_eq!(first["result"]["done"], false, "{first}");
    assert_eq!(first["result"]["size"], long.len());
    let mut bytes = base64::engine::general_purpose::STANDARD
        .decode(first["result"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes.len(), remote_proto::MAX_FILE_CHUNK);
    let second = read("l2", "long.txt", "working", bytes.len() as u64);
    assert_eq!(second["result"]["done"], true, "{second}");
    bytes.extend(
        base64::engine::general_purpose::STANDARD
            .decode(second["result"]["data"].as_str().unwrap())
            .unwrap(),
    );
    assert_eq!(String::from_utf8(bytes).unwrap(), long);
    let past = read("l3", "long.txt", "working", long.len() as u64 + 1);
    assert_eq!(past["error"]["code"], "invalid_request", "{past}");

    // What is not text is said to be so and not sent.
    let binary = read("b1", "logo.bin", "working", 0);
    assert_eq!(binary["result"]["state"], "binary", "{binary}");
    assert!(binary["result"].get("data").is_none());
    assert_eq!(
        read("b2", "no-such-file.txt", "working", 0)["result"]["state"],
        "absent"
    );

    // Nothing outside the workspace: not by climbing, not by a whole path, not through a link.
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.parent().unwrap().join("outside.txt"),
        root.join("link.txt"),
    )
    .unwrap();
    let whole = root.parent().unwrap().join("outside.txt");
    for (n, path) in [
        "../outside.txt",
        &whole.to_string_lossy(),
        "link.txt",
        "src/../../outside.txt",
    ]
    .into_iter()
    .enumerate()
    {
        if path == "link.txt" && !cfg!(unix) {
            continue;
        }
        let reply = read(&format!("o{n}"), path, "working", 0);
        assert_eq!(reply["error"]["code"], "invalid_request", "{path}: {reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(!message.contains(&*root.to_string_lossy()), "{message}");
    }
    let up = ask("o9", "workspace.files", json!({"dir": ".."}));
    assert_eq!(up["error"]["code"], "invalid_request", "{up}");

    // Another project's workspace is not there for this device, and watching alone reads nothing.
    let elsewhere = fx
        .lent
        .store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.project_id == fx.elsewhere)
        .unwrap();
    let other = fx.ask(
        &visitor,
        &out,
        "e1",
        "workspace.changes",
        json!({"workspace": elsewhere.id}),
    );
    assert_eq!(other["error"]["code"], "not_found", "{other}");
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    for (id, op, params) in [
        ("w1", "workspace.changes", json!({})),
        ("w2", "workspace.files", json!({})),
        (
            "w3",
            "file.read",
            json!({"path": "notes.txt", "of": "working"}),
        ),
    ] {
        let reply = ask(id, op, params);
        assert_eq!(reply["error"]["code"], "forbidden", "{op}: {reply}");
    }
}

#[cfg(unix)]
#[test]
fn a_device_with_control_commits_pushes_and_opens_and_merges_a_pull_request_as_the_host() {
    use std::os::unix::fs::PermissionsExt;
    // A remote on this disk behind an address under a name reserved for tests, which no program
    // could reach if it tried, and a stand-in for the forge's program that notes what it was asked.
    let outside = tempfile::tempdir().unwrap();
    let bare = outside.path().join("alpha.git");
    git()
        .run(outside.path(), &["init", "--bare", "alpha.git"])
        .unwrap();
    let asked = outside.path().join("gh.log");
    let listed = outside.path().join("list.json");
    let bin = outside.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(
        bin.join("gh"),
        format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1 $2\" in\n  'pr list') cat '{list}' 2>/dev/null || echo '[]' ;;\n  'pr create') echo 'Creating pull request'; echo 'https://git.example.test/acme/alpha/pull/7' ;;\n  'pr merge') exit 0 ;;\n  *) echo \"not answered: $*\" >&2; exit 1 ;;\nesac\n",
            log = asked.display(),
            list = listed.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let key = format!("url.file://{}/.insteadOf", outside.path().display());
    let rewritten = move || {
        crate::git::testing::git_with(&[
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", &key),
            ("GIT_CONFIG_VALUE_0", "https://git.example.test/acme/"),
        ])
    };
    let fx = Started::with(rewritten, Some(&bin));
    fx.git
        .run(
            &fx.repo(),
            &[
                "remote",
                "add",
                "origin",
                "https://git.example.test/acme/alpha.git",
            ],
        )
        .unwrap();
    fx.approve(&CONTROL);
    let out = Recorded::new(usize::MAX);
    let visitor = fx.visitor(&out);
    let reply = fx.ask(
        &visitor,
        &out,
        "s1",
        "workspace.start",
        json!({"project": fx.project, "prompt": "echo publishing"}),
    );
    let workspace = reply["result"]["workspace"].as_str().unwrap().to_owned();
    let row = fx.lent.store.workspace(&workspace).unwrap().unwrap();
    let root = PathBuf::from(&row.path);
    let branch = row.branch.clone().unwrap();
    let ask = |id: &str, op: &str, mut params: Value| {
        params["workspace"] = json!(workspace);
        fx.ask(&visitor, &out, id, op, params)
    };
    let head = || fx.git.run(&root, &["rev-parse", "HEAD"]).unwrap();
    let on_remote = || {
        git()
            .run(&bare, &["rev-parse", &format!("refs/heads/{branch}")])
            .ok()
    };
    let refused = |reply: &Value, said: &str| {
        assert_eq!(reply["error"]["code"], "invalid_request", "{reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains(said), "{message}");
        // Never Git's own words, which name this host's folders and its remote's address.
        for leak in [outside.path().to_string_lossy(), root.to_string_lossy()] {
            assert!(!message.contains(&*leak), "{message}");
        }
    };

    // Where it stands before anything is done: a branch of its own, nothing pushed.
    let before = ask("p1", "workspace.publish", json!({}));
    assert_eq!(before["result"]["kind"], "publish", "{before}");
    assert_eq!(before["result"]["branch"], branch);
    assert_eq!(before["result"]["base"], "trunk");
    assert_eq!(before["result"]["remote"], true);
    assert!(before["result"].get("pushed").is_none());
    assert!(before["result"]["identity"]
        .as_str()
        .unwrap()
        .contains("Test"));
    assert!(
        before["result"].get("pullRequest").is_none(),
        "the forge was not asked"
    );
    assert!(!asked.exists(), "nor was its program run");

    // A commit is everything changed, by the host's own identity.
    std::fs::write(root.join("notes.txt"), "to commit\n").unwrap();
    let committed = ask(
        "c1",
        "workspace.commit",
        json!({"message": "Add the notes\n\nWith a body."}),
    );
    assert_eq!(committed["result"]["ahead"], 1, "{committed}");
    assert_eq!(committed["result"]["unpushed"], json!(["Add the notes"]));
    assert_eq!(fx.git.run(&root, &["status", "--porcelain"]).unwrap(), "");
    assert!(fx
        .git
        .run(&root, &["log", "-1", "--format=%an <%ae>"])
        .unwrap()
        .contains("Test <test@example.com>"));
    refused(
        &ask("c2", "workspace.commit", json!({"message": "Again"})),
        "nothing to commit",
    );
    assert_eq!(
        ask("c3", "workspace.commit", json!({"message": " "}))["error"]["code"],
        "invalid_request"
    );

    // A push sends the workspace's own branch to the project's own remote.
    assert_eq!(on_remote(), None);
    let pushed = ask("u1", "workspace.push", json!({}));
    assert_eq!(pushed["result"]["pushed"], true, "{pushed}");
    assert_eq!(pushed["result"]["ahead"], 0);
    assert_eq!(on_remote(), Some(head()));

    // A push the remote refuses says why, in the host's words.
    let hook = bare.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'no pushes here' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(root.join("more.txt"), "another\n").unwrap();
    ask("c4", "workspace.commit", json!({"message": "Add more"}));
    let sent = on_remote();
    refused(&ask("u2", "workspace.push", json!({})), "refused the push");
    assert_eq!(on_remote(), sent, "nothing reached the remote");
    std::fs::remove_file(&hook).unwrap();

    // Opening a pull request pushes what the remote lacks first, then asks the forge.
    let opened = ask(
        "o1",
        "pull_request.open",
        json!({"title": "Add the notes", "body": "Two files.", "draft": true}),
    );
    assert_eq!(opened["result"]["kind"], "pullRequestOpened", "{opened}");
    assert_eq!(
        opened["result"]["url"],
        "https://git.example.test/acme/alpha/pull/7"
    );
    assert_eq!(opened["result"]["created"], true);
    assert_eq!(on_remote(), Some(head()));
    let log = std::fs::read_to_string(&asked).unwrap();
    assert!(
        log.contains("pr create --base trunk --title Add the notes --body Two files. --draft"),
        "{log}"
    );

    // With the forge asked, its pull request is told, and there is none to open any more.
    std::fs::write(
        &listed,
        json!([{
            "number": 7,
            "url": "https://git.example.test/acme/alpha/pull/7",
            "title": "Add the notes",
            "headRefName": branch,
            "state": "OPEN",
            "isDraft": false,
            "headRefOid": head(),
            "mergeable": "MERGEABLE",
        }])
        .to_string(),
    )
    .unwrap();
    let with_forge = ask("p2", "workspace.publish", json!({"forge": true}));
    let pr = &with_forge["result"]["pullRequest"];
    assert_eq!(pr["number"], 7, "{with_forge}");
    assert_eq!(pr["state"], "open");
    assert_eq!(pr["headOid"], head());
    assert!(with_forge["result"].get("canOpen").is_none());

    // A merge names the commit the person saw. Another one, or another number, merges nothing.
    for (id, number, oid) in [
        (
            "m1",
            7,
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
        ),
        ("m2", 8, head()),
    ] {
        let reply = ask(
            id,
            "pull_request.merge",
            json!({"number": number, "headOid": oid, "method": "squash"}),
        );
        refused(&reply, "changed or is no longer ready");
    }
    assert!(!std::fs::read_to_string(&asked)
        .unwrap()
        .contains("pr merge"));
    let merged = ask(
        "m3",
        "pull_request.merge",
        json!({"number": 7, "headOid": head(), "method": "squash"}),
    );
    assert_eq!(merged["result"]["kind"], "done", "{merged}");
    assert!(std::fs::read_to_string(&asked).unwrap().contains(&format!(
        "pr merge 7 --squash --match-head-commit {}",
        head()
    )));

    // Watching only: none of it, and nothing is run for it.
    let ran = std::fs::read_to_string(&asked).unwrap();
    fx.approve(&[Scope::WorkspaceRead, Scope::TerminalRead]);
    for (id, op, params) in [
        ("w1", "workspace.publish", json!({})),
        ("w2", "workspace.commit", json!({"message": "No"})),
        ("w3", "workspace.push", json!({})),
        ("w4", "pull_request.open", json!({"title": "No"})),
        (
            "w5",
            "pull_request.merge",
            json!({"number": 7, "headOid": head(), "method": "merge"}),
        ),
    ] {
        let reply = ask(id, op, params);
        assert_eq!(reply["error"]["code"], "forbidden", "{op}: {reply}");
    }
    assert_eq!(std::fs::read_to_string(&asked).unwrap(), ran);
}

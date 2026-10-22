//! The facade against the real things: a SQLite store, git repositories and worktrees in temp
//! directories, and processes in real PTYs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pty_host::{
    AttachmentId, LaunchPlan, OutputSink, PtyHost, Result as HostResult, SessionId, SessionInfo,
    SessionState, TerminalHost,
};
use remote_proto::{
    AttachRequest, DeviceId, ErrorCode, Grant, Hello, Operation, Outcome, ProcessState,
    ProjectGrant, ProtocolVersion, Reply, Request, RequestId, Response, Scope, SessionPage,
    TerminalActivity, WorkspaceListParams, WorkspacePage, WorkspaceRef,
};

use super::*;
use crate::git::{normalize, testing::git};
use crate::launch::{HARNESS_SESSION_LABEL, RECORD_LABEL};
use crate::store::NewSession;

/// A host's records: two projects, each with its own checkout and one worktree.
struct Fixture {
    store: Store,
    git: Git,
    dir: tempfile::TempDir,
    identity: HostIdentity,
    alpha: Project,
    beta: Project,
}

struct Project {
    id: String,
    local: String,
    worktree: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let git = git();
        let alpha = project(&store, &git, &normalize(dir.path()), "alpha");
        let beta = project(&store, &git, &normalize(dir.path()), "beta");
        Self {
            store,
            git,
            dir,
            identity: HostIdentity {
                id: HostId::new("host-test").unwrap(),
                name: "Test host".into(),
            },
            alpha,
            beta,
        }
    }

    fn facade<'a>(&'a self, terminals: Option<&'a dyn TerminalHost>) -> HostFacade<'a> {
        HostFacade {
            store: &self.store,
            git: &self.git,
            terminals,
            host: &self.identity,
        }
    }

    /// Something no response may contain: the temp directory's own name, which every path in
    /// this fixture includes, written however the platform writes paths.
    fn private(&self) -> String {
        self.dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
}

/// A repository with one commit, added as a project, and a worktree on a branch of its own.
fn project(store: &Store, git: &Git, parent: &Path, name: &str) -> Project {
    let root = parent.join(name);
    std::fs::create_dir_all(&root).unwrap();
    git.init(&root).unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    git.commit_all(&root, "first").unwrap();
    let base = git.run(&root, &["branch", "--show-current"]).unwrap();
    let row = store.add_project(name, &root.to_string_lossy()).unwrap();
    let tree = parent.join(format!("{name}-wt"));
    let branch = format!("ys/{name}-work");
    git.worktree_add(&root, &tree, &branch, &base).unwrap();
    let worktree = store
        .add_worktree(
            &row.id,
            "work",
            &tree.to_string_lossy(),
            Some(&branch),
            Some(&base),
        )
        .unwrap();
    let local = store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.project_id == row.id && w.kind == "local")
        .unwrap();
    Project {
        id: row.id,
        local: local.id,
        worktree: worktree.id,
    }
}

fn grant(scopes: &[Scope], projects: ProjectGrant) -> Grant {
    Grant {
        device: DeviceId::new("device-test").unwrap(),
        scopes: scopes.iter().copied().collect(),
        projects,
    }
}

fn everything() -> Grant {
    grant(
        &[
            Scope::WorkspaceRead,
            Scope::ActivityRead,
            Scope::MemoryRead,
            Scope::TerminalRead,
            Scope::TerminalControl,
            Scope::DeviceManage,
        ],
        ProjectGrant::All,
    )
}

fn only(project: &str, scopes: &[Scope]) -> Grant {
    grant(
        scopes,
        ProjectGrant::Only([ProjectId::new(project).unwrap()].into()),
    )
}

fn request(operation: Operation) -> Request {
    Request {
        request_id: RequestId::new("r-1").unwrap(),
        protocol: ProtocolVersion::CURRENT,
        operation,
    }
}

/// Ask, through the wire format: what a client sends and what it would get back.
fn ask(facade: &HostFacade<'_>, grant: &Grant, operation: Operation) -> Response {
    let bytes = serde_json::to_vec(&request(operation)).unwrap();
    serde_json::from_slice(&facade.handle_bytes(grant, &bytes)).unwrap()
}

fn reply(response: Response) -> Reply {
    match response.outcome {
        Outcome::Result(reply) => reply,
        Outcome::Error(error) => panic!("refused: {error:?}"),
    }
}

fn refusal(response: Response) -> remote_proto::RemoteError {
    match response.outcome {
        Outcome::Error(error) => error,
        Outcome::Result(reply) => panic!("answered: {reply:?}"),
    }
}

fn workspaces(
    facade: &HostFacade<'_>,
    grant: &Grant,
    params: WorkspaceListParams,
) -> WorkspacePage {
    match reply(ask(facade, grant, Operation::WorkspaceList(params))) {
        Reply::Workspaces(page) => page,
        other => panic!("{other:?}"),
    }
}

fn sessions(
    facade: &HostFacade<'_>,
    grant: &Grant,
    workspace: &str,
    include_ended: bool,
) -> SessionPage {
    let params = remote_proto::SessionListParams {
        workspace: WorkspaceId::new(workspace).unwrap(),
        include_ended,
        cursor: None,
        limit: None,
    };
    match reply(ask(facade, grant, Operation::SessionList(params))) {
        Reply::Sessions(page) => page,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_host_describes_itself_and_advertises_only_what_it_answers() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let Reply::Host(host) = reply(ask(
        &facade,
        &grant(&[], ProjectGrant::All),
        Operation::HostDescribe,
    )) else {
        panic!()
    };
    assert_eq!(host.host.as_str(), "host-test");
    assert_eq!(host.protocol, ProtocolVersion::CURRENT);
    assert_eq!(host.capabilities, ANSWERED);
    assert!(
        !host
            .capabilities
            .iter()
            .any(|op| op.starts_with("terminal.")),
        "terminals are not offered before leases are enforced"
    );
}

#[test]
fn workspaces_are_listed_with_their_branches_and_never_their_paths() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let page = workspaces(&facade, &everything(), WorkspaceListParams::default());

    let names: Vec<_> = page.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta"]);
    assert_eq!(page.workspaces.len(), 4);
    let work = page
        .workspaces
        .iter()
        .find(|w| w.id.as_str() == f.alpha.worktree)
        .unwrap();
    assert_eq!(work.branch.as_ref().unwrap().label, "ys/alpha-work");
    assert_eq!(work.kind, remote_proto::WorkspaceKind::Worktree);
    assert!(!work.missing);

    let wire = String::from_utf8(facade.handle_bytes(
        &everything(),
        &serde_json::to_vec(&request(Operation::WorkspaceList(Default::default()))).unwrap(),
    ))
    .unwrap();
    assert!(!wire.contains(&f.private()), "a path leaked: {wire}");
}

#[test]
fn a_grant_for_one_project_cannot_see_or_probe_another() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let alpha_only = only(&f.alpha.id, &[Scope::WorkspaceRead, Scope::MemoryRead]);

    let page = workspaces(&facade, &alpha_only, WorkspaceListParams::default());
    assert_eq!(page.projects.len(), 1);
    assert!(page
        .workspaces
        .iter()
        .all(|w| w.project.as_str() == f.alpha.id));

    // Beta's ids, and ids that exist nowhere, get the same answer: nothing tells them apart.
    let nowhere = "00000000-0000-4000-8000-000000000000";
    let asks = |workspace: &str, project: &str| {
        [
            Operation::WorkspaceDescribe(WorkspaceRef {
                workspace: WorkspaceId::new(workspace).unwrap(),
            }),
            Operation::SessionList(remote_proto::SessionListParams {
                workspace: WorkspaceId::new(workspace).unwrap(),
                include_ended: true,
                cursor: None,
                limit: None,
            }),
            Operation::MemoryList(remote_proto::MemoryListParams {
                project: ProjectId::new(project).unwrap(),
                cursor: None,
                limit: None,
            }),
            Operation::WorkspaceList(WorkspaceListParams {
                project: Some(ProjectId::new(project).unwrap()),
                ..Default::default()
            }),
        ]
    };
    for (theirs, absent) in asks(&f.beta.worktree, &f.beta.id)
        .into_iter()
        .zip(asks(nowhere, nowhere))
    {
        let theirs = refusal(ask(&facade, &alpha_only, theirs));
        let absent = refusal(ask(&facade, &alpha_only, absent));
        assert_eq!(theirs.code, ErrorCode::NotFound);
        assert_eq!(theirs, absent);
    }

    // And the same grant does reach its own project's workspace.
    let Reply::Workspace(detail) = reply(ask(
        &facade,
        &alpha_only,
        Operation::WorkspaceDescribe(WorkspaceRef {
            workspace: WorkspaceId::new(&f.alpha.local).unwrap(),
        }),
    )) else {
        panic!()
    };
    assert_eq!(detail.project.name, "alpha");
    assert_eq!(detail.workspace.kind, remote_proto::WorkspaceKind::Local);
}

#[test]
fn a_missing_scope_is_forbidden_before_anything_is_read() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let memory_only = grant(&[Scope::MemoryRead], ProjectGrant::All);
    let error = refusal(ask(
        &facade,
        &memory_only,
        Operation::WorkspaceList(Default::default()),
    ));
    assert_eq!(error.code, ErrorCode::Forbidden);
    assert!(
        error.message.contains("workspace:read"),
        "{}",
        error.message
    );

    let workspace_only = grant(&[Scope::WorkspaceRead], ProjectGrant::All);
    let error = refusal(ask(
        &facade,
        &workspace_only,
        Operation::MemoryList(remote_proto::MemoryListParams {
            project: ProjectId::new(&f.alpha.id).unwrap(),
            cursor: None,
            limit: None,
        }),
    ));
    assert_eq!(error.code, ErrorCode::Forbidden);
}

#[test]
fn a_client_speaking_another_major_is_turned_away() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let mut future = request(Operation::HostDescribe);
    future.protocol = ProtocolVersion { major: 2, minor: 0 };
    let error = refusal(facade.handle(&everything(), &future));
    assert_eq!(error.code, ErrorCode::ProtocolIncompatible);
    assert_eq!(error.supported_protocols, Some(vec![1]));

    // The same, arriving as bytes with fields this build has never heard of.
    let bytes = br#"{"requestId":"r","protocol":{"major":2,"minor":0},"op":"host.describe","params":{"x":[1]}}"#;
    let response: Response =
        serde_json::from_slice(&facade.handle_bytes(&everything(), bytes)).unwrap();
    assert_eq!(refusal(response).code, ErrorCode::ProtocolIncompatible);

    let hello = Hello {
        client: "old-client/0.0.1".into(),
        protocols: vec![0],
    };
    assert_eq!(
        facade.welcome(&hello).unwrap_err().code,
        ErrorCode::ProtocolIncompatible
    );
    let hello = Hello {
        client: "client/0.1.0".into(),
        protocols: vec![2, 1],
    };
    assert_eq!(
        facade.welcome(&hello).unwrap().protocol,
        ProtocolVersion::CURRENT
    );
}

#[test]
fn archived_workspaces_are_left_out_unless_asked_for() {
    let f = Fixture::new();
    let facade = f.facade(None);
    f.store
        .set_workspace_archived(&f.beta.worktree, true)
        .unwrap();
    let listed = |include_archived| {
        workspaces(
            &facade,
            &everything(),
            WorkspaceListParams {
                include_archived,
                ..Default::default()
            },
        )
        .workspaces
        .into_iter()
        .map(|w| w.id.as_str().to_owned())
        .collect::<Vec<_>>()
    };
    assert!(!listed(false).contains(&f.beta.worktree));
    assert!(listed(true).contains(&f.beta.worktree));
}

#[test]
fn pages_follow_cursors_and_a_cursor_whose_item_went_expires() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let all = workspaces(&facade, &everything(), WorkspaceListParams::default());
    let first = workspaces(
        &facade,
        &everything(),
        WorkspaceListParams {
            limit: Some(2),
            ..Default::default()
        },
    );
    assert_eq!(first.workspaces, all.workspaces[..2]);
    let cursor = first.next.clone().expect("two more workspaces are left");
    let second = workspaces(
        &facade,
        &everything(),
        WorkspaceListParams {
            limit: Some(2),
            cursor: Some(cursor.clone()),
            ..Default::default()
        },
    );
    assert_eq!(second.workspaces, all.workspaces[2..]);
    assert_eq!(second.next, None);
    // The page's projects are only those its workspaces belong to.
    assert_eq!(second.projects.len(), 1);

    // The workspace the cursor points after is gone: where the next page starts is unknowable.
    // (A worktree: a project's own checkout cannot be forgotten.)
    let last_of_first = first.workspaces.last().unwrap().id.as_str().to_owned();
    f.store.forget_workspace(&last_of_first, false).unwrap();
    let error = refusal(ask(
        &facade,
        &everything(),
        Operation::WorkspaceList(WorkspaceListParams {
            cursor: Some(cursor),
            ..Default::default()
        }),
    ));
    assert_eq!(error.code, ErrorCode::CursorExpired);

    let error = refusal(ask(
        &facade,
        &everything(),
        Operation::WorkspaceList(WorkspaceListParams {
            cursor: Some(remote_proto::Cursor::new("k0.something").unwrap()),
            ..Default::default()
        }),
    ));
    assert_eq!(error.code, ErrorCode::CursorExpired);
}

#[test]
fn only_approved_memory_is_shown_and_reading_it_changes_nothing() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let approved =
        crate::memory::write(&f.store, &f.alpha.id, "Run the tests with TZ=UTC.").unwrap();
    let proposed = |text: &str| match crate::memory::propose(
        &f.store,
        &f.alpha.id,
        text,
        &crate::memory::Source::default(),
    )
    .unwrap()
    {
        crate::memory::Proposed::New(id) => id,
        other => panic!("{other:?}"),
    };
    let candidate = proposed("An agent's unreviewed idea.");
    let rejected = proposed("An idea the user turned down.");
    crate::memory::decide(&f.store, &rejected, crate::memory::Decision::Reject).unwrap();
    let revoked = crate::memory::write(&f.store, &f.alpha.id, "Withdrawn later.").unwrap();
    crate::memory::decide(&f.store, &revoked, crate::memory::Decision::Revoke).unwrap();
    // What reaches agents is the project's switch; what reaches the person's device is the grant.
    f.store.set_memory_shared(&f.alpha.id, false).unwrap();

    let Reply::Memory(page) = reply(ask(
        &facade,
        &everything(),
        Operation::MemoryList(remote_proto::MemoryListParams {
            project: ProjectId::new(&f.alpha.id).unwrap(),
            cursor: None,
            limit: None,
        }),
    )) else {
        panic!()
    };
    let ids: Vec<_> = page.notes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, [approved.as_str()]);
    assert_eq!(page.notes[0].author, "user");

    let state = |id: &str| f.store.memory_entry(id).unwrap().unwrap().state;
    assert_eq!(state(&candidate), "candidate");
    assert_eq!(state(&rejected), "rejected");
    assert!(!f.store.memory_shared(&f.alpha.id).unwrap());
}

/// A terminal host that lets the facade list sessions and fails the test on anything else: the
/// facade must never write to, resize, attach to, start or stop a terminal.
struct ListOnly(Arc<PtyHost>);

impl TerminalHost for ListOnly {
    fn spawn(&self, _: LaunchPlan) -> HostResult<SessionInfo> {
        panic!("the facade started a process")
    }
    fn attach(&self, _: &SessionId, _: OutputSink) -> HostResult<AttachmentId> {
        panic!("the facade attached to a terminal")
    }
    fn detach(&self, _: &SessionId, _: AttachmentId) -> HostResult<()> {
        panic!("the facade detached from a terminal")
    }
    fn write(&self, _: &SessionId, _: &[u8]) -> HostResult<()> {
        panic!("the facade typed into a terminal")
    }
    fn paste(&self, _: &SessionId, _: &str) -> HostResult<()> {
        panic!("the facade pasted into a terminal")
    }
    fn resize(&self, _: &SessionId, _: pty_host::TermSize) -> HostResult<()> {
        panic!("the facade resized a terminal")
    }
    fn kill(&self, _: &SessionId) -> HostResult<()> {
        panic!("the facade killed a process")
    }
    fn remove(&self, _: &SessionId) -> HostResult<()> {
        panic!("the facade removed a terminal")
    }
    fn info(&self, _: &SessionId) -> HostResult<SessionInfo> {
        panic!("the facade asked for one session rather than the list")
    }
    fn list(&self) -> Vec<SessionInfo> {
        self.0.list()
    }
}

/// A process that stays alive until killed, labelled as Yardsort labels a harness.
fn agent(host: &PtyHost, workspace: &str, record: &str, dir: &Path) -> SessionInfo {
    let (program, args): (&str, Vec<String>) = if cfg!(windows) {
        ("cmd.exe", vec!["/C".into(), "ping -n 60 127.0.0.1".into()])
    } else {
        (
            "/bin/sh",
            vec!["-c".into(), "echo started; sleep 60".into()],
        )
    };
    let mut labels = BTreeMap::new();
    labels.insert(WORKSPACE_LABEL.to_owned(), workspace.to_owned());
    labels.insert(RECORD_LABEL.to_owned(), record.to_owned());
    labels.insert(
        HARNESS_SESSION_LABEL.to_owned(),
        "native-conversation-id".to_owned(),
    );
    host.spawn(LaunchPlan {
        program: program.into(),
        args,
        cwd: Some(PathBuf::from(dir)),
        env: vec![("REMOTE_TEST_SECRET".into(), "hunter2".into())],
        clear_env: false,
        size: pty_host::TermSize {
            cols: 100,
            rows: 30,
        },
        labels,
        prompt: None,
    })
    .unwrap()
}

fn record(store: &Store, id: &str, workspace: &str, pty: &str) {
    store
        .add_session(&NewSession {
            id,
            workspace_id: workspace,
            harness_id: "claude",
            model: Some("opus"),
            title: "Fix the login timeout",
            harness_session_id: Some("native-conversation-id"),
            pty_session_id: pty,
            prompt: Some("Fix the login timeout, and here is a secret: hunter2"),
            ..Default::default()
        })
        .unwrap();
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn sessions_say_what_the_daemon_knows_and_nothing_it_was_told_in_confidence() {
    let f = Fixture::new();
    let host = Arc::new(PtyHost::new(Arc::new(|_| {})));
    let list_only = ListOnly(Arc::clone(&host));
    let ws = f.alpha.worktree.clone();

    // A live agent with its record.
    let live = agent(&host, &ws, "rec-live", f.dir.path());
    record(&f.store, "rec-live", &ws, &live.id.0);
    // A record that says running, though its process is gone.
    let gone = agent(&host, &ws, "rec-gone", f.dir.path());
    record(&f.store, "rec-gone", &ws, &gone.id.0);
    host.remove(&gone.id).unwrap();
    // A record pointing at a process that was started in another workspace. It is not shown here.
    let elsewhere = agent(&host, &f.beta.worktree, "rec-elsewhere", f.dir.path());
    record(&f.store, "rec-elsewhere", &ws, &elsewhere.id.0);
    // A conversation that ended.
    record(&f.store, "rec-ended", &ws, "pty-long-gone");
    f.store
        .end_session_by_pty("pty-long-gone", Some(0))
        .unwrap();

    wait_for("the live agent to print", || {
        host.info(&live.id).is_ok_and(|info| info.has_output)
    });

    let facade = f.facade(Some(&list_only));
    let page = sessions(&facade, &everything(), &ws, false);
    let state = |page: &SessionPage, id: &str| {
        page.sessions
            .iter()
            .find(|s| s.id.as_str() == id)
            .map(|s| (s.process, s.terminal.clone()))
    };
    let (process, terminal) = state(&page, "rec-live").unwrap();
    assert_eq!(process, ProcessState::Running);
    let terminal = terminal.unwrap();
    assert_eq!(terminal.id.as_str(), live.id.0);
    assert_eq!((terminal.size.cols, terminal.size.rows), (100, 30));
    assert!(matches!(
        terminal.activity,
        TerminalActivity::Busy | TerminalActivity::Quiet
    ));
    assert_eq!(
        state(&page, "rec-gone").unwrap(),
        (ProcessState::Gone, None)
    );
    assert_eq!(
        state(&page, "rec-elsewhere").unwrap(),
        (ProcessState::Gone, None)
    );
    assert!(
        state(&page, "rec-ended").is_none(),
        "ended ones only when asked"
    );

    let page = sessions(&facade, &everything(), &ws, true);
    assert_eq!(
        state(&page, "rec-ended").unwrap(),
        (ProcessState::Ended, None)
    );

    // No daemon to ask: a running record is reported as unknown, not as running or gone.
    let page = sessions(&f.facade(None), &everything(), &ws, false);
    assert_eq!(
        state(&page, "rec-live").unwrap(),
        (ProcessState::Unknown, None)
    );

    // What crossed the wire: no argv, no environment, no labels, no first message, no paths.
    let params = remote_proto::SessionListParams {
        workspace: WorkspaceId::new(&ws).unwrap(),
        include_ended: true,
        cursor: None,
        limit: None,
    };
    let wire = String::from_utf8(facade.handle_bytes(
        &everything(),
        &serde_json::to_vec(&request(Operation::SessionList(params))).unwrap(),
    ))
    .unwrap();
    for secret in [
        "hunter2",
        "native-conversation-id",
        "sleep 60",
        "ping -n",
        "REMOTE_TEST_SECRET",
        &f.private(),
    ] {
        assert!(!wire.contains(secret), "{secret} leaked: {wire}");
    }

    // The exit is noticed too, and the terminal stays viewable with its code.
    host.kill(&live.id).unwrap();
    wait_for("the live agent to exit", || {
        host.info(&live.id)
            .is_ok_and(|info| matches!(info.state, SessionState::Exited { .. }))
    });
    let page = sessions(&facade, &everything(), &ws, false);
    let (process, terminal) = state(&page, "rec-live").unwrap();
    assert_eq!(
        process,
        ProcessState::Gone,
        "the record has not been settled yet"
    );
    assert_eq!(terminal.unwrap().activity, TerminalActivity::Exited);
}

#[test]
fn terminal_operations_are_refused_and_touch_no_terminal() {
    let f = Fixture::new();
    let host = Arc::new(PtyHost::new(Arc::new(|_| {})));
    let list_only = ListOnly(Arc::clone(&host));
    let live = agent(&host, &f.alpha.worktree, "rec", f.dir.path());
    let facade = f.facade(Some(&list_only));
    let attach = || {
        Operation::TerminalAttach(AttachRequest {
            terminal: remote_proto::TerminalId::new(&live.id.0).unwrap(),
            resume_after: None,
        })
    };

    let error = refusal(ask(&facade, &everything(), attach()));
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert!(!error.retryable);

    // Without the scope it is forbidden first: a client learns nothing about terminals it may
    // not see.
    let error = refusal(ask(
        &facade,
        &grant(&[Scope::WorkspaceRead], ProjectGrant::All),
        attach(),
    ));
    assert_eq!(error.code, ErrorCode::Forbidden);

    let input = Operation::TerminalInput(remote_proto::InputRequest {
        lease: remote_proto::LeaseRef {
            terminal: remote_proto::TerminalId::new(&live.id.0).unwrap(),
            lease: remote_proto::LeaseId::new("guessed").unwrap(),
            generation: 1,
        },
        seq: 1,
        data: "rm -rf ~\r".into(),
        paste: false,
    });
    assert_eq!(
        refusal(ask(&facade, &everything(), input)).code,
        ErrorCode::Unsupported
    );
    host.kill(&live.id).unwrap();
}

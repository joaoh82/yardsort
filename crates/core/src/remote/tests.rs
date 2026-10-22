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
use crate::store::{NewEvent, NewRun, NewSession};

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

/// Ask, through the wire format: what a client sends and what it would get back. Every answer
/// in these tests must also be one a client reads, so within a frame.
fn ask(facade: &HostFacade<'_>, grant: &Grant, operation: Operation) -> Response {
    let bytes = serde_json::to_vec(&request(operation)).unwrap();
    let answer = facade.handle_bytes(grant, &bytes);
    assert!(
        answer.len() <= remote_proto::MAX_CONTROL_FRAME,
        "a {} byte response",
        answer.len()
    );
    serde_json::from_slice(&answer).unwrap()
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
    labelled(
        host,
        &[
            (WORKSPACE_LABEL, workspace),
            (RECORD_LABEL, record),
            (HARNESS_SESSION_LABEL, "native-conversation-id"),
        ],
        dir,
    )
}

/// A process that stays alive until killed, with these labels.
fn labelled(host: &PtyHost, labels: &[(&str, &str)], dir: &Path) -> SessionInfo {
    let (program, args): (&str, Vec<String>) = if cfg!(windows) {
        ("cmd.exe", vec!["/C".into(), "ping -n 60 127.0.0.1".into()])
    } else {
        (
            "/bin/sh",
            vec!["-c".into(), "echo started; sleep 60".into()],
        )
    };
    let labels: BTreeMap<String, String> = labels
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
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

#[test]
fn other_terminals_are_listed_once_with_their_purpose_and_without_their_command_line() {
    let f = Fixture::new();
    let host = Arc::new(PtyHost::new(Arc::new(|_| {})));
    let list_only = ListOnly(Arc::clone(&host));
    let ws = f.alpha.worktree.clone();
    let dir = f.dir.path();

    // A conversation, and an earlier process of it that is still on show: both are its, so
    // neither is an "other" terminal.
    let conversation = agent(&host, &ws, "rec-a", dir);
    record(&f.store, "rec-a", &ws, &conversation.id.0);
    let earlier = agent(&host, &ws, "rec-a", dir);
    record(&f.store, "rec-ended", &ws, "pty-long-gone");
    f.store
        .end_session_by_pty("pty-long-gone", Some(0))
        .unwrap();

    let run_command = labelled(
        &host,
        &[(WORKSPACE_LABEL, &ws), (PROJECT_RUN_LABEL, &ws)],
        dir,
    );
    f.store
        .add_run(&NewRun {
            id: "run-shell",
            workspace_id: &ws,
            kind: "shell",
            launched_by: "app",
            ..Default::default()
        })
        .unwrap();
    let shell = labelled(
        &host,
        &[(WORKSPACE_LABEL, &ws), (RUN_LABEL, "run-shell")],
        dir,
    );
    let unrecorded = labelled(&host, &[(WORKSPACE_LABEL, &ws)], dir);
    let orphan = labelled(
        &host,
        &[
            (WORKSPACE_LABEL, &ws),
            (HARNESS_LABEL, "claude"),
            (RECORD_LABEL, "rec-forgotten"),
        ],
        dir,
    );
    // Another workspace's shell is not this one's.
    let theirs = labelled(&host, &[(WORKSPACE_LABEL, &f.beta.worktree)], dir);

    let facade = f.facade(Some(&list_only));
    let page = sessions(&facade, &everything(), &ws, true);
    let purpose = |id: &SessionId| {
        page.terminals
            .iter()
            .find(|t| t.terminal.id.as_str() == id.0)
            .map(|t| (t.purpose, t.harness.clone()))
    };
    assert_eq!(page.terminals.len(), 4, "{:?}", page.terminals);
    assert_eq!(purpose(&conversation.id), None);
    assert_eq!(purpose(&earlier.id), None);
    assert_eq!(purpose(&theirs.id), None);
    assert_eq!(
        purpose(&run_command.id),
        Some((TerminalPurpose::RunCommand, None))
    );
    assert_eq!(purpose(&shell.id), Some((TerminalPurpose::Shell, None)));
    assert_eq!(
        purpose(&unrecorded.id),
        Some((TerminalPurpose::Other, None))
    );
    assert_eq!(
        purpose(&orphan.id),
        Some((TerminalPurpose::Agent, Some("claude".into())))
    );
    let program = if cfg!(windows) { "cmd" } else { "sh" };
    assert!(page.terminals.iter().all(|t| t.program == program));

    let params = |cursor, limit| remote_proto::SessionListParams {
        workspace: WorkspaceId::new(&ws).unwrap(),
        include_ended: true,
        cursor,
        limit,
    };
    let wire = String::from_utf8(facade.handle_bytes(
        &everything(),
        &serde_json::to_vec(&request(Operation::SessionList(params(None, None)))).unwrap(),
    ))
    .unwrap();
    for secret in ["sleep 60", "ping -n", "/bin/", "run-shell", &f.private()] {
        assert!(!wire.contains(secret), "{secret} leaked: {wire}");
    }

    // They come with the first page only, and only when there is a daemon to ask.
    let Reply::Sessions(first) = reply(ask(
        &facade,
        &everything(),
        Operation::SessionList(params(None, Some(1))),
    )) else {
        panic!()
    };
    assert_eq!(first.terminals.len(), 4);
    let Reply::Sessions(second) = reply(ask(
        &facade,
        &everything(),
        Operation::SessionList(params(first.next, Some(1))),
    )) else {
        panic!()
    };
    assert_eq!(second.sessions.len(), 1);
    assert!(second.terminals.is_empty());
    assert!(sessions(&f.facade(None), &everything(), &ws, true)
        .terminals
        .is_empty());

    for info in host.list() {
        host.kill(&info.id).unwrap();
    }
}

/// Record an event as a producer would.
fn event(
    store: &Store,
    id: &str,
    workspace: &str,
    session: Option<&str>,
    kind: &str,
    payload: serde_json::Value,
) {
    add_event(store, id, workspace, session, kind, "metadata", payload);
}

fn add_event(
    store: &Store,
    id: &str,
    workspace: &str,
    session: Option<&str>,
    kind: &str,
    privacy_class: &str,
    payload: serde_json::Value,
) {
    assert!(store
        .add_event(&NewEvent {
            id,
            schema_version: 1,
            workspace_id: workspace,
            session_id: session,
            occurred_at: 1_000,
            kind,
            producer: "yardsort",
            method: "lifecycle",
            fidelity: "observed",
            privacy_class,
            payload: &payload.to_string(),
            ..Default::default()
        })
        .unwrap());
}

fn activity_params(
    workspace: &str,
    session: Option<&str>,
    cursor: Option<remote_proto::Cursor>,
    limit: Option<u32>,
) -> Operation {
    Operation::ActivityList(remote_proto::ActivityListParams {
        workspace: WorkspaceId::new(workspace).unwrap(),
        session: session.map(|s| SessionRecordId::new(s).unwrap()),
        cursor,
        limit,
    })
}

fn activity(facade: &HostFacade<'_>, grant: &Grant, operation: Operation) -> ActivityPage {
    match reply(ask(facade, grant, operation)) {
        Reply::Activity(page) => page,
        other => panic!("{other:?}"),
    }
}

fn ids(page: &ActivityPage) -> Vec<&str> {
    page.events.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn activity_pages_back_and_catches_up_with_only_what_is_new() {
    let f = Fixture::new();
    let ws = f.alpha.worktree.clone();
    record(&f.store, "rec-a", &ws, "pty-a");
    for n in 1..=5 {
        let session = (n % 2 == 1).then_some("rec-a");
        event(
            &f.store,
            &format!("ev-{n}"),
            &ws,
            session,
            "turn.completed",
            serde_json::json!({}),
        );
        // Interleaved: another workspace's, and a class remote readers are not shown.
        event(
            &f.store,
            &format!("beta-{n}"),
            &f.beta.worktree,
            None,
            "turn.completed",
            serde_json::json!({}),
        );
        add_event(
            &f.store,
            &format!("content-{n}"),
            &ws,
            None,
            "prompt.submitted",
            "content",
            serde_json::json!({ "text": "hunter2" }),
        );
    }
    let facade = f.facade(None);
    let all = everything();
    let list = |session, cursor, limit| {
        activity(&facade, &all, activity_params(&ws, session, cursor, limit))
    };

    // The newest first page, oldest first within it; then back, a page at a time, to the start.
    let newest = list(None, None, Some(2));
    assert_eq!(ids(&newest), ["ev-4", "ev-5"]);
    let middle = list(None, newest.older.clone(), Some(2));
    assert_eq!(ids(&middle), ["ev-2", "ev-3"]);
    let oldest = list(None, middle.older.clone(), Some(2));
    assert_eq!(ids(&oldest), ["ev-1"]);
    assert_eq!(oldest.older, None, "nothing older is kept");

    // Back from a reconnect with nothing new: an empty page, and the same place to ask from.
    let caught_up = list(None, newest.newer.clone(), Some(2));
    assert!(caught_up.events.is_empty());
    assert_eq!(caught_up.newer, newest.newer);

    // Three more while away: only those come back, in order, a page at a time.
    for n in 6..=8 {
        event(
            &f.store,
            &format!("ev-{n}"),
            &ws,
            None,
            "turn.completed",
            serde_json::json!({}),
        );
    }
    let first = list(None, newest.newer.clone(), Some(2));
    assert_eq!(ids(&first), ["ev-6", "ev-7"]);
    assert!(first.older.is_some());
    let second = list(None, first.newer.clone(), Some(2));
    assert_eq!(ids(&second), ["ev-8"]);
    let third = list(None, second.newer.clone(), Some(2));
    assert!(third.events.is_empty());

    // One conversation's only.
    let theirs = list(Some("rec-a"), None, None);
    assert_eq!(ids(&theirs), ["ev-1", "ev-3", "ev-5"]);
    assert!(theirs
        .events
        .iter()
        .all(|e| e.session.as_ref().map(SessionRecordId::as_str) == Some("rec-a")));

    // An empty workspace has no cursors at all.
    let empty = activity(
        &facade,
        &all,
        activity_params(&f.alpha.local, None, None, None),
    );
    assert!(empty.events.is_empty());
    assert_eq!((empty.older, empty.newer), (None, None));
}

#[test]
fn activity_is_scoped_and_a_cursor_outlives_nothing_it_named() {
    let f = Fixture::new();
    let ws = f.alpha.worktree.clone();
    record(&f.store, "rec-a", &ws, "pty-a");
    record(&f.store, "rec-b", &ws, "pty-b");
    record(&f.store, "rec-beta", &f.beta.worktree, "pty-beta");
    event(
        &f.store,
        "ev-1",
        &ws,
        Some("rec-a"),
        "turn.completed",
        serde_json::json!({}),
    );
    event(
        &f.store,
        "ev-2",
        &ws,
        Some("rec-b"),
        "turn.completed",
        serde_json::json!({}),
    );
    let facade = f.facade(None);
    let all = everything();

    // The scope is asked for first.
    let error = refusal(ask(
        &facade,
        &grant(&[Scope::WorkspaceRead], ProjectGrant::All),
        activity_params(&ws, None, None, None),
    ));
    assert_eq!(error.code, ErrorCode::Forbidden);
    assert!(error.message.contains("activity:read"), "{}", error.message);

    // Another project's workspace, and a conversation from another workspace, look like nothing.
    let alpha_only = only(&f.alpha.id, &[Scope::ActivityRead]);
    let nowhere = "00000000-0000-4000-8000-000000000000";
    for (theirs, absent) in [
        (
            activity_params(&f.beta.worktree, None, None, None),
            activity_params(nowhere, None, None, None),
        ),
        (
            activity_params(&ws, Some("rec-beta"), None, None),
            activity_params(&ws, Some(nowhere), None, None),
        ),
    ] {
        let theirs = refusal(ask(&facade, &alpha_only, theirs));
        assert_eq!(theirs.code, ErrorCode::NotFound);
        assert_eq!(theirs, refusal(ask(&facade, &alpha_only, absent)));
    }

    // A cursor that is not one of these, or names an event another filter would not show.
    let page = activity(
        &facade,
        &all,
        activity_params(&ws, Some("rec-b"), None, None),
    );
    let cursor = |text: &str| Some(remote_proto::Cursor::new(text).unwrap());
    for (session, bad) in [
        (None, cursor("k1.ev-1")),
        (None, cursor("a1.x2")),
        (None, cursor("a1.o-2")),
        (None, cursor("a1.n")),
        (Some("rec-a"), page.newer.clone()),
    ] {
        let error = refusal(ask(&facade, &all, activity_params(&ws, session, bad, None)));
        assert_eq!(error.code, ErrorCode::CursorExpired);
    }
    // The workspace-wide list does include rec-b's event, so its cursor is good there.
    assert!(activity(
        &facade,
        &all,
        activity_params(&ws, None, page.newer.clone(), None)
    )
    .events
    .is_empty());

    // Clearing the workspace's activity takes the cursor's event with it.
    f.store.clear_activity(Some(&ws)).unwrap();
    let error = refusal(ask(
        &facade,
        &all,
        activity_params(&ws, None, page.newer, None),
    ));
    assert_eq!(error.code, ErrorCode::CursorExpired);
}

#[test]
fn activity_shows_its_listed_fields_and_nothing_a_producer_kept_for_this_host() {
    let f = Fixture::new();
    let ws = f.alpha.worktree.clone();
    record(&f.store, "rec-a", &ws, "pty-a");
    let rows = [
        (
            "process.started",
            serde_json::json!({
                "kind": "harness", "harnessId": "claude", "model": "opus", "effort": "high",
                "program": "claude", "continuation": "fresh", "launchedBy": "app",
                "ptySessionId": "pty-secret-id", "capture": "hook",
            }),
        ),
        (
            "process.spawn_failed",
            serde_json::json!({
                "kind": "program", "program": "/home/someone/bin/tool",
                "reason": "`/home/someone/bin/tool` was not found on PATH",
            }),
        ),
        (
            "turn.completed",
            serde_json::json!({
                "codexHome": "/home/someone/.codex-private", "threadId": "native-thread",
                "detail": "notify", "contextTokens": 1000, "contextWindow": 2000,
            }),
        ),
        (
            "tool.completed",
            serde_json::json!({
                "tool": "Edit", "path": "src/app.rs", "durationMs": 12,
                "toolUseId": "toolu_secret", "command": "rm -rf secret-cmd",
            }),
        ),
        (
            "tool.failed",
            serde_json::json!({ "tool": "Read", "path": "/etc/passwd", "exitCode": -1 }),
        ),
        (
            "prompt.submitted",
            serde_json::json!({ "chars": 147, "prompt": "the text of hunter2" }),
        ),
        (
            "agent.notified",
            serde_json::json!({ "type": "permission_prompt", "message": "Allow hunter2?" }),
        ),
        (
            "session.forked",
            serde_json::json!({ "sessionId": "rec-a", "fromSessionId": "rec-a" }),
        ),
        ("process.exited", serde_json::json!("not an object")),
    ];
    for (n, (kind, payload)) in rows.iter().enumerate() {
        event(
            &f.store,
            &format!("ev-{n}"),
            &ws,
            Some("rec-a"),
            kind,
            payload.clone(),
        );
    }
    let facade = f.facade(None);
    let page = activity(
        &facade,
        &everything(),
        activity_params(&ws, None, None, None),
    );
    assert_eq!(page.events.len(), rows.len());
    let detail = |kind: &str| {
        page.events
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| e.detail.clone())
            .unwrap()
    };

    let started = detail("process.started");
    assert_eq!(started.harness.as_deref(), Some("claude"));
    assert_eq!(started.run_kind.as_deref(), Some("harness"));
    assert_eq!(started.program.as_deref(), Some("claude"));
    assert_eq!(started.model.as_deref(), Some("opus"));
    assert_eq!(started.capture.as_deref(), Some("hook"));
    let failed = detail("process.spawn_failed");
    assert_eq!((failed.program, failed.reason), (None, None));
    let turn = detail("turn.completed");
    assert_eq!(turn.note.as_deref(), Some("notify"));
    assert_eq!(
        (turn.context_tokens, turn.context_window),
        (Some(1000), Some(2000))
    );
    let edit = detail("tool.completed");
    assert_eq!(edit.path.as_deref(), Some("src/app.rs"));
    assert_eq!(edit.duration_ms, Some(12));
    let read = detail("tool.failed");
    assert_eq!(
        read.path, None,
        "a path outside the workspace is not one to send"
    );
    assert_eq!(read.exit_code, Some(-1));
    assert_eq!(detail("prompt.submitted").chars, Some(147));
    assert_eq!(
        detail("agent.notified").notification.as_deref(),
        Some("permission_prompt")
    );
    assert_eq!(
        detail("session.forked")
            .forked_from
            .map(|id| id.as_str().to_owned()),
        Some("rec-a".to_owned())
    );
    assert_eq!(
        detail("process.exited"),
        remote_proto::ActivityDetail::default()
    );
    let first = &page.events[0];
    assert_eq!(first.source.producer, "yardsort");
    assert_eq!(first.session.as_ref().unwrap().as_str(), "rec-a");

    let wire = String::from_utf8(facade.handle_bytes(
        &everything(),
        &serde_json::to_vec(&request(activity_params(&ws, None, None, None))).unwrap(),
    ))
    .unwrap();
    for secret in [
        "pty-secret-id",
        "/home/someone",
        "codex-private",
        "native-thread",
        "/etc/passwd",
        "secret-cmd",
        "toolu_secret",
        "hunter2",
        "not found",
        "effort",
    ] {
        assert!(!wire.contains(secret), "{secret} leaked: {wire}");
    }
}

/// The reproduction from review: fifty approved notes, each at the length limit in four-byte
/// characters, are more than one frame at the default page size.
#[test]
fn a_page_of_large_notes_holds_fewer_and_its_cursor_finds_the_rest() {
    let f = Fixture::new();
    let facade = f.facade(None);
    let mut written: Vec<String> = (0..50)
        .map(|n| {
            let text = format!("{}{n:02}", "\u{1f600}".repeat(crate::memory::MAX_CHARS - 2));
            crate::memory::write(&f.store, &f.alpha.id, &text).unwrap()
        })
        .collect();

    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        // `ask` checks each answer fits in a frame.
        let Reply::Memory(page) = reply(ask(
            &facade,
            &everything(),
            Operation::MemoryList(remote_proto::MemoryListParams {
                project: ProjectId::new(&f.alpha.id).unwrap(),
                cursor: cursor.take(),
                limit: None,
            }),
        )) else {
            panic!()
        };
        assert!(!page.notes.is_empty());
        assert!(page
            .notes
            .iter()
            .all(|n| n.text.ends_with(char::is_numeric)));
        seen.extend(page.notes.iter().map(|n| n.id.as_str().to_owned()));
        pages += 1;
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(pages > 1, "the default page of these is over a frame");
    seen.sort();
    written.sort();
    assert_eq!(seen, written, "every note once, none skipped");
}

/// Activity trims a page from the end away from its cursor, whichever way it walks, and the
/// next page starts where the short one stopped.
#[test]
fn large_events_page_back_and_forward_without_a_gap() {
    let f = Fixture::new();
    let ws = f.alpha.worktree.clone();
    let wide = "\u{1f600}".repeat(super::activity::MAX_NAME);
    let all: Vec<String> = (0..200).map(|n| format!("ev-{n:03}")).collect();
    for id in &all {
        event(
            &f.store,
            id,
            &ws,
            None,
            "tool.completed",
            serde_json::json!({
                "harnessId": wide, "model": wide, "tool": wide, "subagentType": wide,
                "agentType": wide, "program": wide,
                "path": format!("src/{}", "p".repeat(super::activity::MAX_PATH - 4)),
            }),
        );
    }
    let facade = f.facade(None);
    let grant = everything();
    let list = |cursor| {
        activity(
            &facade,
            &grant,
            activity_params(&ws, None, cursor, Some(remote_proto::MAX_PAGE)),
        )
    };

    // Back from the newest: each page oldest first, each one before the last.
    let mut back = Vec::new();
    let mut page = list(None);
    let mut pages = vec![page.clone()];
    loop {
        assert!(!page.events.is_empty());
        let mut ids: Vec<String> = ids(&page).iter().map(|&id| id.to_owned()).collect();
        ids.append(&mut back);
        back = ids;
        let Some(older) = page.older.clone() else {
            break;
        };
        page = list(Some(older));
        pages.push(page.clone());
    }
    assert!(pages.len() > 1, "two hundred of these are over a frame");
    assert_eq!(back, all);

    // Forward again from the oldest page, as a client catching up does.
    let mut forward: Vec<String> = ids(pages.last().unwrap())
        .iter()
        .map(|&id| id.to_owned())
        .collect();
    let mut newer = pages.last().unwrap().newer.clone();
    loop {
        let page = list(newer.clone());
        if page.events.is_empty() {
            break;
        }
        forward.extend(ids(&page).iter().map(|&id| id.to_owned()));
        newer = page.newer;
    }
    assert_eq!(forward, all);
}

/// Every field of every item has a limit, and together they keep one item, at its largest, within
/// a page's budget: so `fitting` always finds room for one, and a list can always go on. Text
/// at its largest is control characters, which JSON writes as six bytes each.
#[test]
fn the_largest_item_of_every_list_fits_in_a_frame() {
    use super::activity::{MAX_CODE, MAX_NAME, MAX_PATH};
    use remote_proto::{
        ActivityDetail, ActivityEvent, EventId, MemoryNote, Provenance, RunId, TaskLink, TermSize,
        TerminalPurpose, WorkspaceTerminal, MAX_CURSOR_LEN, MAX_LABEL, MAX_LINK, MAX_NOTE,
        MAX_TASKS, MAX_TERMINALS, MAX_TEXT,
    };

    let id = "i".repeat(remote_proto::ids::MAX_ID_LEN);
    let text = |chars: usize| "\u{1}".repeat(chars);
    let code = || Some("c".repeat(MAX_CODE));
    // Activity names exclude control characters; a four-byte character is their largest.
    let name = || Some("\u{1f600}".repeat(MAX_NAME));
    let cursor = || Some(Cursor::new("c".repeat(MAX_CURSOR_LEN)).unwrap());
    let terminal = || TerminalSummary {
        id: TerminalId::new(&id).unwrap(),
        activity: TerminalActivity::Exited,
        idle_ms: u32::MAX,
        size: TermSize {
            cols: u16::MAX,
            rows: u16::MAX,
        },
        exit_code: Some(u32::MAX),
    };
    let project = ProjectSummary {
        id: ProjectId::new(&id).unwrap(),
        name: text(MAX_TEXT),
        missing: true,
    };
    let workspace = WorkspaceSummary {
        id: WorkspaceId::new(&id).unwrap(),
        project: ProjectId::new(&id).unwrap(),
        name: text(MAX_TEXT),
        kind: WorkspaceKind::Worktree,
        branch: Some(BranchRef {
            label: text(MAX_TEXT),
            detached: true,
            unborn: true,
        }),
        archived: true,
        missing: true,
        tasks: vec![
            TaskLink {
                source: text(MAX_LABEL),
                key: text(MAX_TEXT),
                title: text(MAX_TEXT),
                // Links hold no control characters; a quote is their largest.
                url: "\"".repeat(MAX_LINK),
            };
            MAX_TASKS
        ],
        running_sessions: u32::MAX,
    };
    let session = SessionSummary {
        id: SessionRecordId::new(&id).unwrap(),
        workspace: WorkspaceId::new(&id).unwrap(),
        harness: text(MAX_LABEL),
        model: Some(text(MAX_LABEL)),
        effort: Some(text(MAX_LABEL)),
        title: text(MAX_TEXT),
        forked_from: Some(SessionRecordId::new(&id).unwrap()),
        started_at: i64::MIN,
        ended_at: Some(i64::MIN),
        exit_code: Some(i64::MIN),
        process: ProcessState::Unknown,
        terminal: Some(terminal()),
    };
    let other = WorkspaceTerminal {
        purpose: TerminalPurpose::RunCommand,
        program: text(MAX_LABEL),
        harness: Some(text(MAX_LABEL)),
        terminal: terminal(),
    };
    let note = MemoryNote {
        id: MemoryId::new(&id).unwrap(),
        project: ProjectId::new(&id).unwrap(),
        text: text(MAX_NOTE),
        author: text(MAX_LABEL),
        harness: Some(text(MAX_LABEL)),
        created_at: i64::MIN,
        updated_at: i64::MIN,
    };
    let event = ActivityEvent {
        id: EventId::new(&id).unwrap(),
        workspace: WorkspaceId::new(&id).unwrap(),
        session: Some(SessionRecordId::new(&id).unwrap()),
        run: Some(RunId::new(&id).unwrap()),
        occurred_at: i64::MIN,
        kind: "c".repeat(MAX_CODE),
        source: Provenance {
            producer: "c".repeat(MAX_CODE),
            method: "c".repeat(MAX_CODE),
            fidelity: "c".repeat(MAX_CODE),
        },
        detail: ActivityDetail {
            harness: name(),
            run_kind: code(),
            program: name(),
            model: name(),
            continuation: code(),
            launched_by: code(),
            capture: code(),
            forked_from: Some(SessionRecordId::new(&id).unwrap()),
            exit_code: Some(i64::MIN),
            success: Some(false),
            signal: code(),
            via: code(),
            reason: code(),
            tool: name(),
            subagent_type: name(),
            // Paths hold no control characters; a quote is their largest.
            path: Some("\"".repeat(MAX_PATH)),
            path_outside_workspace: Some(false),
            change: code(),
            status: code(),
            duration_ms: Some(u64::MAX),
            chars: Some(u64::MAX),
            decision: code(),
            wait_ms: Some(u64::MAX),
            notification: code(),
            error_type: code(),
            agent_type: name(),
            trigger: code(),
            outcome: code(),
            turn_number: Some(u64::MAX),
            context_tokens: Some(u64::MAX),
            context_window: Some(u64::MAX),
            total_tokens: Some(u64::MAX),
            output_tokens: Some(u64::MAX),
            note: code(),
        },
    };

    let pages = [
        Reply::Workspaces(WorkspacePage {
            projects: vec![project.clone()],
            workspaces: vec![workspace.clone()],
            next: cursor(),
        }),
        Reply::Workspace(WorkspaceDetail { workspace, project }),
        Reply::Sessions(SessionPage {
            sessions: vec![session],
            terminals: vec![other; MAX_TERMINALS],
            next: cursor(),
        }),
        Reply::Memory(MemoryPage {
            notes: vec![note],
            next: cursor(),
        }),
        Reply::Activity(ActivityPage {
            events: vec![event],
            older: cursor(),
            newer: cursor(),
        }),
        Reply::Host(HostDescription {
            host: HostId::new(&id).unwrap(),
            name: text(MAX_TEXT),
            platform: Platform::current(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol: ProtocolVersion::CURRENT,
            capabilities: ANSWERED.iter().map(|&op| op.to_owned()).collect(),
        }),
    ];
    for reply in pages {
        // What `fitting` measures: the page alone.
        let page = match &reply {
            Reply::Workspaces(page) => encoded_len(page),
            Reply::Sessions(page) => encoded_len(page),
            Reply::Memory(page) => encoded_len(page),
            Reply::Activity(page) => encoded_len(page),
            other => encoded_len(other),
        };
        assert!(page <= PAGE_BUDGET, "{page} bytes: {reply:?}");
        // And what goes on the wire, with the longest request id.
        let wire = remote_proto::encode_response(&Response {
            request_id: Some(RequestId::new(&id).unwrap()),
            outcome: Outcome::Result(reply),
        });
        assert!(wire.len() <= remote_proto::MAX_CONTROL_FRAME);
    }
}

#[test]
fn text_longer_than_the_protocol_allows_is_clipped_on_the_way_out() {
    let f = Fixture::new();
    let long = "a long title ".repeat(1_000);
    f.store
        .add_session(&NewSession {
            id: "rec-long",
            workspace_id: &f.alpha.worktree,
            harness_id: "claude",
            model: Some(&long),
            title: &long,
            pty_session_id: "pty-long",
            ..Default::default()
        })
        .unwrap();
    let page = sessions(&f.facade(None), &everything(), &f.alpha.worktree, true);
    let session = &page.sessions[0];
    assert_eq!(session.title.chars().count(), remote_proto::MAX_TEXT);
    assert!(session.title.ends_with('…'));
    assert!(long.starts_with(session.title.trim_end_matches('…')));
    assert_eq!(
        session.model.as_deref().map(|m| m.chars().count()),
        Some(remote_proto::MAX_LABEL)
    );
}

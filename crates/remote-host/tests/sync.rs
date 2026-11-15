//! The sync loop and the person's acts against a stand-in server on a real socket, with a real
//! store: a stop waits for a create on its way, so nothing is left linked at the server that
//! this host forgot; a link the server has and this host does not is offered, and is never
//! undone by a link that failed here; a project reaching the server, and a pass going through
//! after one failed, both tell the window; a link sends this host's own notes in the same
//! pass; and a workspace another process records, as `ys` does, goes up within seconds.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use remote_host::account::{Home, Remote, Request};
use remote_host::sync;
use remote_proto::account::{Credentials, Secret};
use remote_proto::sync::{
    Action, OpResult, PushReply, PushRequest, RecordKind, SnapshotPage, SyncId, SyncPage,
    SyncProject, SyncProjects, SyncRecord,
};
use remote_proto::{DeviceId, ErrorCode, HostId, RemoteError};
use serde_json::json;
use tokio::sync::Notify;
use yardsort_core::remote::host::{refresh_entry, CredentialStore, MemoryCredentials};
use yardsort_core::remote::serve::Context;
use yardsort_core::settings::SettingsFile;
use yardsort_core::store::{RemoteHostRow, Store};

const SOON: Duration = Duration::from_secs(10);
const DEVICE: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const HOST: &str = "7f3c1a2b-0d4e-4f5a-8b6c-1d2e3f4a5b6c";
const ELSEWHERE: &str = "9e8d7c6b-5a49-4382-b716-0f1e2d3c4b5a";
const SYNCED: &str = "4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e";

/// What the stand-in server holds: one account's projects, whether this host is linked to each,
/// and what it was asked, in order.
#[derive(Default)]
struct Account {
    records: Vec<SyncRecord>,
    linked: Vec<SyncId>,
    asked: Vec<String>,
    /// When set, a push waits for it before it is answered.
    hold_push: Option<Arc<Notify>>,
    /// Told when a push arrives.
    pushed: Arc<Notify>,
    /// Pulls to fail before one goes through.
    failing_pulls: usize,
    /// The notes pushed, by their text.
    notes: Vec<String>,
    /// The workspaces pushed, by their name.
    workspaces: Vec<String>,
}

type Server = Arc<Mutex<Account>>;

fn record(id: &SyncId, revision: u64, name: &str, host: &str) -> SyncRecord {
    SyncRecord {
        kind: RecordKind::Project,
        id: id.clone(),
        project: id.clone(),
        revision,
        schema: 1,
        body: Some(json!({"name": name, "remote": "https://github.com/acme/widgets.git"})),
        revoked: false,
        deleted: false,
        host: HostId::new(host).unwrap(),
        changed_at: 1,
        cursor: revision,
    }
}

fn not_found() -> Response {
    let error = RemoteError::new(ErrorCode::NotFound, "There is nothing by that id here.");
    (StatusCode::NOT_FOUND, Json(error)).into_response()
}

async fn serve(server: Server) -> String {
    let app = Router::new()
        .route("/v1/token", post(token))
        .route("/v1/sync/projects", get(projects))
        .route("/v1/sync/projects/{id}/{verb}", post(link))
        .route("/v1/sync/push", post(push))
        .route("/v1/sync/pull", get(pull))
        .route("/v1/sync/snapshot", get(snapshot))
        .with_state(server);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

async fn token() -> Json<Credentials> {
    Json(Credentials {
        device: DeviceId::new(DEVICE).unwrap(),
        access: Secret::new("access"),
        access_expires_in_ms: 3_600_000,
        refresh: Secret::new("refresh"),
    })
}

async fn projects(State(server): State<Server>) -> Json<SyncProjects> {
    let account = server.lock().unwrap();
    Json(SyncProjects {
        projects: account
            .records
            .iter()
            .filter(|r| !r.deleted)
            .map(|r| SyncProject {
                record: r.clone(),
                linked: account.linked.contains(&r.id),
            })
            .collect(),
    })
}

async fn link(State(server): State<Server>, Path((id, verb)): Path<(String, String)>) -> Response {
    let mut account = server.lock().unwrap();
    let id = SyncId::new(id).unwrap();
    account.asked.push(verb.clone());
    if !account.records.iter().any(|r| r.id == id) {
        return not_found();
    }
    account.linked.retain(|l| *l != id);
    if verb == "link" {
        account.linked.push(id);
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn push(State(server): State<Server>, Json(request): Json<PushRequest>) -> Json<PushReply> {
    let (pushed, hold) = {
        let mut account = server.lock().unwrap();
        account.asked.push("push".into());
        (account.pushed.clone(), account.hold_push.clone())
    };
    pushed.notify_one();
    if let Some(hold) = hold {
        hold.notified().await;
    }
    let mut account = server.lock().unwrap();
    let results = request
        .ops
        .iter()
        .map(|op| {
            assert_eq!(op.action, Action::Put, "only creates in these tests");
            if op.kind == RecordKind::Note {
                let body = op.body.as_ref().unwrap();
                account
                    .notes
                    .push(body["text"].as_str().unwrap().to_owned());
            } else if op.kind == RecordKind::Workspace {
                let body = op.body.as_ref().unwrap();
                account
                    .workspaces
                    .push(body["name"].as_str().unwrap().to_owned());
            } else {
                // A create links the host that made it.
                account.records.push(record(&op.id, 1, "widgets", HOST));
                account.linked.push(op.id.clone());
            }
            OpResult::Applied {
                op: op.op.clone(),
                revision: 1,
                cursor: 1,
            }
        })
        .collect();
    Json(PushReply { results })
}

async fn pull(State(server): State<Server>, RawQuery(query): RawQuery) -> Response {
    let mut account = server.lock().unwrap();
    if account.failing_pulls > 0 {
        account.failing_pulls -= 1;
        let error = RemoteError::new(ErrorCode::Internal, "The server is having a moment.");
        return (StatusCode::SERVICE_UNAVAILABLE, Json(error)).into_response();
    }
    let after: u64 = query
        .as_deref()
        .and_then(|q| q.strip_prefix("after="))
        .unwrap()
        .parse()
        .unwrap();
    Json(SyncPage {
        records: Vec::new(),
        cursor: after,
        more: false,
    })
    .into_response()
}

async fn snapshot(State(server): State<Server>) -> Json<SnapshotPage> {
    let account = server.lock().unwrap();
    Json(SnapshotPage {
        records: account
            .records
            .iter()
            .filter(|r| account.linked.contains(&r.id))
            .cloned()
            .collect(),
        cursor: 1,
        next: None,
    })
}

struct Kept(Arc<MemoryCredentials>);

impl CredentialStore for Kept {
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

/// A host with Remote on at the stand-in, signed in, and counting what it tells the window.
struct Here {
    remote: Remote,
    store: Store,
    settings: SettingsFile,
    credentials: Arc<MemoryCredentials>,
    told: AtomicUsize,
    _dir: tempfile::TempDir,
}

impl Home for Here {
    fn remote(&self) -> &Remote {
        &self.remote
    }
    fn store(&self) -> &Store {
        &self.store
    }
    fn settings(&self) -> &SettingsFile {
        &self.settings
    }
    fn credentials(&self) -> remote_host::account::Result<Box<dyn CredentialStore>> {
        Ok(Box::new(Kept(self.credentials.clone())))
    }
    fn serve(&self, _f: &mut dyn FnMut(&Context<'_>)) {
        unreachable!("no device asks in these tests")
    }
    fn asked(&self, _request: Request) {}
    fn synced(&self) {
        self.told.fetch_add(1, Ordering::SeqCst);
    }
}

async fn here() -> (Arc<Here>, Server) {
    let server = Server::default();
    let base = serve(server.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    store
        .set_remote_host(&RemoteHostRow {
            server_url: base,
            server_id: "server".into(),
            host_id: HOST.into(),
            device_id: DEVICE.into(),
            name: "Here".into(),
            enabled_at: 1,
        })
        .unwrap();
    let credentials = Arc::new(MemoryCredentials::default());
    credentials
        .set(&refresh_entry(DEVICE), &Secret::new("refresh"))
        .unwrap();
    let home = Here {
        remote: Remote::default(),
        store,
        settings: SettingsFile::load(dir.path().join("settings.toml")),
        credentials,
        told: AtomicUsize::new(0),
        _dir: dir,
    };
    (Arc::new(home), server)
}

/// A project another host turned on, which this host is not linked to.
fn synced_elsewhere(server: &Server) -> SyncId {
    let id = SyncId::new(SYNCED).unwrap();
    server
        .lock()
        .unwrap()
        .records
        .push(record(&id, 1, "widgets", ELSEWHERE));
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_waits_for_a_create_on_its_way_and_leaves_nothing_linked() {
    let (home, server) = here().await;
    let project = home.store.add_project("widgets", "/src/widgets").unwrap();
    let hold = Arc::new(Notify::new());
    let pushed = {
        let mut account = server.lock().unwrap();
        account.hold_push = Some(hold.clone());
        account.pushed.clone()
    };
    sync::turn_on(home.as_ref(), &project.id, None).unwrap();

    let passing = tokio::spawn({
        let home = home.clone();
        async move { sync::pass(home.as_ref()).await }
    });
    tokio::time::timeout(SOON, pushed.notified()).await.unwrap();
    let stopping = tokio::spawn({
        let home = home.clone();
        let id = project.id.clone();
        async move { sync::stop(home.as_ref(), &id).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !stopping.is_finished(),
        "the stop went ahead while the create was out"
    );

    hold.notify_one();
    passing.await.unwrap().unwrap();
    stopping.await.unwrap().unwrap();
    let account = server.lock().unwrap();
    assert_eq!(account.asked, ["push", "unlink"]);
    assert!(
        account.linked.is_empty(),
        "left linked: {:?}",
        account.linked
    );
    assert!(home.store.synced_project(&project.id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_the_server_has_and_this_host_lost_is_offered_and_kept() {
    let (home, server) = here().await;
    let id = synced_elsewhere(&server);
    // Linked at the server, as when the answer to a link never came back.
    server.lock().unwrap().linked.push(id.clone());

    let offered = sync::offered(home.as_ref()).await.unwrap();
    assert_eq!(
        offered.iter().map(|o| &o.id).collect::<Vec<_>>(),
        [&id],
        "the lost link is offered again"
    );

    // A link that fails here leaves the server's link as it was.
    let error = sync::link(home.as_ref(), "no-such-project", &id)
        .await
        .unwrap_err();
    assert_eq!(error.message, "There is no such project.");
    assert_eq!(server.lock().unwrap().linked, std::slice::from_ref(&id));
    assert!(!server.lock().unwrap().asked.contains(&"unlink".into()));

    let project = home.store.add_project("widgets", "/src/widgets").unwrap();
    sync::link(home.as_ref(), &project.id, &id).await.unwrap();
    let row = home.store.synced_project(&project.id).unwrap().unwrap();
    assert_eq!(row.sync_id, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_clone_linked_to_the_same_project_is_refused_and_the_first_kept() {
    let (home, server) = here().await;
    let id = synced_elsewhere(&server);
    let first = home.store.add_project("widgets", "/src/widgets").unwrap();
    let second = home
        .store
        .add_project("widgets-copy", "/src/widgets-copy")
        .unwrap();

    sync::link(home.as_ref(), &first.id, &id).await.unwrap();
    let error = sync::link(home.as_ref(), &second.id, &id)
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("linked to that synced project already"),
        "{error:?}"
    );

    assert_eq!(server.lock().unwrap().linked, std::slice::from_ref(&id));
    assert_eq!(server.lock().unwrap().asked, ["link"]);
    sync::pass(home.as_ref()).await.unwrap();
    assert_eq!(
        home.store
            .synced_project(&first.id)
            .unwrap()
            .unwrap()
            .sync_id,
        id
    );
    assert!(home.store.synced_project(&second.id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn reaching_the_server_and_recovering_from_a_failed_pass_both_tell_the_window() {
    let (home, server) = here().await;
    let project = home.store.add_project("widgets", "/src/widgets").unwrap();
    sync::turn_on(home.as_ref(), &project.id, None).unwrap();
    assert!(
        sync::pass(home.as_ref()).await.unwrap(),
        "reaching the server is a change the window shows"
    );
    assert!(home
        .store
        .synced_project(&project.id)
        .unwrap()
        .unwrap()
        .revision
        .is_some());

    // The loop: a pass that fails, then one that goes through with nothing new.
    server.lock().unwrap().failing_pulls = 1;
    let syncer = sync::Syncer::start(home.clone());
    let deadline = tokio::time::Instant::now() + SOON;
    while home.told.load(Ordering::SeqCst) < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "told {} times",
            home.told.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(home.store.sync_state().unwrap().problem, None);
    assert_eq!(syncer.state(), sync::Loop::Idle);
    syncer.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_sends_this_hosts_own_notes_in_the_same_pass() {
    let (home, server) = here().await;
    let id = synced_elsewhere(&server);
    let project = home.store.add_project("widgets", "/src/widgets").unwrap();
    yardsort_core::memory::write(&home.store, &project.id, "The tests need TZ=UTC.").unwrap();
    assert_eq!(
        home.store.sync_waiting().unwrap(),
        0,
        "nothing syncs before the link"
    );

    sync::link(home.as_ref(), &project.id, &id).await.unwrap();
    sync::pass(home.as_ref()).await.unwrap();
    let account = server.lock().unwrap();
    assert_eq!(account.asked, ["link", "push"]);
    assert_eq!(account.notes, ["The tests need TZ=UTC."]);
    assert_eq!(home.store.sync_waiting().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_another_process_records_goes_up_within_seconds() {
    let (home, server) = here().await;
    let project = home.store.add_project("widgets", "/src/widgets").unwrap();
    sync::turn_on(home.as_ref(), &project.id, None).unwrap();
    sync::pass(home.as_ref()).await.unwrap();
    let syncer = sync::Syncer::start(home.clone());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(syncer.state(), sync::Loop::Idle);

    // As `ys workspace new` would, in a process of its own: straight into the database, with
    // nobody waking the loop.
    let other = Store::open(&home._dir.path().join("yardsort.db")).unwrap();
    other
        .add_worktree(
            &project.id,
            "fix-login",
            "/wt/fix-login",
            Some("ys/fix-login"),
            None,
        )
        .unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    while server.lock().unwrap().workspaces.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "never went up");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(server.lock().unwrap().workspaces, ["fix-login"]);
    syncer.stop().await;
}

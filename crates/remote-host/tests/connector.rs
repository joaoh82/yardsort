//! The connector against a stand-in relay on a real socket, a real store and programs in real
//! PTYs: devices come through the link, a failed link lets go of everything they held while the
//! programs run on, and the connector comes back by itself.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use pty_host::{Controller, LaunchPlan, PtyHost, SessionState};
use remote_client::Client;
use remote_host::{Access, AccessFuture, Connector, Serving, Status};
use remote_proto::account::Secret;
use remote_proto::relay::{ChannelOpen, CloseReason, Greeting, LinkFrame};
use remote_proto::{DeviceId, GrantId, HostId, ProjectGrant, Scope};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use yardsort_core::git::Git;
use yardsort_core::remote::host::keep_approved;
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::store::Store;

const SOON: Duration = Duration::from_secs(10);

/// A relay that issues any ticket and hands each host socket to the test once greeted.
struct FakeRelay {
    base: String,
    sockets: mpsc::UnboundedReceiver<WebSocket>,
}

impl FakeRelay {
    async fn start() -> Self {
        let (to_test, sockets) = mpsc::unbounded_channel();
        let app = Router::new()
            .route(
                "/v1/relay/tickets",
                post(|| async { Json(json!({"ticket": "ys1.rl.fake", "expiresInMs": 60000})) }),
            )
            .route("/v1/relay", get(upgrade))
            .with_state(to_test);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, sockets }
    }

    async fn next_socket(&mut self) -> WebSocket {
        tokio::time::timeout(SOON, self.sockets.recv())
            .await
            .expect("the host did not connect")
            .unwrap()
    }
}

async fn upgrade(
    State(to_test): State<mpsc::UnboundedSender<WebSocket>>,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    upgrade.on_upgrade(move |mut socket| async move {
        let Some(Ok(Message::Text(_ticket))) = socket.next().await else {
            return;
        };
        let greeting = Greeting::Connected {
            host: HostId::new("host-test").unwrap(),
        };
        let text = serde_json::to_string(&greeting).unwrap();
        socket.send(Message::Text(text.into())).await.unwrap();
        let _ = to_test.send(socket);
    })
}

/// What a link frame from the host says, owned.
#[derive(Debug)]
enum Heard {
    Close(u32, CloseReason),
    Text(u32, Value),
    Binary(u32),
}

async fn heard(socket: &mut WebSocket) -> Heard {
    loop {
        let message = tokio::time::timeout(SOON, socket.next())
            .await
            .expect("the host said nothing")
            .expect("the link closed")
            .unwrap();
        let Message::Binary(bytes) = message else {
            continue;
        };
        return match LinkFrame::decode(&bytes).unwrap() {
            LinkFrame::Close(c, reason) => Heard::Close(c, reason),
            LinkFrame::Text(c, text) => Heard::Text(c, serde_json::from_slice(text).unwrap()),
            LinkFrame::Binary(c, _) => Heard::Binary(c),
            LinkFrame::Open(..) => panic!("a host never opens a channel"),
        };
    }
}

async fn text(socket: &mut WebSocket) -> (u32, Value) {
    loop {
        match heard(socket).await {
            Heard::Text(c, value) => return (c, value),
            Heard::Binary(_) => {}
            Heard::Close(c, reason) => panic!("channel {c} closed: {reason:?}"),
        }
    }
}

async fn send(socket: &mut WebSocket, frame: LinkFrame<'_>) {
    socket
        .send(Message::Binary(frame.encode().into()))
        .await
        .unwrap();
}

async fn say(socket: &mut WebSocket, channel: u32, value: Value) {
    send(
        socket,
        LinkFrame::Text(channel, value.to_string().as_bytes()),
    )
    .await;
}

fn request(id: &str, op: &str, params: Value) -> Value {
    json!({"requestId": id, "protocol": {"major": 1, "minor": 1}, "op": op, "params": params})
}

struct Credentials(String);

impl Access for Credentials {
    fn access(&self) -> AccessFuture<'_> {
        let client = Client::new(&self.0).unwrap();
        Box::pin(async move { Ok((client, Secret::new("ys1.at.fake"))) })
    }
}

/// This host: a store with one workspace, a phone let in, and terminals.
struct Here {
    _dir: tempfile::TempDir,
    store: Store,
    git: Git,
    terminals: Arc<PtyHost>,
    identity: HostIdentity,
    workspace: String,
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

fn here() -> Arc<Here> {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("alpha", "/code/alpha").unwrap();
    let workspace = store
        .add_worktree(&project.id, "work", "/code/alpha-wt", None, None)
        .unwrap();
    keep_approved(
        &store,
        &GrantId::new("g-phone").unwrap(),
        &DeviceId::new("device-phone").unwrap(),
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
    Arc::new(Here {
        _dir: dir,
        store,
        git: yardsort_core::git::testing::git(),
        terminals: Arc::new(PtyHost::new(Arc::new(|_| {}))),
        identity: HostIdentity {
            id: HostId::new("host-test").unwrap(),
            name: "Test host".into(),
        },
        workspace: workspace.id,
    })
}

fn long_running(workspace: &str) -> LaunchPlan {
    let (program, flag, script) = if cfg!(windows) {
        ("cmd.exe", "/C", "ping -n 60 127.0.0.1")
    } else {
        ("/bin/sh", "-c", "sleep 60")
    };
    LaunchPlan {
        program: program.into(),
        args: vec![flag.into(), script.into()],
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

fn open(channel: u32, grant: &str) -> LinkFrame<'static> {
    LinkFrame::Open(
        channel,
        ChannelOpen {
            device: DeviceId::new("device-phone").unwrap(),
            grant: GrantId::new(grant).unwrap(),
        },
    )
}

fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < SOON, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_come_through_the_link_and_a_failed_link_leaves_the_programs_running() {
    let mut relay = FakeRelay::start().await;
    let here = here();
    let terminal = here.terminals.spawn(long_running(&here.workspace)).unwrap();
    let connector = Connector::start(
        HostId::new("host-test").unwrap(),
        Arc::new(Credentials(relay.base.clone())),
        Arc::clone(&here) as Arc<dyn Serving>,
    );
    let mut socket = relay.next_socket().await;

    // A phone comes through: greeted, it takes control and watches.
    send(&mut socket, open(1, "g-phone")).await;
    say(
        &mut socket,
        1,
        json!({"client": "test-phone/1", "protocols": [1]}),
    )
    .await;
    let (channel, welcome) = text(&mut socket).await;
    assert_eq!(channel, 1);
    assert!(welcome["welcome"]["host"]["capabilities"]
        .to_string()
        .contains("terminal.acquire"));
    assert_eq!(connector.status(), Status::Connected { devices: 1 });
    let id = terminal.id.0.clone();
    say(
        &mut socket,
        1,
        request("r1", "terminal.acquire", json!({"terminal": id})),
    )
    .await;
    let (_, lease) = text(&mut socket).await;
    assert_eq!(lease["result"]["kind"], "lease", "{lease}");
    say(
        &mut socket,
        1,
        request("r2", "terminal.attach", json!({"terminal": id})),
    )
    .await;
    let (_, attached) = text(&mut socket).await;
    assert_eq!(attached["result"]["kind"], "attached", "{attached}");
    assert!(matches!(heard(&mut socket).await, Heard::Binary(1)));
    assert!(matches!(
        here.terminals.controller(&terminal.id).unwrap(),
        Controller::Leased { .. }
    ));

    // The relay goes. Nothing the phone held is held any more; the program runs on, and the
    // person here can type into it.
    drop(socket);
    let terminals = Arc::clone(&here.terminals);
    let session = terminal.id.clone();
    tokio::task::spawn_blocking(move || {
        until("the lease is let go", || {
            terminals.controller(&session).unwrap() == Controller::Nobody
        });
        assert!(matches!(
            terminals.info(&session).unwrap().state,
            SessionState::Running
        ));
        terminals.write(&session, b"still mine\r").unwrap();
    })
    .await
    .unwrap();

    // The connector comes back by itself.
    let mut socket = relay.next_socket().await;
    // A grant this host never approved is refused here, whatever the relay says.
    send(&mut socket, open(2, "g-unknown")).await;
    assert!(matches!(
        heard(&mut socket).await,
        Heard::Close(2, CloseReason::Refused)
    ));

    // Revoked here: the channel is closed without waiting for the server.
    send(&mut socket, open(3, "g-phone")).await;
    say(
        &mut socket,
        3,
        json!({"client": "test-phone/1", "protocols": [1]}),
    )
    .await;
    let (_, _welcome) = text(&mut socket).await;
    here.store.drop_remote_grant("g-phone").unwrap();
    connector.recheck();
    assert!(matches!(
        heard(&mut socket).await,
        Heard::Close(3, CloseReason::Revoked)
    ));

    connector.stop().await;
    let closed = tokio::time::timeout(SOON, async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                return true;
            }
        }
        true
    })
    .await;
    assert!(closed.is_ok(), "the link stayed open after stopping");
}

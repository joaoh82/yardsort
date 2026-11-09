//! The channel against a relay and a host played by the test, over a real WebSocket on this
//! machine: what the host answers comes from the fixtures in `remote-proto/fixtures/v1/`, the
//! same ones the TypeScript client is pinned by, and what it does wrong it does on purpose.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use remote_client::channel::{AttachmentEnd, ControlLoss, EndReason, Followed, Watched};
use remote_client::{Channel, ChannelError, Client};
use remote_proto::account::Secret;
use remote_proto::relay::Greeting;
use remote_proto::{
    ActivitySubscribeParams, ErrorCode, HostId, OutputFrame, StreamPosition, TerminalId,
    WorkspaceListParams, MAX_OUTPUT_CHUNK,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const SOON: Duration = Duration::from_secs(10);

/// A relay that issues any ticket and hands each device's socket to the test, as the host.
struct Relay {
    client: Client,
    sockets: mpsc::UnboundedReceiver<WebSocket>,
}

impl Relay {
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
        Self {
            client: Client::new(&base).unwrap(),
            sockets,
        }
    }

    /// A channel, and the host's end of it, greeted and welcomed.
    async fn open(&mut self) -> (Channel, Host) {
        let (access, id) = (
            Secret::new("access".to_owned()),
            HostId::new("host-example").unwrap(),
        );
        let opening = Channel::open(&self.client, &access, &id, "channel-test/1");
        let host = async {
            let mut host = Host(
                tokio::time::timeout(SOON, self.sockets.recv())
                    .await
                    .expect("the device did not connect")
                    .unwrap(),
            );
            let hello = host.text().await;
            assert_eq!(hello, json!({"client": "channel-test/1", "protocols": [1]}));
            host.send(fixture("hello-welcome")).await;
            host
        };
        let (channel, host) = tokio::join!(opening, host);
        (channel.unwrap(), host)
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
            host: HostId::new("host-example").unwrap(),
        };
        let text = serde_json::to_string(&greeting).unwrap();
        socket.send(Message::Text(text.into())).await.unwrap();
        let _ = to_test.send(socket);
    })
}

/// The host's end of a device's socket.
struct Host(WebSocket);

impl Host {
    async fn text(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(SOON, self.0.next())
                .await
                .expect("the device said nothing")
                .expect("the socket closed")
                .unwrap();
            if let Message::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    /// The next request, and its id.
    async fn request(&mut self, op: &str) -> (String, Value) {
        let request = self.text().await;
        assert_eq!(request["op"], op, "{request}");
        assert_eq!(request["protocol"]["major"], 1);
        (request["requestId"].as_str().unwrap().to_owned(), request)
    }

    async fn send(&mut self, value: Value) {
        self.0
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn raw(&mut self, text: &str) {
        self.0.send(Message::Text(text.into())).await.unwrap();
    }

    /// A fixture's answer, to the request `id`.
    async fn answer(&mut self, id: &str, name: &str) {
        let mut response = fixture(name);
        response["requestId"] = id.into();
        self.send(response).await;
    }

    async fn frame(&mut self, stream: u32, epoch: u32, seq: u64, bytes: &[u8]) {
        let mut out = Vec::new();
        OutputFrame {
            stream,
            position: StreamPosition { epoch, seq },
            bytes,
        }
        .encode(&mut out)
        .unwrap();
        self.0.send(Message::Binary(out.into())).await.unwrap();
    }
}

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../remote-proto/fixtures/v1/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn terminal() -> TerminalId {
    TerminalId::new("terminal-1").unwrap()
}

async fn soon<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(SOON, future)
        .await
        .expect("nothing came in time")
}

#[tokio::test]
async fn each_answer_reaches_its_own_question_whatever_order_they_come_in() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    assert_eq!(channel.host().name, "Example workstation");

    let asking = async {
        tokio::join!(
            channel.workspaces(WorkspaceListParams::default()),
            channel.describe()
        )
    };
    let answering = async {
        let (first, _) = host.request("workspace.list").await;
        let (second, _) = host.request("host.describe").await;
        assert_ne!(first, second);
        host.answer(&second, "response-host").await;
        host.answer(&first, "response-workspaces").await;
    };
    let ((workspaces, described), ()) = soon(async { tokio::join!(asking, answering) }).await;
    assert_eq!(workspaces.unwrap().workspaces[0].name, "fix-login");
    assert!(described.is_ok());
}

#[tokio::test]
async fn a_refusal_comes_back_with_the_hosts_reason() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let asking = channel.describe();
    let answering = async {
        let (id, _) = host.request("host.describe").await;
        host.send(json!({"requestId": id, "error": {"code": "forbidden", "message": "Not for you.", "retryable": false}}))
            .await;
    };
    let (refused, ()) = soon(async { tokio::join!(asking, answering) }).await;
    let error = refused.unwrap_err();
    assert_eq!(error.code(), Some(ErrorCode::Forbidden));
    assert_eq!(error.to_string(), "Not for you.");
}

#[tokio::test]
async fn a_terminal_is_drawn_in_order_a_split_piece_included_and_ends_at_the_first_gap() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let attaching = channel.attach(terminal(), None);
    let answering = async {
        let (id, request) = host.request("terminal.attach").await;
        assert_eq!(request["params"]["terminal"], "terminal-1");
        host.answer(&id, "response-attached").await;
        // The snapshot at 41, in two frames: the first full, so more of it may come.
        host.frame(1, 1, 41, &vec![b'x'; MAX_OUTPUT_CHUNK]).await;
    };
    let (attached, ()) = soon(async { tokio::join!(attaching, answering) }).await;
    let mut attachment = attached.unwrap();
    let Some(Watched::Output { position, .. }) = soon(attachment.next()).await else {
        panic!("no snapshot");
    };
    assert_eq!(position.seq, 41);
    assert_eq!(
        attachment.drawn(),
        None,
        "a full frame may have more to come"
    );

    host.frame(1, 1, 41, b"end of the snapshot").await;
    host.send(json!({"event": "terminal.resized", "stream": 1, "position": {"epoch": 1, "seq": 42}, "size": {"cols": 80, "rows": 24}}))
        .await;
    host.frame(1, 1, 43, b"next").await;
    // Not this stream's: nothing is drawn of it.
    host.frame(2, 1, 1, b"another stream").await;
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(soon(attachment.next()).await.unwrap());
    }
    assert!(matches!(&seen[0], Watched::Output { bytes, .. } if bytes == b"end of the snapshot"));
    assert!(matches!(&seen[1], Watched::Resized { size, .. } if size.cols == 80));
    assert!(matches!(&seen[2], Watched::Output { bytes, .. } if bytes == b"next"));
    assert_eq!(
        attachment.drawn(),
        Some(StreamPosition { epoch: 1, seq: 43 })
    );

    // 44 never came.
    host.frame(1, 1, 45, b"too far").await;
    let ended = soon(attachment.next()).await.unwrap();
    assert_eq!(
        ended,
        Watched::Ended(AttachmentEnd::Gap {
            expected: StreamPosition { epoch: 1, seq: 44 },
            got: StreamPosition { epoch: 1, seq: 45 },
        })
    );
    // And the host is told to stop sending it.
    let (_, detach) = host.request("terminal.detach").await;
    assert_eq!(
        detach["params"],
        json!({"terminal": "terminal-1", "stream": 1})
    );
}

#[tokio::test]
async fn what_a_newer_host_sends_is_passed_over_and_what_no_host_sends_ends_the_channel() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    host.send(json!({"event": "terminal.someday", "stream": 1}))
        .await;
    // A frame of a kind this build does not know.
    host.0
        .send(Message::Binary(vec![9; 24].into()))
        .await
        .unwrap();
    let asking = channel.describe();
    let answering = async {
        let (id, _) = host.request("host.describe").await;
        host.answer(&id, "response-host").await;
    };
    let (described, ()) = soon(async { tokio::join!(asking, answering) }).await;
    assert!(described.is_ok(), "{described:?}");
    assert!(channel.is_open());

    let asking = channel.describe();
    let breaking = async {
        host.request("host.describe").await;
        host.raw("this is not JSON").await;
    };
    let (unanswered, ()) = soon(async { tokio::join!(asking, breaking) }).await;
    assert!(
        matches!(unanswered, Err(ChannelError::Closed(_))),
        "{unanswered:?}"
    );
    let end = soon(channel.ended()).await;
    assert_eq!(end.reason, EndReason::Invalid);
    assert_eq!(end.text, "the computer sent a message that is not JSON");
}

#[tokio::test]
async fn input_is_numbered_within_the_lease_and_a_lost_lease_loses_control() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let acquiring = channel.acquire(terminal(), false);
    let answering = async {
        let (id, request) = host.request("terminal.acquire").await;
        assert_eq!(request["params"]["takeOver"], false);
        host.send(json!({"requestId": id, "result": {"kind": "lease", "id": "lease-1", "generation": 3, "ttlMs": 30000, "renewEveryMs": 10000}}))
            .await;
    };
    let (control, ()) = soon(async { tokio::join!(acquiring, answering) }).await;
    let control = control.unwrap();

    let typing = async {
        let first = control.input("a", false).await;
        let second = control.input("b", true).await;
        (first, second)
    };
    let answering = async {
        let (id, first) = host.request("terminal.input").await;
        assert_eq!(first["params"]["seq"], 1);
        assert_eq!(first["params"]["generation"], 3);
        host.send(json!({"requestId": id, "result": {"kind": "done"}}))
            .await;
        let (id, second) = host.request("terminal.input").await;
        assert_eq!(second["params"]["seq"], 2);
        assert_eq!(second["params"]["paste"], true);
        host.send(json!({"requestId": id, "error": {"code": "lease_lost", "message": "Someone took control.", "retryable": false}}))
            .await;
    };
    let ((first, second), ()) = soon(async { tokio::join!(typing, answering) }).await;
    assert!(first.is_ok());
    assert_eq!(second.unwrap_err().code(), Some(ErrorCode::LeaseLost));
    assert!(matches!(soon(control.lost()).await, ControlLoss::Lost(_)));
    assert!(!control.is_held());
    // Nothing more is sent under a lost lease.
    assert!(matches!(
        control.input("c", false).await,
        Err(ChannelError::Invalid(_))
    ));
}

#[tokio::test]
async fn followed_activity_arrives_on_its_subscription_and_moves_it_on() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let subscribing = channel.subscribe(ActivitySubscribeParams::default());
    let answering = async {
        let (id, _) = host.request("activity.subscribe").await;
        host.answer(&id, "response-subscribed").await;
        host.send(fixture("event-activity-events")).await;
    };
    let (subscription, ()) = soon(async { tokio::join!(subscribing, answering) }).await;
    let mut subscription = subscription.unwrap();
    assert_eq!(subscription.newer().unwrap().as_str(), "a1.n4219");
    let Some(Followed::Events(batch)) = soon(subscription.next()).await else {
        panic!("no batch");
    };
    assert_eq!(batch.events[0].kind, "permission.requested");
    assert_eq!(subscription.newer().unwrap().as_str(), "a1.n4223");
}

#[tokio::test]
async fn closing_here_ends_what_was_watched_and_says_so() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let attaching = channel.attach(terminal(), None);
    let answering = async {
        let (id, _) = host.request("terminal.attach").await;
        host.answer(&id, "response-attached").await;
    };
    let (attachment, ()) = soon(async { tokio::join!(attaching, answering) }).await;
    let mut attachment = attachment.unwrap();
    channel.close();
    assert_eq!(soon(channel.ended()).await.reason, EndReason::ClosedHere);
    assert!(matches!(
        soon(attachment.next()).await,
        Some(Watched::Ended(AttachmentEnd::Closed(_)))
    ));
}

#[tokio::test]
async fn a_lease_is_renewed_while_held_and_nothing_renews_it_once_let_go() {
    let mut relay = Relay::start().await;
    let (channel, mut host) = relay.open().await;
    let acquiring = channel.acquire(terminal(), true);
    let answering = async {
        let (id, _) = host.request("terminal.acquire").await;
        host.send(json!({"requestId": id, "result": {"kind": "lease", "id": "lease-1", "generation": 1, "ttlMs": 3000, "renewEveryMs": 1000}}))
            .await;
    };
    let (control, ()) = soon(async { tokio::join!(acquiring, answering) }).await;
    let control = control.unwrap();
    let (id, renew) = host.request("terminal.renew").await;
    assert_eq!(renew["params"]["lease"], "lease-1");
    host.send(json!({"requestId": id, "result": {"kind": "lease", "id": "lease-1", "generation": 2, "ttlMs": 3000, "renewEveryMs": 1000}}))
        .await;
    let (id, renew) = host.request("terminal.renew").await;
    assert_eq!(
        renew["params"]["generation"], 2,
        "the renewed lease is the one held"
    );
    host.send(json!({"requestId": id, "result": {"kind": "lease", "id": "lease-1", "generation": 2, "ttlMs": 3000, "renewEveryMs": 1000}}))
        .await;

    let releasing = control.release();
    let answering = async {
        let (id, _) = host.request("terminal.release").await;
        host.send(json!({"requestId": id, "result": {"kind": "done"}}))
            .await;
    };
    let (released, ()) = soon(async { tokio::join!(releasing, answering) }).await;
    assert!(released.is_ok());
    // Two renewal periods with nothing asked.
    let quiet = tokio::time::timeout(Duration::from_millis(2500), host.text()).await;
    assert!(quiet.is_err(), "still renewed after release: {quiet:?}");
}

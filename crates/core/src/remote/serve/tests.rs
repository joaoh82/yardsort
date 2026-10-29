//! A device's channel against a real store and real programs in real PTYs: what it sees, what it
//! may type, and what is left when it goes.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pty_host::{LaunchPlan, Numbered, PtyHost, SessionId, SessionInfo, SessionState};
use remote_proto::{DeviceId, GrantId, HostId, ProjectGrant, ProjectId, ProtocolVersion, Scope};
use serde_json::{json, Value};

use super::*;
use crate::git::testing::git;
use crate::launch::WORKSPACE_LABEL;

/// What a channel sent the device, in order.
#[derive(Debug, Clone)]
enum Sent {
    Text(Value),
    Output(u32, u64, Vec<u8>),
}

/// The relay's side of a channel, with a queue of `room` bytes that nothing drains.
struct Recorded {
    sent: Mutex<Vec<Sent>>,
    room: Mutex<usize>,
}

impl Recorded {
    fn new(room: usize) -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::new(Vec::new()),
            room: Mutex::new(room),
        })
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn texts(&self) -> Vec<Value> {
        self.sent()
            .into_iter()
            .filter_map(|s| match s {
                Sent::Text(v) => Some(v),
                Sent::Output(..) => None,
            })
            .collect()
    }

    /// Everything a stream has drawn, snapshot and output together.
    fn drawn(&self, stream: u32) -> String {
        self.sent()
            .into_iter()
            .filter_map(|s| match s {
                Sent::Output(s, _, bytes) if s == stream => Some(bytes),
                _ => None,
            })
            .flat_map(|b| String::from_utf8_lossy(&b).into_owned().into_bytes())
            .map(char::from)
            .collect()
    }

    fn wait_for(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}: {:?}",
                self.texts()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Outbound for Recorded {
    fn text(&self, text: String) {
        let value = serde_json::from_str(&text).expect("every text message is JSON");
        self.sent.lock().unwrap().push(Sent::Text(value));
    }

    fn output(&self, frame: Vec<u8>) -> bool {
        let mut room = self.room.lock().unwrap();
        if frame.len() > *room {
            return false;
        }
        *room -= frame.len();
        let decoded = OutputFrame::decode(&frame).expect("a well-formed frame");
        self.sent.lock().unwrap().push(Sent::Output(
            decoded.stream,
            decoded.position.seq,
            decoded.bytes.to_vec(),
        ));
        true
    }
}

/// Two projects with a workspace each, a phone let in to some of them, and a terminal host.
struct Fixture {
    store: Store,
    git: Git,
    identity: HostIdentity,
    host: PtyHost,
    alpha: String,
    beta: String,
    alpha_project: String,
}

const PHONE: &str = "device-phone";
const LAPTOP: &str = "device-laptop";

impl Fixture {
    fn new() -> Self {
        let store = Store::in_memory();
        let workspace = |name: &str| {
            let project = store.add_project(name, &format!("/code/{name}")).unwrap();
            let workspace = store
                .add_worktree(&project.id, "work", &format!("/code/{name}-wt"), None, None)
                .unwrap();
            (project.id, workspace.id)
        };
        let (alpha_project, alpha) = workspace("alpha");
        let (_, beta) = workspace("beta");
        Self {
            store,
            git: git(),
            identity: HostIdentity {
                id: HostId::new("host-test").unwrap(),
                name: "Test host".into(),
            },
            host: PtyHost::new(Arc::new(|_| {})),
            alpha,
            beta,
            alpha_project,
        }
    }

    /// Let `device` in under `grant`, to `scopes` on `projects`.
    fn approve(&self, device: &str, grant: &str, scopes: &[Scope], projects: ProjectGrant) {
        host::keep_approved(
            &self.store,
            &GrantId::new(grant).unwrap(),
            &DeviceId::new(device).unwrap(),
            &format!("{device} by name"),
            &scopes.iter().copied().collect::<BTreeSet<_>>(),
            &projects,
            1,
        )
        .unwrap();
    }

    fn context(&self) -> Context<'_> {
        Context {
            store: &self.store,
            git: &self.git,
            terminals: &self.host,
            host: &self.identity,
            interactive: true,
        }
    }

    /// A channel for `device` under `grant`, greeted.
    fn visitor(&self, device: &str, grant: &str, out: &Arc<Recorded>) -> Visitor {
        let out: Arc<dyn Outbound> = Arc::clone(out) as _;
        let visitor = Visitor::open(
            &self.store,
            DeviceId::new(device).unwrap(),
            GrantId::new(grant).unwrap(),
            out,
        )
        .unwrap();
        let hello = json!({"client": "test-phone/1", "protocols": [1]}).to_string();
        visitor.handle(&self.context(), &hello).unwrap();
        visitor
    }

    fn terminal(&self, workspace: &str, plan: LaunchPlan) -> SessionInfo {
        let mut plan = plan;
        plan.labels
            .insert(WORKSPACE_LABEL.to_owned(), workspace.to_owned());
        self.host.spawn(plan).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for session in self.host.list() {
            let _ = self.host.kill(&session.id);
        }
    }
}

fn shell(script: &str) -> LaunchPlan {
    let (program, flag) = if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    };
    LaunchPlan {
        program: program.into(),
        args: vec![flag.into(), script.into()],
        cwd: None,
        env: vec![],
        clear_env: false,
        size: pty_host::TermSize { cols: 80, rows: 24 },
        labels: Default::default(),
        prompt: None,
    }
}

/// A program written once for PowerShell on Windows, and once for `sh` everywhere else.
fn script(windows: &str, unix: &str) -> LaunchPlan {
    if cfg!(windows) {
        LaunchPlan {
            program: "powershell.exe".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), windows.into()],
            ..shell(unix)
        }
    } else {
        shell(unix)
    }
}

/// Wait until the program has drawn `text`, watching as a viewer that is not a terminal, so
/// the host goes on answering the program's queries meanwhile.
fn wait_until_drawn(host: &PtyHost, id: &SessionId, text: &str) {
    let seen = Arc::new(Mutex::new(String::new()));
    let watching = Arc::clone(&seen);
    let attachment = host
        .attach_numbered(
            id,
            Box::new(move |item| {
                if let Numbered::Snapshot { bytes, .. } | Numbered::Output { bytes, .. } = item {
                    watching
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(bytes));
                }
                true
            }),
        )
        .unwrap();
    let start = Instant::now();
    while !seen.lock().unwrap().contains(text) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "never drew {text:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    host.detach(id, attachment).unwrap();
}

fn long_running() -> LaunchPlan {
    shell(if cfg!(windows) {
        "ping -n 60 127.0.0.1"
    } else {
        "sleep 60"
    })
}

const ALL: [Scope; 3] = [
    Scope::WorkspaceRead,
    Scope::TerminalRead,
    Scope::TerminalControl,
];

/// Send one request on the channel; its reply is the last text sent with its id.
fn ask(
    fx: &Fixture,
    visitor: &Visitor,
    out: &Recorded,
    id: &str,
    op: &str,
    params: Value,
) -> Value {
    let request = json!({
        "requestId": id,
        "protocol": ProtocolVersion::CURRENT,
        "op": op,
        "params": params,
    });
    visitor.handle(&fx.context(), &request.to_string()).unwrap();
    out.texts()
        .into_iter()
        .rev()
        .find(|t| t["requestId"] == id)
        .unwrap_or_else(|| panic!("no reply to {op}"))
}

fn code(reply: &Value) -> String {
    reply["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("answered: {reply}"))
        .to_owned()
}

#[test]
fn a_device_is_let_in_only_under_a_grant_this_host_approved() {
    let fx = Fixture::new();
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let out = Recorded::new(usize::MAX);
    let as_dyn = || Arc::clone(&out) as Arc<dyn Outbound>;
    let device = DeviceId::new(PHONE).unwrap();
    // A grant only the server knows of, or another device's.
    for (device, grant) in [(PHONE, "g-other"), (LAPTOP, "g-phone")] {
        assert!(matches!(
            Visitor::open(
                &fx.store,
                DeviceId::new(device).unwrap(),
                GrantId::new(grant).unwrap(),
                as_dyn()
            ),
            Err(CloseReason::Refused)
        ));
    }

    // Greeted first; anything else first is refused, and the channel closes.
    let visitor = Visitor::open(
        &fx.store,
        device.clone(),
        GrantId::new("g-phone").unwrap(),
        as_dyn(),
    )
    .unwrap();
    assert_eq!(
        visitor.handle(&fx.context(), r#"{"requestId":"r1"}"#),
        Err(CloseReason::Invalid)
    );
    let visitor = fx.visitor(PHONE, "g-phone", &out);
    let welcome = out.texts().last().unwrap().clone();
    let capabilities = welcome["welcome"]["host"]["capabilities"].to_string();
    assert!(capabilities.contains("terminal.attach"), "{welcome}");

    // Revoked here: the next request is refused, and the channel is to close.
    fx.store.drop_remote_grant("g-phone").unwrap();
    assert!(!visitor.allowed(&fx.store));
    let request = json!({
        "requestId": "r2",
        "protocol": ProtocolVersion::CURRENT,
        "op": "host.describe",
    });
    assert_eq!(
        visitor.handle(&fx.context(), &request.to_string()),
        Err(CloseReason::Revoked)
    );
    assert_eq!(code(out.texts().last().unwrap()), "unauthenticated");
}

#[test]
fn terminals_outside_the_grant_do_not_exist_and_a_lease_is_its_channels_own() {
    let fx = Fixture::new();
    fx.approve(
        PHONE,
        "g-phone",
        &ALL,
        ProjectGrant::Only([ProjectId::new(fx.alpha_project.clone()).unwrap()].into()),
    );
    fx.approve(LAPTOP, "g-laptop", &ALL, ProjectGrant::All);
    fx.approve(
        "device-viewer",
        "g-viewer",
        &[Scope::TerminalRead],
        ProjectGrant::All,
    );
    let alpha = fx.terminal(&fx.alpha, long_running());
    let beta = fx.terminal(&fx.beta, long_running());
    let loose = fx.host.spawn(long_running()).unwrap();

    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    for (i, terminal) in [&beta.id, &loose.id].into_iter().enumerate() {
        for op in ["terminal.attach", "terminal.acquire"] {
            let reply = ask(
                &fx,
                &phone,
                &out,
                &format!("r{op}{i}"),
                op,
                json!({"terminal": terminal.0}),
            );
            assert_eq!(code(&reply), "not_found", "{op} {terminal:?}");
        }
    }
    let viewer_out = Recorded::new(usize::MAX);
    let viewer = fx.visitor("device-viewer", "g-viewer", &viewer_out);
    let reply = ask(
        &fx,
        &viewer,
        &viewer_out,
        "r",
        "terminal.acquire",
        json!({"terminal": alpha.id.0}),
    );
    assert_eq!(code(&reply), "forbidden");

    let lease = ask(
        &fx,
        &phone,
        &out,
        "r-acquire",
        "terminal.acquire",
        json!({"terminal": alpha.id.0}),
    );
    assert_eq!(lease["result"]["kind"], "lease", "{lease}");
    let lease_ref = json!({
        "terminal": alpha.id.0,
        "lease": lease["result"]["id"],
        "generation": lease["result"]["generation"],
    });
    // Another device naming this lease is told it is gone, as for any lease it does not hold.
    let laptop_out = Recorded::new(usize::MAX);
    let laptop = fx.visitor(LAPTOP, "g-laptop", &laptop_out);
    let mut input = lease_ref.clone();
    input["seq"] = 1.into();
    input["data"] = "x".into();
    let stolen = ask(
        &fx,
        &laptop,
        &laptop_out,
        "r-steal",
        "terminal.input",
        input.clone(),
    );
    assert_eq!(code(&stolen), "lease_lost");
    // And it cannot take control from a device without asking to take over.
    let held = ask(
        &fx,
        &laptop,
        &laptop_out,
        "r-held",
        "terminal.acquire",
        json!({"terminal": alpha.id.0}),
    );
    assert_eq!(code(&held), "control_conflict");
    match fx.host.controller(&alpha.id).unwrap() {
        pty_host::Controller::Leased { holder } => assert_eq!(holder, format!("device:{PHONE}")),
        other => panic!("{other:?}"),
    }
    // The holder renews and releases its own.
    let renewed = ask(
        &fx,
        &phone,
        &out,
        "r-renew",
        "terminal.renew",
        lease_ref.clone(),
    );
    assert_eq!(
        renewed["result"]["generation"],
        lease["result"]["generation"]
    );
    let released = ask(
        &fx,
        &phone,
        &out,
        "r-release",
        "terminal.release",
        lease_ref.clone(),
    );
    assert_eq!(released["result"]["kind"], "done", "{released}");
    assert_eq!(
        fx.host.controller(&alpha.id).unwrap(),
        pty_host::Controller::Nobody
    );
    let again = ask(&fx, &phone, &out, "r-again", "terminal.renew", lease_ref);
    assert_eq!(code(&again), "lease_lost");
}

#[test]
fn closing_a_channel_lets_go_of_everything_and_the_program_runs_on() {
    let fx = Fixture::new();
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let terminal = fx.terminal(&fx.alpha, long_running());
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    let attached = ask(
        &fx,
        &phone,
        &out,
        "r-attach",
        "terminal.attach",
        json!({"terminal": terminal.id.0}),
    );
    assert_eq!(attached["result"]["kind"], "attached", "{attached}");
    let lease = ask(
        &fx,
        &phone,
        &out,
        "r-acquire",
        "terminal.acquire",
        json!({"terminal": terminal.id.0}),
    );
    assert_eq!(lease["result"]["kind"], "lease");

    // The relay went, or the phone did.
    phone.close(&fx.host);
    assert_eq!(
        fx.host.controller(&terminal.id).unwrap(),
        pty_host::Controller::Nobody
    );
    assert!(matches!(
        fx.host.info(&terminal.id).unwrap().state,
        SessionState::Running
    ));
    // Someone else may take control at once.
    let laptop_out = Recorded::new(usize::MAX);
    fx.approve(LAPTOP, "g-laptop", &ALL, ProjectGrant::All);
    let laptop = fx.visitor(LAPTOP, "g-laptop", &laptop_out);
    let lease = ask(
        &fx,
        &laptop,
        &laptop_out,
        "r",
        "terminal.acquire",
        json!({"terminal": terminal.id.0}),
    );
    assert_eq!(lease["result"]["kind"], "lease", "{lease}");
}

/// Waits for a line, prints it back, a hundred times.
fn echo_lines() -> LaunchPlan {
    script(
        "for ($i = 0; $i -lt 100; $i++) { $l = [Console]::In.ReadLine(); Write-Output \"got:$l\" }",
        "i=0; while [ $i -lt 100 ]; do read line; echo \"got:$line\"; i=$((i+1)); done",
    )
}

#[test]
fn a_device_watches_from_a_snapshot_and_types_under_its_lease() {
    let fx = Fixture::new();
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let terminal = fx.terminal(
        &fx.alpha,
        script(
            "Write-Output before; $l = [Console]::In.ReadLine(); Write-Output \"got:$l\"; \
             Start-Sleep 30",
            "printf 'before\\n'; exec sh -c 'read l; echo \"got:$l\"; sleep 30'",
        ),
    );
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    // Something drawn before the device came.
    wait_until_drawn(&fx.host, &terminal.id, "before");

    let attached = ask(
        &fx,
        &phone,
        &out,
        "r-attach",
        "terminal.attach",
        json!({"terminal": terminal.id.0}),
    );
    let result = &attached["result"];
    assert_eq!(result["start"], "snapshot", "{attached}");
    assert_eq!(result["controller"]["kind"], "nobody");
    let stream = result["stream"].as_u64().unwrap() as u32;
    let snapshot_seq = result["position"]["seq"].as_u64().unwrap();

    // The reply came before any of the stream's output, and the snapshot is numbered as the
    // reply says.
    let sent = out.sent();
    let reply_at = sent
        .iter()
        .position(|s| matches!(s, Sent::Text(v) if v["requestId"] == "r-attach"))
        .unwrap();
    let first_output = sent
        .iter()
        .position(|s| matches!(s, Sent::Output(..)))
        .unwrap();
    assert!(reply_at < first_output);
    let Sent::Output(_, seq, snapshot) = &sent[first_output] else {
        unreachable!()
    };
    assert_eq!(*seq, snapshot_seq);
    assert!(String::from_utf8_lossy(snapshot).contains("before"));

    // Typing under a lease reaches the program, and its answer reaches the device numbered on.
    let lease = ask(
        &fx,
        &phone,
        &out,
        "r-acquire",
        "terminal.acquire",
        json!({"terminal": terminal.id.0}),
    );
    let input = json!({
        "terminal": terminal.id.0,
        "lease": lease["result"]["id"],
        "generation": lease["result"]["generation"],
        "seq": 1,
        "data": "hello\r",
    });
    let typed = ask(&fx, &phone, &out, "r-input", "terminal.input", input);
    assert_eq!(typed["result"]["kind"], "done", "{typed}");
    out.wait_for("the program's answer", |out| {
        out.drawn(stream).contains("got:hello")
    });
    let numbers: Vec<u64> = out
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Output(s, seq, _) if s == stream => Some(seq),
            _ => None,
        })
        .collect();
    assert!(
        numbers.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1),
        "numbers skip or go back: {numbers:?}"
    );

    // A resize under the lease is an event in the stream, numbered after the output before it.
    let resize = json!({
        "terminal": terminal.id.0,
        "lease": lease["result"]["id"],
        "generation": lease["result"]["generation"],
        "size": {"cols": 100, "rows": 30},
    });
    let resized = ask(&fx, &phone, &out, "r-resize", "terminal.resize", resize);
    assert_eq!(resized["result"]["kind"], "done", "{resized}");
    let event = out
        .texts()
        .into_iter()
        .find(|t| t["event"] == "terminal.resized")
        .expect("a resize event");
    assert_eq!(event["size"]["cols"], 100);
    assert!(event["position"]["seq"].as_u64().unwrap() > *numbers.last().unwrap());

    // Attached again, the controller is the phone, by the name this host knows it by.
    let again = ask(
        &fx,
        &phone,
        &out,
        "r-again",
        "terminal.attach",
        json!({"terminal": terminal.id.0}),
    );
    assert_eq!(again["result"]["controller"]["kind"], "device", "{again}");
    assert_eq!(
        again["result"]["controller"]["name"],
        format!("{PHONE} by name")
    );
}

#[test]
fn a_device_that_falls_behind_is_detached_and_told_and_the_program_never_waits() {
    let fx = Fixture::new();
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let terminal = fx.terminal(&fx.alpha, echo_lines());
    // Room for the snapshot and a little more, never drained.
    let out = Recorded::new(16 * 1024);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    let attached = ask(
        &fx,
        &phone,
        &out,
        "r-attach",
        "terminal.attach",
        json!({"terminal": terminal.id.0}),
    );
    let stream = attached["result"]["stream"].clone();

    let line = "x".repeat(1000);
    for _ in 0..40 {
        fx.host
            .write(&terminal.id, format!("{line}\r").as_bytes())
            .unwrap();
    }
    out.wait_for("the device to be detached", |out| {
        out.texts()
            .iter()
            .any(|t| t["event"] == "terminal.detached")
    });
    let detached = out
        .texts()
        .into_iter()
        .find(|t| t["event"] == "terminal.detached")
        .unwrap();
    assert_eq!(detached["stream"], stream);
    assert_eq!(detached["error"]["code"], "resnapshot_required");
    let frames = out.sent().len();

    // The program carried on, and nothing more came on the dead stream.
    fx.host.write(&terminal.id, b"after\r").unwrap();
    let start = Instant::now();
    loop {
        let snapshot = fx.host.info(&terminal.id).unwrap();
        assert!(matches!(snapshot.state, SessionState::Running));
        if start.elapsed() > Duration::from_millis(500) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(out.sent().len(), frames);
}

/// The last number a stream has sent, output or resize.
fn last_seq(out: &Recorded, stream: u32) -> u64 {
    out.sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Output(s, seq, _) if s == stream => Some(seq),
            Sent::Text(v) if v["event"] == "terminal.resized" && v["stream"] == stream => {
                v["position"]["seq"].as_u64()
            }
            _ => None,
        })
        .max()
        .expect("the stream sent something")
}

#[test]
fn a_device_that_comes_back_resumes_and_hears_who_controls_and_that_the_program_ended() {
    let fx = Fixture::new();
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let terminal = fx.terminal(&fx.alpha, echo_lines());
    let id = json!({"terminal": terminal.id.0});
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    let attached = ask(&fx, &phone, &out, "r1", "terminal.attach", id.clone());
    assert_eq!(attached["result"]["controller"]["kind"], "nobody");
    let first = attached["result"]["stream"].as_u64().unwrap() as u32;

    // Typed at the host: the person here has control, and the device is told.
    fx.host.write(&terminal.id, b"one\r").unwrap();
    out.wait_for("the program's first answer", |out| {
        out.drawn(first).contains("got:one")
    });
    out.wait_for("the control event", |out| {
        out.texts()
            .iter()
            .any(|t| t["event"] == "terminal.control" && t["stream"] == first)
    });
    let control = out
        .texts()
        .into_iter()
        .find(|t| t["event"] == "terminal.control")
        .unwrap();
    assert_eq!(control["controller"]["kind"], "local", "{control}");

    // The device goes, the program carries on, and the device comes back where it left off.
    let left_at = last_seq(&out, first);
    let detached = ask(
        &fx,
        &phone,
        &out,
        "r2",
        "terminal.detach",
        json!({"terminal": terminal.id.0, "stream": first}),
    );
    assert_eq!(detached["result"]["kind"], "done", "{detached}");
    fx.host.write(&terminal.id, b"two\r").unwrap();
    wait_until_drawn(&fx.host, &terminal.id, "got:two");
    let mut again = id.clone();
    again["resumeAfter"] = json!({"epoch": 0, "seq": left_at});
    let resumed = ask(&fx, &phone, &out, "r3", "terminal.attach", again);
    assert_eq!(resumed["result"]["start"], "resumed", "{resumed}");
    assert_eq!(resumed["result"]["position"]["seq"], left_at);
    assert_eq!(resumed["result"]["controller"]["kind"], "local");
    let second = resumed["result"]["stream"].as_u64().unwrap() as u32;
    out.wait_for("what it missed", |out| {
        out.drawn(second).contains("got:two")
    });
    // Only what it missed: nothing from before it left.
    assert!(
        !out.drawn(second).contains("got:one"),
        "{:?}",
        out.drawn(second)
    );
    let numbers: Vec<u64> = out
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Output(s, seq, _) if s == second => Some(seq),
            _ => None,
        })
        .collect();
    assert_eq!(numbers.first(), Some(&(left_at + 1)), "{numbers:?}");

    // A position from another epoch is not this terminal's: a snapshot.
    let mut stranger = id.clone();
    stranger["resumeAfter"] = json!({"epoch": 7, "seq": left_at});
    let fresh = ask(&fx, &phone, &out, "r4", "terminal.attach", stranger);
    assert_eq!(fresh["result"]["start"], "snapshot", "{fresh}");

    // The program ends: every stream watching it is told, after everything it printed.
    fx.host.kill(&terminal.id).unwrap();
    out.wait_for("the exit event", |out| {
        out.texts()
            .iter()
            .any(|t| t["event"] == "terminal.exited" && t["stream"] == second)
    });
    let exited = out
        .texts()
        .into_iter()
        .find(|t| t["event"] == "terminal.exited" && t["stream"] == second)
        .unwrap();
    assert_eq!(exited["exit"]["success"], false, "{exited}");
}

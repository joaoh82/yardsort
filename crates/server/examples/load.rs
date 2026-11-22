//! A known load through a Yardsort server's relay and sync, measured against the proposal's
//! budgets (25 §14): R7 slice 6's load test ([46 §13]).
//!
//! ```text
//! cargo run --release -p yardsort-server --example load -- [SERVER]
//!     [--hosts N] [--rate PER_SECOND] [--output BYTES] [--minutes M] [--stalled]
//!     [--server-pid PID] [--json FILE] [--follow | --zitadel-api]
//! ```
//!
//! `SERVER` is `http://127.0.0.1:8787` unless given. It signs in `N` hosts (5 unless given) and
//! a phone for each, registers each host, pairs each phone with its host and has the host approve
//! it, and opens every relay link. Then, for `M` minutes (1 unless given):
//!
//! - each phone sends `PER_SECOND` small messages a second (20 unless given) through the relay to
//!   its host, which answers each at once with `BYTES` of output (512 unless given), as a terminal
//!   echoes typing; the phone times each round trip;
//! - each host pushes a note to a project of its own once a second, and pulls what changed, as a
//!   host keeps its projects in sync; each push and pull is timed.
//!
//! With `--stalled`, the first host has one more phone, which opens its link and never reads
//! while the host sends it output at the same rate: the others must not slow down for it, and the
//! relay should let it go. With `--server-pid`, the server's resident memory (Linux's
//! `/proc/PID/status`) is read every ten seconds: a server on this machine only. On staging,
//! Fly's metrics say it.
//!
//! At the end the first host revokes its phone's grant, and the time until the phone's socket is
//! closed is measured. Then every phone and host is signed out and every host removed. It prints
//! a report, writes it as JSON with `--json`, and exits non-zero when a budget was missed:
//!
//! - a round trip through the relay, p95 under 300 ms at a 100 ms network: the proposal's
//!   input-to-echo budget. A round trip crosses the network twice (phone and server, server and
//!   host), so the network's own round trip, timed from `GET /health` every two seconds during
//!   the run, is taken off twice at its median, and what is left must be under the 100 ms that
//!   two 100 ms crossings leave of 300;
//! - a link opened (ticket, socket and the host told), p95 under 5 s (reconnection);
//! - a revoked phone's socket closed within 5 s;
//! - sync's push and pull, p95 under 1 s;
//! - with `--stalled`, the stalled host's other phone within the same round-trip budget;
//! - with `--server-pid`, memory at the end no more than half again what it was after the first
//!   minute.
//!
//! Signing in: in a browser for each device unless `--follow` (the stand-in's provider,
//! `examples/stand-in.rs`) or `--zitadel-api` (as `examples/sign-in.rs` says; every device signs
//! in as the one user, so it is one account with many devices). Setup keeps to the server's rate
//! limits, waiting when told to. Credentials are never printed; nothing is saved.
//!
//! [46 §13]: ../../../docs/design/46-remote-r7.md

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{open, through_zitadel_api};
use remote_client::{Client, ClientError, Incoming, LinkReader, LinkWriter, Session};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::pairing::{GrantDecision, HostRegistration, PairingRequest};
use remote_proto::relay::{LinkFrame, TicketRequest};
use remote_proto::sync::{Cursor, PushRequest};
use remote_proto::{ErrorCode, GrantId, HostId, Platform};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const ECHO_P95_MS: f64 = 300.0;
/// The network round trip the proposal's echo budget is stated at, each way to the server.
const BUDGET_RTT_MS: f64 = 100.0;
const LINK_P95_MS: f64 = 5_000.0;
const REVOKE_MS: f64 = 5_000.0;
const SYNC_P95_MS: f64 = 1_000.0;
const MEMORY_GROWTH: f64 = 1.5;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("load: {e}");
        std::process::exit(1);
    }
}

struct Options {
    server: String,
    hosts: usize,
    rate: u32,
    output: usize,
    minutes: f64,
    stalled: bool,
    server_pid: Option<u32>,
    json: Option<String>,
    how: How,
}

fn options() -> Result<Options, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let number = |name: &str, default: f64| -> Result<f64, String> {
        value(name)
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| format!("{name} takes a number"))
            })
            .unwrap_or(Ok(default))
    };
    let flag = |name: &str| args.iter().any(|a| a == name);
    let with_value = [
        "--hosts",
        "--rate",
        "--output",
        "--minutes",
        "--server-pid",
        "--json",
    ];
    let mut server = None;
    let mut skip = false;
    for arg in &args {
        if skip {
            skip = false;
        } else if with_value.contains(&arg.as_str()) {
            skip = true;
        } else if !arg.starts_with("--") {
            server = Some(arg.clone());
        }
    }
    let hosts = number("--hosts", 5.0)? as usize;
    if hosts == 0 {
        return Err("--hosts takes at least 1".into());
    }
    Ok(Options {
        server: server.unwrap_or_else(|| "http://127.0.0.1:8787".into()),
        hosts,
        rate: (number("--rate", 20.0)? as u32).max(1),
        output: number("--output", 512.0)? as usize,
        minutes: number("--minutes", 1.0)?,
        stalled: flag("--stalled"),
        server_pid: value("--server-pid")
            .map(|v| v.parse().map_err(|_| "--server-pid takes a process id"))
            .transpose()?,
        json: value("--json"),
        how: if flag("--zitadel-api") {
            How::Zitadel
        } else if flag("--follow") {
            How::Follow
        } else {
            How::Browser
        },
    })
}

#[derive(Clone, Copy)]
enum How {
    Browser,
    Follow,
    Zitadel,
}

/// A signed-in device, kept fresh across a long run.
#[derive(Clone)]
struct Device {
    client: Client,
    session: Arc<tokio::sync::Mutex<Session>>,
}

impl Device {
    async fn access(&self) -> Result<Secret, String> {
        self.session
            .lock()
            .await
            .access(|_| Ok(()))
            .await
            .map_err(|e| e.to_string())
    }
}

/// Retry what the server refused for its rate limits, after the time it said, or a few seconds.
async fn paced<T, F, Fut>(what: &str, mut call: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ClientError>>,
{
    for _ in 0..30 {
        match call().await {
            Ok(value) => return Ok(value),
            Err(e) if e.code() == Some(ErrorCode::RateLimited) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => return Err(format!("{what}: {e}")),
        }
    }
    Err(format!("{what}: still limited after a few minutes"))
}

async fn sign_in(
    client: &Client,
    kind: DeviceKind,
    name: &str,
    how: How,
) -> Result<Device, String> {
    let sign_in = paced("starting a sign-in", || client.begin_sign_in()).await?;
    let device = NewDevice {
        name: name.into(),
        kind,
        platform: Platform::current(),
    };
    let url = sign_in.url().clone();
    let credentials = match how {
        How::Browser => {
            println!("Sign {name} in, in a browser:\n\n  {url}\n");
            open(url.as_str());
            sign_in
                .finish(client, device, remote_client::SIGN_IN_TIMEOUT)
                .await
        }
        How::Follow => {
            let browser = yardsort_server::oidc::http_client_builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .map_err(|e| e.to_string())?;
            let follow = async {
                let _ = browser.get(url).send().await;
            };
            let (credentials, ()) = tokio::join!(
                sign_in.finish(client, device, Duration::from_secs(60)),
                follow
            );
            credentials
        }
        How::Zitadel => {
            let http = yardsort_server::oidc::http_client_builder()
                .build()
                .map_err(|e| e.to_string())?;
            let (credentials, browsed) = tokio::join!(
                sign_in.finish(client, device, Duration::from_secs(60)),
                through_zitadel_api(&http, url),
            );
            browsed?;
            credentials
        }
    }
    .map_err(|e| format!("signing {name} in: {e}"))?;
    Ok(Device {
        client: client.clone(),
        session: Arc::new(tokio::sync::Mutex::new(Session::new(
            client.clone(),
            credentials,
        ))),
    })
}

/// A host, signed in and registered, and a phone of its, paired and approved.
struct Pair {
    host: Device,
    host_id: HostId,
    phones: Vec<(Device, GrantId)>,
}

async fn pair(client: &Client, n: usize, phones: usize, how: How) -> Result<Pair, String> {
    let host = sign_in(client, DeviceKind::Host, &format!("Load host {n}"), how).await?;
    let access = host.access().await?;
    let registered = client
        .register_host(
            &access,
            &HostRegistration {
                host: None,
                name: format!("Load host {n}"),
            },
        )
        .await
        .map_err(|e| format!("registering host {n}: {e}"))?;
    let scopes = json!(["workspace:read", "terminal:read", "terminal:control"]);
    let decision: GrantDecision =
        serde_json::from_value(json!({ "scopes": scopes, "projects": { "kind": "all" } }))
            .map_err(|e| e.to_string())?;
    let mut paired = Vec::new();
    for p in 0..phones {
        let phone = sign_in(
            client,
            DeviceKind::Phone,
            &format!("Load phone {n}.{p}"),
            how,
        )
        .await?;
        let offer = paced("making an offer", || client.offer(&access, &registered.id)).await?;
        let request: PairingRequest = serde_json::from_value(json!({
            "offer": { "kind": "secret", "value": offer.secret.expose() },
            "scopes": scopes,
        }))
        .map_err(|e| e.to_string())?;
        let phone_access = phone.access().await?;
        let asked = paced("pairing", || client.pair(&phone_access, &request)).await?;
        client
            .decide(&access, &asked.id, Some(&decision))
            .await
            .map_err(|e| format!("approving phone {n}.{p}: {e}"))?;
        paired.push((phone, asked.id));
    }
    Ok(Pair {
        host,
        host_id: registered.id,
        phones: paired,
    })
}

/// Times, in milliseconds, kept for the report.
#[derive(Default)]
struct Timings(Mutex<Vec<f64>>);

impl Timings {
    fn add(&self, since: Instant) {
        self.0
            .lock()
            .unwrap()
            .push(since.elapsed().as_secs_f64() * 1000.0);
    }

    fn summary(&self) -> Summary {
        let mut all = self.0.lock().unwrap().clone();
        all.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| -> f64 {
            if all.is_empty() {
                return f64::NAN;
            }
            all[((all.len() as f64 * q).ceil() as usize).clamp(1, all.len()) - 1]
        };
        Summary {
            count: all.len(),
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: all.last().copied().unwrap_or(f64::NAN),
        }
    }
}

struct Summary {
    count: usize,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Summary {
    fn json(&self) -> Value {
        json!({ "count": self.count, "p50": self.p50, "p95": self.p95, "p99": self.p99, "max": self.max })
    }

    fn line(&self, name: &str) -> String {
        format!(
            "{name:<26} {:>8} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            self.count, self.p50, self.p95, self.p99, self.max
        )
    }
}

/// What a run measured.
#[derive(Default)]
struct Measured {
    echo: Timings,
    /// The stalled host's other phone, measured apart.
    beside_stalled: Timings,
    link: Timings,
    push: Timings,
    pull: Timings,
    /// `GET /health` while the load runs: the network's own round trip, the server doing nothing.
    network: Timings,
    lost: std::sync::atomic::AtomicU64,
}

/// Open a phone's link, timed until its host has been told.
async fn phone_link(
    phone: &Device,
    host: &HostId,
    told: &Arc<HostEars>,
    measured: &Measured,
) -> Result<(LinkReader, LinkWriter, u32), String> {
    let started = Instant::now();
    let access = phone.access().await?;
    let device = phone.session.lock().await.device.to_string();
    let waiting = told.expect(&device);
    let request = TicketRequest::Device { host: host.clone() };
    let link = paced("opening a phone's link", || {
        phone.client.open_relay(&access, &request)
    })
    .await?;
    let channel = tokio::time::timeout(Duration::from_secs(30), waiting)
        .await
        .map_err(|_| "the host was not told of a phone".to_string())?
        .map_err(|_| "the host's link closed".to_string())?;
    measured.link.add(started);
    Ok((link.reader, link.writer, channel))
}

/// Who the host has been told of: each phone's channel, by its device.
#[derive(Default)]
struct HostEars {
    waiting: Mutex<HashMap<String, tokio::sync::oneshot::Sender<u32>>>,
}

impl HostEars {
    fn expect(&self, device: &str) -> tokio::sync::oneshot::Receiver<u32> {
        let (tell, told) = tokio::sync::oneshot::channel();
        self.waiting.lock().unwrap().insert(device.into(), tell);
        told
    }

    fn opened(&self, device: &str, channel: u32) {
        if let Some(tell) = self.waiting.lock().unwrap().remove(device) {
            let _ = tell.send(channel);
        }
    }
}

/// A host's link: told of each phone, answering each message at once with `output` bytes. The
/// returned sender takes frames to send, for the stalled phone's output.
async fn host_link(
    pair: &Pair,
    output: usize,
) -> Result<(Arc<HostEars>, mpsc::Sender<Vec<u8>>), String> {
    let access = pair.host.access().await?;
    let request = TicketRequest::Host {
        host: pair.host_id.clone(),
        protocol: None,
    };
    let link = paced("opening a host's link", || {
        pair.host.client.open_relay(&access, &request)
    })
    .await?;
    let ears = Arc::new(HostEars::default());
    let (send, mut sending) = mpsc::channel::<Vec<u8>>(1024);
    let mut writer = link.writer;
    tokio::spawn(async move {
        while let Some(frame) = sending.recv().await {
            if writer.binary(frame).await.is_err() {
                break;
            }
        }
    });
    let mut reader = link.reader;
    let heard = Arc::clone(&ears);
    let answer = send.clone();
    let padding = "x".repeat(output);
    tokio::spawn(async move {
        while let Ok(message) = reader.next().await {
            let Incoming::Binary(bytes) = message else {
                continue;
            };
            match LinkFrame::decode(&bytes) {
                Ok(LinkFrame::Open(channel, open)) => {
                    heard.opened(&open.device.to_string(), channel);
                }
                Ok(LinkFrame::Text(channel, text)) => {
                    // The phone's sequence number, and the output it is waiting for.
                    let asked: Value = serde_json::from_slice(text).unwrap_or(Value::Null);
                    let reply = json!({ "n": asked["n"], "out": padding }).to_string();
                    if answer
                        .send(LinkFrame::Text(channel, reply.as_bytes()).encode())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                _ => {}
            }
        }
    });
    Ok((ears, send))
}

/// A phone typing for `until`, timing each echo. Returns its link, still open, for the
/// revocation at the end.
async fn type_and_time(
    mut reader: LinkReader,
    mut writer: LinkWriter,
    rate: u32,
    until: Instant,
    times: Arc<Measured>,
    beside_stalled: bool,
) -> (LinkReader, LinkWriter) {
    let sent: Arc<Mutex<HashMap<u64, Instant>>> = Arc::default();
    let counted = Arc::clone(&sent);
    let timing = Arc::clone(&times);
    let listening = tokio::spawn(async move {
        loop {
            let Ok(message) = reader.next().await else {
                break;
            };
            let Incoming::Text(text) = message else {
                continue;
            };
            let reply: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let Some(n) = reply["n"].as_u64() else {
                continue;
            };
            if let Some(at) = counted.lock().unwrap().remove(&n) {
                if beside_stalled {
                    timing.beside_stalled.add(at);
                } else {
                    timing.echo.add(at);
                }
            }
            if Instant::now() >= until && counted.lock().unwrap().is_empty() {
                break;
            }
        }
        reader
    });
    let mut every = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(rate)));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n = 0u64;
    while Instant::now() < until {
        every.tick().await;
        sent.lock().unwrap().insert(n, Instant::now());
        if writer
            .text(json!({ "n": n, "keys": "ls -la\r" }).to_string())
            .await
            .is_err()
        {
            break;
        }
        n += 1;
    }
    // A few seconds for the last answers; what never came back is lost.
    let reader = match tokio::time::timeout(Duration::from_secs(5), listening).await {
        Ok(Ok(reader)) => Some(reader),
        _ => None,
    };
    let lost = sent.lock().unwrap().len() as u64;
    times
        .lost
        .fetch_add(lost, std::sync::atomic::Ordering::Relaxed);
    (reader.expect("a phone's listener ended"), writer)
}

/// A host keeping a project of its own in sync, once a second until `until`.
async fn sync_for(
    host: Device,
    n: usize,
    until: Instant,
    times: Arc<Measured>,
) -> Result<(), String> {
    let project = uuid::Uuid::new_v4().to_string();
    let op = |kind: &str, id: &str, text: String| {
        json!({
            "op": uuid::Uuid::new_v4().to_string(), "action": "put", "kind": kind, "id": id,
            "project": project, "schema": 1, "body": { "text": text },
        })
    };
    let first: PushRequest =
        serde_json::from_value(json!({ "ops": [op("project", &project, format!("Load {n}"))] }))
            .map_err(|e| e.to_string())?;
    let access = host.access().await?;
    host.client
        .sync_push(&access, &first)
        .await
        .map_err(|e| format!("host {n} turning its project on: {e}"))?;
    let mut cursor: Cursor = serde_json::from_value(json!(0)).map_err(|e| e.to_string())?;
    let mut every = tokio::time::interval(Duration::from_secs(1));
    let mut note = 0;
    while Instant::now() < until {
        every.tick().await;
        let access = host.access().await?;
        let push: PushRequest = serde_json::from_value(json!({
            "ops": [op("note", &uuid::Uuid::new_v4().to_string(), format!("note {note}"))],
        }))
        .map_err(|e| e.to_string())?;
        note += 1;
        let started = Instant::now();
        match host.client.sync_push(&access, &push).await {
            Ok(_) => times.push.add(started),
            Err(e) if e.code() == Some(ErrorCode::RateLimited) => continue,
            Err(e) => return Err(format!("host {n} pushing: {e}")),
        }
        let started = Instant::now();
        match host.client.sync_pull(&access, cursor).await {
            Ok(page) => {
                times.pull.add(started);
                cursor = page.cursor;
            }
            Err(e) if e.code() == Some(ErrorCode::RateLimited) => continue,
            Err(e) => return Err(format!("host {n} pulling: {e}")),
        }
    }
    Ok(())
}

fn resident_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

async fn run() -> Result<(), String> {
    let o = options()?;
    let client = Client::new(&o.server).map_err(|e| e.to_string())?;
    println!(
        "Load on {}: {} hosts with a phone each{}, {} messages a second from each phone, {} bytes back, for {} minutes.",
        client.address(),
        o.hosts,
        if o.stalled { ", and a stalled phone on the first" } else { "" },
        o.rate,
        o.output,
        o.minutes
    );

    // Setup, within the server's rate limits.
    let mut pairs = Vec::new();
    for n in 0..o.hosts {
        let phones = if n == 0 && o.stalled { 2 } else { 1 };
        pairs.push(pair(&client, n, phones, o.how).await?);
        println!("Host {n} and its phone signed in and paired.");
    }

    let measured = Arc::new(Measured::default());
    let mut hosts = Vec::new();
    for p in &pairs {
        hosts.push(host_link(p, o.output).await?);
    }
    let mut links = Vec::new();
    for (p, (ears, _)) in pairs.iter().zip(&hosts) {
        let (phone, _) = &p.phones[0];
        links.push(phone_link(phone, &p.host_id, ears, &measured).await?);
    }
    let stalled = if o.stalled {
        let (phone, _) = &pairs[0].phones[1];
        let (reader, writer, channel) =
            phone_link(phone, &pairs[0].host_id, &hosts[0].0, &measured).await?;
        Some((reader, writer, channel))
    } else {
        None
    };
    println!("Every link is open. Running.");

    let started = Instant::now();
    let until = started + Duration::from_secs_f64(o.minutes * 60.0);
    let mut memory = Vec::new();
    let sampling = o.server_pid.map(|pid| {
        tokio::spawn(async move {
            let mut samples = Vec::new();
            let mut every = tokio::time::interval(Duration::from_secs(10));
            while Instant::now() < until + Duration::from_secs(1) {
                every.tick().await;
                if let Some(kb) = resident_kb(pid) {
                    samples.push((started.elapsed().as_secs_f64(), kb));
                }
            }
            samples
        })
    });

    // The network's own round trip, on a connection of its own, every two seconds.
    let probing = {
        let times = Arc::clone(&measured);
        let health = client.address().join("health").map_err(|e| e.to_string())?;
        let http = yardsort_server::oidc::http_client_builder()
            .build()
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _ = http.get(health.clone()).send().await;
            let mut every = tokio::time::interval(Duration::from_secs(2));
            while Instant::now() < until {
                every.tick().await;
                let at = Instant::now();
                if let Ok(response) = http.get(health.clone()).send().await {
                    if response.bytes().await.is_ok() {
                        times.network.add(at);
                    }
                }
            }
        })
    };

    // The stalled phone: output sent to it at the typing rate, and never read.
    let stalled_closed = Arc::new(Mutex::new(None::<f64>));
    if let Some((mut reader, writer, channel)) = stalled {
        let send = hosts[0].1.clone();
        let rate = o.rate;
        let padding = "x".repeat(o.output);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(rate)));
            while Instant::now() < until {
                every.tick().await;
                let frame = json!({ "out": padding }).to_string();
                if send
                    .send(LinkFrame::Text(channel, frame.as_bytes()).encode())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        // Not read until the end: then drained, to see whether the relay had let it go.
        let closed = Arc::clone(&stalled_closed);
        tokio::spawn(async move {
            let _writer = writer;
            tokio::time::sleep_until(until.into()).await;
            let drained = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if reader.next().await.is_err() {
                        return true;
                    }
                }
            })
            .await
            .unwrap_or(false);
            *closed.lock().unwrap() = Some(if drained { 1.0 } else { 0.0 });
        });
    }

    let mut typing = Vec::new();
    for (i, (reader, writer, _)) in links.into_iter().enumerate() {
        typing.push(tokio::spawn(type_and_time(
            reader,
            writer,
            o.rate,
            until,
            Arc::clone(&measured),
            o.stalled && i == 0,
        )));
    }
    let mut syncing = Vec::new();
    for (n, p) in pairs.iter().enumerate() {
        syncing.push(tokio::spawn(sync_for(
            p.host.clone(),
            n,
            until,
            Arc::clone(&measured),
        )));
    }
    let mut readers = Vec::new();
    for t in typing {
        readers.push(t.await.map_err(|e| e.to_string())?);
    }
    for s in syncing {
        s.await.map_err(|e| e.to_string())??;
    }
    let _ = probing.await;
    if let Some(sampling) = sampling {
        memory = sampling.await.map_err(|e| e.to_string())?;
    }

    // The first host takes its phone's grant back: how long until the phone is let go.
    let open_before;
    let revoked = {
        let p = &pairs[0];
        let access = p.host.access().await?;
        let (mut reader, _writer) = readers.remove(0);
        // Still open before it: a socket that had closed already proves nothing.
        open_before = match tokio::time::timeout(Duration::from_millis(300), reader.next()).await {
            Err(_) | Ok(Ok(_)) => true,
            Ok(Err(_)) => false,
        };
        let at = Instant::now();
        client
            .revoke_grant(&access, &p.phones[0].1)
            .await
            .map_err(|e| format!("revoking: {e}"))?;
        let closed = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if reader.next().await.is_err() {
                    break;
                }
            }
        })
        .await;
        closed.ok().map(|()| at.elapsed().as_secs_f64() * 1000.0)
    };
    let revoked = revoked.filter(|_| open_before);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stalled_let_go = *stalled_closed.lock().unwrap();

    // Leave nothing behind: every device signed out, every host removed.
    for p in &pairs {
        let access = p.host.access().await?;
        let _ = client.remove_host(&access, &p.host_id).await;
        for (phone, _) in &p.phones {
            if let Ok(phone_access) = phone.access().await {
                let device = phone.session.lock().await.device.clone();
                let _ = client.revoke_device(&phone_access, &device).await;
            }
        }
        let device = p.host.session.lock().await.device.clone();
        let _ = client.revoke_device(&access, &device).await;
    }

    // The report.
    let echo = measured.echo.summary();
    let beside = measured.beside_stalled.summary();
    let link = measured.link.summary();
    let push = measured.push.summary();
    let pull = measured.pull.summary();
    let network = measured.network.summary();
    let lost = measured.lost.load(std::sync::atomic::Ordering::Relaxed);
    println!(
        "\n{:<26} {:>8} {:>9} {:>9} {:>9} {:>9}",
        "ms", "count", "p50", "p95", "p99", "max"
    );
    println!("{}", echo.line("round trip through relay"));
    if o.stalled {
        println!("{}", beside.line("  beside the stalled one"));
    }
    println!("{}", link.line("link opened"));
    println!("{}", push.line("sync push"));
    println!("{}", pull.line("sync pull"));
    println!("{}", network.line("network (GET /health)"));
    println!("Messages that never came back: {lost}.");

    let mut missed = Vec::new();
    let mut check = |name: &str, ok: bool, said: String| {
        println!("{} {name}: {said}", if ok { "ok  " } else { "MISS" });
        if !ok {
            missed.push(name.to_string());
        }
    };
    println!();
    // A round trip through the relay crosses the network twice: phone to server and back, and
    // server to host and back. The budget is 300 ms with each of those at 100 ms, which leaves
    // 100 ms for the rest. What this run's network took is taken off, and that 100 ms is the
    // budget for what is left.
    let beyond = |p95: f64| p95 - 2.0 * network.p50;
    let left = ECHO_P95_MS - 2.0 * BUDGET_RTT_MS;
    let said = |p95: f64| {
        format!(
            "{p95:.1} ms, with the network at {:.1} ms each way and back: {:.1} ms beyond it",
            network.p50,
            beyond(p95)
        )
    };
    check(
        "round trip p95 under 300 ms at a 100 ms network",
        beyond(echo.p95) < left,
        said(echo.p95),
    );
    if o.stalled {
        check(
            "beside a stalled phone, the same",
            beyond(beside.p95) < left,
            said(beside.p95),
        );
        check(
            "the stalled phone let go by the relay",
            stalled_let_go == Some(1.0),
            if stalled_let_go == Some(1.0) {
                "closed".into()
            } else {
                "still open".into()
            },
        );
    }
    check(
        "link opened p95 under 5 s",
        link.p95 < LINK_P95_MS,
        format!("{:.1} ms", link.p95),
    );
    check(
        "a revoked phone let go within 5 s",
        revoked.is_some_and(|ms| ms < REVOKE_MS),
        match revoked {
            Some(ms) => format!("{ms:.1} ms"),
            None if !open_before => "its socket had closed before the revocation".into(),
            None => "never".into(),
        },
    );
    check(
        "sync push p95 under 1 s",
        push.p95 < SYNC_P95_MS,
        format!("{:.1} ms", push.p95),
    );
    check(
        "sync pull p95 under 1 s",
        pull.p95 < SYNC_P95_MS,
        format!("{:.1} ms", pull.p95),
    );
    check("every message answered", lost == 0, format!("{lost} lost"));
    let warmed = memory
        .iter()
        .find(|(at, _)| *at >= 60.0)
        .or(memory.first())
        .map(|(_, kb)| *kb);
    let last = memory.last().map(|(_, kb)| *kb);
    if let (Some(warmed), Some(last)) = (warmed, last) {
        check(
            "server memory bounded",
            (last as f64) <= warmed as f64 * MEMORY_GROWTH,
            format!(
                "{} MB after the first minute, {} MB at the end, {} MB at most",
                warmed / 1024,
                last / 1024,
                memory.iter().map(|(_, kb)| kb).max().unwrap() / 1024
            ),
        );
    }

    if let Some(path) = &o.json {
        let report = json!({
            "server": client.address().as_str(),
            "hosts": o.hosts, "rate": o.rate, "output": o.output, "minutes": o.minutes,
            "stalled": o.stalled,
            "echo": echo.json(), "besideStalled": beside.json(), "link": link.json(),
            "push": push.json(), "pull": pull.json(), "network": network.json(), "lost": lost,
            "revokedMs": revoked, "stalledLetGo": stalled_let_go.map(|v| v == 1.0),
            "memoryKb": memory.iter().map(|(at, kb)| json!([at, kb])).collect::<Vec<_>>(),
            "missed": missed,
        });
        std::fs::write(path, format!("{report:#}\n")).map_err(|e| format!("{path}: {e}"))?;
    }
    if missed.is_empty() {
        println!("\nEvery budget met.");
        Ok(())
    } else {
        Err(format!("{} budgets missed", missed.len()))
    }
}

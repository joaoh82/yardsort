//! A stand-in for Fly.io's proxy in front of several instances of the server, as far as the relay
//! needs one: each request goes to an instance of the proxy's choosing, and an answer carrying
//! `fly-replay` is not passed on: the request goes again to the instance it names, with
//! `fly-replay-src` saying where it was. `prefer_instance` falls back to another instance when the
//! one named cannot be reached, and says so in `fly-preferred-instance-unavailable`.
//!
//! New requests go to each instance in turn, or to the one [`ReplayProxy::land_on`] names, and to
//! the next one when that one cannot be reached. HTTP/1.1 only. Each connection carries one request, except a WebSocket, which is carried
//! through once upgraded. Used by the tests (`tests/relay_instances.rs`) and by the local smoke
//! test (`examples/replay_proxy.rs`).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The largest request or response head carried.
const MAX_HEAD: usize = 64 * 1024;

pub struct ReplayProxy {
    pub addr: SocketAddr,
    state: Arc<State>,
    serving: tokio::task::JoinHandle<()>,
}

struct State {
    /// Each instance's name, and where it listens.
    instances: Vec<(String, SocketAddr)>,
    /// Where new requests go: the instance named, or each in turn.
    land_on: Mutex<Option<String>>,
    turn: AtomicUsize,
    replays: AtomicUsize,
    /// Print where each request went.
    verbose: bool,
}

impl ReplayProxy {
    pub async fn start(instances: Vec<(String, SocketAddr)>) -> Self {
        Self::bind("127.0.0.1:0".parse().unwrap(), instances, false)
            .await
            .unwrap()
    }

    /// On `addr`, printing where each request went if `verbose`.
    pub async fn bind(
        addr: SocketAddr,
        instances: Vec<(String, SocketAddr)>,
        verbose: bool,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            instances,
            land_on: Mutex::new(None),
            turn: AtomicUsize::new(0),
            replays: AtomicUsize::new(0),
            verbose,
        });
        let serving = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Err(error) = carry(&state, client).await {
                            eprintln!("replay proxy: {error}");
                        }
                    });
                }
            }
        });
        Ok(Self {
            addr,
            state,
            serving,
        })
    }

    pub fn base(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Send new requests to `instance`, or to each in turn with `None`.
    pub fn land_on(&self, instance: Option<&str>) {
        *self.state.land_on.lock().unwrap() = instance.map(str::to_owned);
    }

    /// How many answers said `fly-replay` so far.
    pub fn replays(&self) -> usize {
        self.state.replays.load(Ordering::SeqCst)
    }
}

impl Drop for ReplayProxy {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

impl State {
    fn chosen(&self) -> String {
        if let Some(name) = self.land_on.lock().unwrap().clone() {
            return name;
        }
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        self.instances[turn % self.instances.len()].0.clone()
    }

    fn addr(&self, name: &str) -> Option<SocketAddr> {
        self.instances
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, addr)| *addr)
    }
}

/// One client connection: its request to an instance, sent on as the answer says, then the answer
/// back, and everything after it both ways.
async fn carry(state: &State, mut client: TcpStream) -> std::io::Result<()> {
    let (head, mut rest) = read_head(&mut client).await?;
    let mut lines: Vec<String> = head.split("\r\n").map(str::to_owned).collect();
    let upgrade = header(&lines, "upgrade").is_some();
    let length: usize = header(&lines, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while rest.len() < length {
        let mut buffer = vec![0; length - rest.len()];
        let read = client.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        rest.extend_from_slice(&buffer[..read]);
    }
    if !upgrade {
        // One request per connection, so every request is the proxy's to place.
        lines.retain(|line| !line.to_ascii_lowercase().starts_with("connection:"));
        lines.push("Connection: close".into());
    }

    // The instance chosen, or the next that can be reached, as a proxy skips one that is down.
    let chosen = state.chosen();
    let mut order: Vec<String> = state.instances.iter().map(|(n, _)| n.clone()).collect();
    order.sort_by_key(|name| *name != chosen);
    let mut landed = None;
    for name in order {
        if let Some(sent) = send(state, &name, &lines, &rest).await? {
            landed = Some((name, sent));
            break;
        }
    }
    let Some((first, (mut instance, mut answer, mut after))) = landed else {
        return answer_bad_gateway(&mut client).await;
    };
    let mut went = first.clone();
    let answer_lines: Vec<&str> = answer.split("\r\n").collect();
    let replay = header(&answer_lines, "fly-replay").map(str::to_owned);
    if let Some(replay) = replay {
        state.replays.fetch_add(1, Ordering::SeqCst);
        let field = |name: &str| {
            replay
                .split(';')
                .filter_map(|f| f.trim().split_once('='))
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_owned())
        };
        let (target, preferred) = match (field("instance"), field("prefer_instance")) {
            (Some(target), _) => (target, false),
            (None, Some(target)) => (target, true),
            (None, None) => return answer_bad_gateway(&mut client).await,
        };
        lines.push(format!("fly-replay-src: instance={first}"));
        match send(state, &target, &lines, &rest).await? {
            Some(sent) => {
                (instance, answer, after) = sent;
                went = format!("{first}, sent on to {target}");
            }
            None if preferred => {
                let other = state
                    .instances
                    .iter()
                    .map(|(name, _)| name.clone())
                    .find(|name| *name != target);
                lines.push(format!("fly-preferred-instance-unavailable: {target}"));
                match other {
                    Some(other) => match send(state, &other, &lines, &rest).await? {
                        Some(sent) => {
                            (instance, answer, after) = sent;
                            went = format!("{first}, sent on to {target}, down, so {other}");
                        }
                        None => return answer_bad_gateway(&mut client).await,
                    },
                    None => return answer_bad_gateway(&mut client).await,
                }
            }
            None => return answer_bad_gateway(&mut client).await,
        }
    }
    if state.verbose {
        // The request line without its query, and the answer's status: never a header.
        let request = lines[0].split('?').next().unwrap_or_default();
        let request = request.trim_end_matches(" HTTP/1.1");
        let status = answer.split(' ').nth(1).unwrap_or("?");
        println!("{request} → {went}: {status}");
    }
    client.write_all(answer.as_bytes()).await?;
    client.write_all(b"\r\n\r\n").await?;
    client.write_all(&after).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut instance).await;
    Ok(())
}

/// Send the request to `name`: its connection, its answer's head, and what came after the head.
/// `None` if it cannot be reached.
async fn send(
    state: &State,
    name: &str,
    lines: &[String],
    body: &[u8],
) -> std::io::Result<Option<(TcpStream, String, Vec<u8>)>> {
    let Some(addr) = state.addr(name) else {
        return Ok(None);
    };
    let Ok(mut instance) = TcpStream::connect(addr).await else {
        return Ok(None);
    };
    let mut request = lines.join("\r\n").into_bytes();
    request.extend_from_slice(b"\r\n\r\n");
    request.extend_from_slice(body);
    instance.write_all(&request).await?;
    match read_head(&mut instance).await {
        Ok((head, after)) => Ok(Some((instance, head, after))),
        Err(_) => Ok(None),
    }
}

/// A message's head, without its blank line, and whatever was read past it.
async fn read_head(stream: &mut (impl AsyncRead + Unpin)) -> std::io::Result<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
            return Ok((head, buffer[end + 4..].to_vec()));
        }
        if buffer.len() > MAX_HEAD {
            return Err(std::io::Error::other("head too large"));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn header<'a>(lines: &'a [impl AsRef<str>], name: &str) -> Option<&'a str> {
    lines.iter().skip(1).find_map(|line| {
        let (key, value) = line.as_ref().split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

async fn answer_bad_gateway(client: &mut TcpStream) -> std::io::Result<()> {
    client
        .write_all(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
        .await
}

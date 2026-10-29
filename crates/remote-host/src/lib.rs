//! The host's outbound connector: this computer's link to its server's relay, and the devices
//! that come through it.
//!
//! [`Connector::start`] keeps one link open for as long as Remote is on: it asks for a host
//! ticket, opens the relay, and serves each device the relay connects as a channel of its own
//! ([`yardsort_core::remote::serve::Visitor`]). When the link fails — the network, the relay, a
//! server restart — it closes every channel and connects again, waiting longer each time up to
//! [`MAX_BACKOFF`].
//!
//! **The terminals outlive all of it.** They belong to the daemon. Closing a channel detaches
//! what its device watched and releases what it controlled; nothing else changes, and the person
//! here can type on as before. Nothing a device typed is ever sent again after a reconnect.
//!
//! **Nothing waits on a slow device.** Each channel's output is bounded
//! ([`remote_proto::MAX_QUEUED_OUTPUT`]): past it, the stream is detached and the device told to
//! attach again. Each channel's requests are answered on a thread of its own, in order, so a
//! device's slow request holds up only that device.
//!
//! **Grants are checked here too.** A channel opens only when this host's record has the grant
//! the relay names, every request looks it up again, and every [`RECHECK_EVERY`] — or at once,
//! with [`Connector::recheck`] — a channel whose grant is gone here is closed.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use remote_client::{Client, ClientError, Incoming, RelayLink};
use remote_proto::account::Secret;
use remote_proto::relay::{CloseReason, LinkFrame, TicketRequest};
use remote_proto::{ErrorCode, HostId, MAX_QUEUED_OUTPUT};
use tokio::sync::{mpsc as tokio_mpsc, Notify};
use yardsort_core::remote::serve::{Context, Outbound, Visitor};

/// How often every channel's grant is checked against this host's record.
pub const RECHECK_EVERY: Duration = Duration::from_secs(5);
/// The longest wait between attempts to connect.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The first wait after a link fails.
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
/// A link that lasted this long was a good one: the next failure starts the waits over.
const STEADY: Duration = Duration::from_secs(60);

/// Why an access credential could not be had.
#[derive(Debug, Clone)]
pub enum AccessError {
    /// Signed out, or the host is no longer registered: trying again will not help.
    SignedOut(String),
    /// Anything else, such as the network: try again later.
    Unavailable(String),
}

pub type AccessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(Client, Secret), AccessError>> + Send + 'a>>;

/// The host's credentials, as the app or a headless host keeps them.
pub trait Access: Send + Sync + 'static {
    /// The server's client and an access credential good for at least a minute.
    fn access(&self) -> AccessFuture<'_>;
}

/// This host's records and terminals, as a channel needs them.
pub trait Serving: Send + Sync + 'static {
    /// Run `f` with what a channel answers from. Blocking: called on a channel's own thread.
    fn with(&self, f: &mut dyn FnMut(&Context<'_>));
}

/// Where the link stands, for the person here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected {
        devices: usize,
    },
    /// Not connected; trying again in `retry_in`.
    Waiting {
        why: String,
        retry_in: Duration,
    },
    /// Stopped for good: signed out, or the host was removed.
    Stopped {
        why: String,
    },
}

/// The running connector. Dropping it stops it.
pub struct Connector {
    status: Arc<Mutex<Status>>,
    recheck: Arc<Notify>,
    stop: Arc<Notify>,
    task: Option<tokio::task::JoinHandle<()>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connector {
    /// Start keeping `host`'s link, on the current Tokio runtime.
    pub fn start(host: HostId, access: Arc<dyn Access>, serving: Arc<dyn Serving>) -> Self {
        let status = Arc::new(Mutex::new(Status::Connecting));
        let recheck = Arc::new(Notify::new());
        let stop = Arc::new(Notify::new());
        let task = tokio::spawn(keep(
            host,
            access,
            serving,
            Arc::clone(&status),
            Arc::clone(&recheck),
            Arc::clone(&stop),
        ));
        Self {
            status,
            recheck,
            stop,
            task: Some(task),
        }
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    /// Check every channel's grant now: one was just revoked here.
    pub fn recheck(&self) {
        self.recheck.notify_one();
    }

    /// Close the link and every channel, and stop.
    pub async fn stop(mut self) {
        self.stop.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Connector {
    fn drop(&mut self) {
        self.stop.notify_one();
    }
}

async fn keep(
    host: HostId,
    access: Arc<dyn Access>,
    serving: Arc<dyn Serving>,
    status: Arc<Mutex<Status>>,
    recheck: Arc<Notify>,
    stop: Arc<Notify>,
) {
    let set = |s: Status| *lock(&status) = s;
    let mut backoff = FIRST_BACKOFF;
    loop {
        set(Status::Connecting);
        let opened = tokio::select! {
            opened = open(&host, access.as_ref()) => opened,
            () = stop.notified() => return,
        };
        let why = match opened {
            Ok(link) => {
                let since = Instant::now();
                let reason = serve(link, &serving, &status, &recheck, &stop).await;
                if reason.is_none() {
                    return;
                }
                if since.elapsed() >= STEADY {
                    backoff = FIRST_BACKOFF;
                }
                format!(
                    "The relay closed the link: {}.",
                    reason.map_or("", CloseReason::text)
                )
            }
            Err(AccessError::SignedOut(why)) => {
                set(Status::Stopped { why });
                return;
            }
            Err(AccessError::Unavailable(why)) => why,
        };
        set(Status::Waiting {
            why,
            retry_in: backoff,
        });
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            () = stop.notified() => return,
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn open(host: &HostId, access: &dyn Access) -> Result<RelayLink, AccessError> {
    let (client, secret) = access.access().await?;
    let request = TicketRequest::Host { host: host.clone() };
    client
        .open_relay(&secret, &request)
        .await
        .map_err(|error| match error.code() {
            Some(ErrorCode::Unauthenticated | ErrorCode::NotFound | ErrorCode::Forbidden) => {
                AccessError::SignedOut(error.to_string())
            }
            _ => AccessError::Unavailable(error.to_string()),
        })
}

impl From<ClientError> for AccessError {
    fn from(error: ClientError) -> Self {
        match error.code() {
            Some(ErrorCode::Unauthenticated) => Self::SignedOut(error.to_string()),
            _ => Self::Unavailable(error.to_string()),
        }
    }
}

/// Something to write on the link, and the channel output it counts against.
struct Out {
    frame: Vec<u8>,
    counted: Option<(Arc<AtomicUsize>, usize)>,
}

/// A channel's way to the link.
struct ChannelOut {
    channel: u32,
    link: tokio_mpsc::UnboundedSender<Out>,
    queued: Arc<AtomicUsize>,
}

impl Outbound for ChannelOut {
    fn text(&self, text: String) {
        let frame = LinkFrame::Text(self.channel, text.as_bytes()).encode();
        let _ = self.link.send(Out {
            frame,
            counted: None,
        });
    }

    fn output(&self, frame: Vec<u8>) -> bool {
        let size = frame.len();
        if self.queued.load(Ordering::SeqCst) + size > MAX_QUEUED_OUTPUT {
            return false;
        }
        self.queued.fetch_add(size, Ordering::SeqCst);
        let frame = LinkFrame::Binary(self.channel, &frame).encode();
        self.link
            .send(Out {
                frame,
                counted: Some((Arc::clone(&self.queued), size)),
            })
            .is_ok()
    }
}

/// What a channel's thread is handed.
enum Inbox {
    Text(String),
    Recheck,
}

/// Serve one link until it closes (`Some(why)`) or the connector stops (`None`).
async fn serve(
    link: RelayLink,
    serving: &Arc<dyn Serving>,
    status: &Mutex<Status>,
    recheck: &Notify,
    stop: &Notify,
) -> Option<CloseReason> {
    let RelayLink {
        mut reader,
        mut writer,
        ..
    } = link;
    let (to_link, mut outgoing) = tokio_mpsc::unbounded_channel::<Out>();
    let writing = tokio::spawn(async move {
        while let Some(out) = outgoing.recv().await {
            let sent = writer.binary(out.frame).await;
            if let Some((queued, size)) = out.counted {
                queued.fetch_sub(size, Ordering::SeqCst);
            }
            if sent.is_err() {
                break;
            }
        }
        writer.close().await;
    });

    let mut channels: HashMap<u32, mpsc::Sender<Inbox>> = HashMap::new();
    let mut threads = Vec::new();
    let mut round = tokio::time::interval(RECHECK_EVERY);
    let connected = |n: usize| *lock(status) = Status::Connected { devices: n };
    connected(0);
    let ended = loop {
        tokio::select! {
            incoming = reader.next() => {
                let bytes = match incoming {
                    Ok(Incoming::Binary(bytes)) => bytes,
                    Ok(Incoming::Text(_)) => break Some(CloseReason::Invalid),
                    Err(reason) => break Some(reason),
                };
                let Ok(frame) = LinkFrame::decode(&bytes) else {
                    break Some(CloseReason::Invalid);
                };
                match frame {
                    LinkFrame::Open(channel, open) => {
                        let (inbox, messages) = mpsc::channel();
                        channels.insert(channel, inbox);
                        let out = ChannelOut {
                            channel,
                            link: to_link.clone(),
                            queued: Arc::new(AtomicUsize::new(0)),
                        };
                        let serving = Arc::clone(serving);
                        let to_link = to_link.clone();
                        threads.push(std::thread::Builder::new()
                            .name(format!("remote-channel-{channel}"))
                            .spawn(move || visit(channel, open, out, messages, serving, to_link))
                            .expect("a thread for a channel"));
                    }
                    LinkFrame::Close(channel, _) => {
                        channels.remove(&channel);
                    }
                    LinkFrame::Text(channel, text) => {
                        let text = String::from_utf8_lossy(text).into_owned();
                        if let Some(inbox) = channels.get(&channel) {
                            if inbox.send(Inbox::Text(text)).is_err() {
                                channels.remove(&channel);
                            }
                        }
                    }
                    // Nothing a device sends is binary; the relay does not pass it on.
                    LinkFrame::Binary(..) => {}
                }
                connected(channels.len());
            }
            _ = round.tick() => {
                channels.retain(|_, inbox| inbox.send(Inbox::Recheck).is_ok());
                connected(channels.len());
            }
            () = recheck.notified() => {
                channels.retain(|_, inbox| inbox.send(Inbox::Recheck).is_ok());
            }
            () = stop.notified() => break None,
        }
    };
    // Every channel closes, letting go of what it watched and controlled, before this returns:
    // a reconnect starts with nothing held.
    drop(channels);
    drop(to_link);
    let _ = tokio::task::spawn_blocking(move || {
        for thread in threads {
            let _ = thread.join();
        }
    })
    .await;
    writing.abort();
    ended
}

/// One channel, on its own thread: open it, answer it in order, close it.
fn visit(
    channel: u32,
    open: remote_proto::relay::ChannelOpen,
    out: ChannelOut,
    messages: mpsc::Receiver<Inbox>,
    serving: Arc<dyn Serving>,
    to_link: tokio_mpsc::UnboundedSender<Out>,
) {
    let close = |reason: CloseReason| {
        let _ = to_link.send(Out {
            frame: LinkFrame::Close(channel, reason).encode(),
            counted: None,
        });
    };
    let mut opened = None;
    serving.with(&mut |context| {
        opened = Some(Visitor::open(
            context.store,
            open.device.clone(),
            open.grant.clone(),
            Arc::new(ChannelOut {
                channel: out.channel,
                link: out.link.clone(),
                queued: Arc::clone(&out.queued),
            }),
        ));
    });
    let visitor = match opened {
        Some(Ok(visitor)) => visitor,
        Some(Err(reason)) => return close(reason),
        None => return close(CloseReason::Gone),
    };
    while let Ok(message) = messages.recv() {
        let mut outcome = Ok(());
        serving.with(&mut |context| {
            outcome = match &message {
                Inbox::Text(text) => visitor.handle(context, text),
                Inbox::Recheck if !visitor.allowed(context.store) => Err(CloseReason::Revoked),
                Inbox::Recheck => Ok(()),
            };
        });
        if let Err(reason) = outcome {
            close(reason);
            break;
        }
    }
    serving.with(&mut |context| visitor.close(context.terminals));
}

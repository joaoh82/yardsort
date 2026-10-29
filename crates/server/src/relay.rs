//! The relay: carries a device's traffic to its host and back, both connected out to here. See
//! [`remote_proto::relay`] for the messages and the framing.
//!
//! **Who gets through.** A ticket is issued only to a signed-in device: for a host's link, to the
//! device the host is signed in as; for a device, only with a grant on that host in force here.
//! What a ticket was issued for is all the socket that presents it can do. The host then checks
//! its own record before it answers anything ([`ChannelOpen`] names the grant).
//!
//! **Revocation reaches live sockets.** Every connected account is checked again every
//! [`RECHECK_EVERY`], and at once when anything revokes something: a device, a grant, a host, a
//! grant replaced by a new approval, a device signed out for reusing its refresh credential, or
//! the operator's `revoke-account`. Whatever instance it happened on signals every instance
//! through the records ([`Signal`]), and each checks the sockets it holds; the rounds are what
//! catches a signal that did not arrive. A socket whose device, grant or host is no longer in
//! force is closed with [`CloseReason::Revoked`], and its host is told to drop the channel.
//!
//! **Several instances.** A server can run as several instances behind one address, and a
//! device's socket has to reach the one holding its host's link. Each instance notes in the
//! records where the links it holds are ([`LinkPlace`]), and notes them again every
//! [`RECHECK_EVERY`]. Behind a proxy that can send a request on to a named instance
//! ([`Routing::FlyReplay`]), a device's ticket request that reaches another instance is sent to
//! the link's, which issues the ticket with its own name as the route; the socket names the
//! route, and is sent there too. A link that a later one of the same host replaced on another
//! instance is closed at the next round. Without such a proxy ([`Routing::Single`]) the server
//! must run as one instance.
//!
//! **A host hears of a device asking to pair** on its link's own channel ([`HostNotice`]), at
//! once: from the instance the device asked through, or by its [`Signal`] from another. Each
//! round also looks for requests waiting, in case a signal went missing, and a host that just
//! connected is told of what waits for it. Each
//! request is told once per link. The host then asks the server, as it would anyway.
//!
//! **Nothing waits on a slow socket.** What is to be sent to each socket waits in its own queue,
//! bounded by bytes ([`MAX_QUEUED`]). A device that falls that far behind is closed with
//! [`CloseReason::Overloaded`] and attaches again for a snapshot; its host and the other devices
//! carry on. Reading and writing are separate tasks, so a socket that does not read never stops
//! the relay from reading it, or anything else.
//!
//! **Nothing is kept.** Tickets and queues live in this process's memory and go with the
//! socket, or the process; the records hold only which instance holds which host's link. What a
//! socket carries is never logged: only who connected, to which host, and why a socket closed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::relay::{
    valid_route, Channel, ChannelOpen, CloseReason, Greeting, HostNotice, LinkFrame, Presented,
    RelayTicket, TicketRequest, FIRST_MESSAGE_MS, MAX_DEVICES_PER_HOST, MAX_QUEUED, NOTICE_CHANNEL,
    PING_EVERY_MS, SILENCE_MS, TICKET_TTL_MS,
};
use remote_proto::{DeviceId, ErrorCode, GrantId, HostId, RemoteError};
use tokio::sync::{broadcast, Notify};

use crate::http::now_ms;
use crate::records::{AccountId, GrantFilter, GrantStatus, LinkPlace, Records, Signal};
use crate::service::unreadable;
use crate::tokens::{self, Kind, TokenHash};
use crate::{Caller, ControlService};

/// How often every connected account is checked again.
pub const RECHECK_EVERY: Duration = Duration::from_secs(5);
/// How long a closing socket has to take its close frame.
const CLOSING: Duration = Duration::from_secs(5);
/// Tickets waiting to be presented, across all accounts. Past this, new ones are refused until
/// some are spent or lapse.
pub const MAX_PENDING_TICKETS: usize = 10_000;

/// How long a link's place is believed without its instance noting it again: several rounds of
/// [`RECHECK_EVERY`], so one slow round does not lose it.
pub const PLACE_LAPSES_AFTER_MS: i64 = 30_000;

/// The name of one instance of the server, as the proxy in front of them knows it: on Fly.io, the
/// Machine's id. Letters, digits, `-` and `_`, as [`valid_route`] says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Instance(String);

impl Instance {
    pub fn new(name: impl Into<String>) -> Result<Self, String> {
        let name = name.into();
        if valid_route(&name) {
            Ok(Self(name))
        } else {
            Err(format!(
                "{name:?} is not an instance name: letters, digits, - and _ only"
            ))
        }
    }

    /// A name of its own, for an instance nobody named.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a request that reached one instance gets to the instance it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Routing {
    /// It does not: the server runs as one instance, and everything is here.
    #[default]
    Single,
    /// Fly.io's proxy: an answer carrying `fly-replay` is not passed on, and the request goes
    /// again to the instance it names.
    FlyReplay,
}

impl Routing {
    /// Whether the proxy already sent this request on once. It is then answered here, whatever
    /// the records say, so a request is never sent round in circles.
    pub fn replayed(self, headers: &HeaderMap) -> bool {
        match self {
            Self::Single => false,
            Self::FlyReplay => {
                headers.contains_key("fly-replay-src")
                    || headers.contains_key("fly-preferred-instance-unavailable")
            }
        }
    }
}

/// A ticket request's answer: a ticket, or the instance to send the request on to.
#[derive(Debug)]
pub enum Ticketed {
    Here(RelayTicket),
    Elsewhere(Instance),
}

/// What a ticket lets its socket do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Role {
    Host {
        account: AccountId,
        host: HostId,
        device: DeviceId,
    },
    Device {
        account: AccountId,
        host: HostId,
        device: DeviceId,
        grant: GrantId,
    },
}

struct Pending {
    role: Role,
    expires_at: Instant,
}

/// The relay's state: tickets waiting, and the hosts connected with their devices.
#[derive(Clone)]
pub struct Relay {
    hub: Arc<Hub>,
}

impl Default for Relay {
    /// One instance, named at random.
    fn default() -> Self {
        Self::new(Instance::random(), Routing::Single)
    }
}

struct Hub {
    instance: Instance,
    routing: Routing,
    /// How often the rounds of checks come: [`RECHECK_EVERY`], but for tests.
    rounds: Duration,
    tickets: Mutex<HashMap<TokenHash, Pending>>,
    hosts: Mutex<HashMap<HostId, Arc<HostLink>>>,
    /// Accounts to check again now, rather than at the next round.
    dirty: Mutex<HashSet<AccountId>>,
    recheck: Notify,
    stopping: AtomicBool,
    next_link: AtomicU64,
}

struct HostLink {
    /// Tells this link from a later one of the same host.
    id: u64,
    /// The same, on every instance: as its [`LinkPlace`] names it.
    place: String,
    account: AccountId,
    host: HostId,
    device: DeviceId,
    out: Outbox,
    channels: Mutex<HashMap<Channel, Arc<DeviceLink>>>,
    next_channel: AtomicU32,
    /// The pairing requests this link was told of, so each is told once.
    told: Mutex<HashSet<GrantId>>,
}

struct DeviceLink {
    channel: Channel,
    device: DeviceId,
    grant: GrantId,
    out: Outbox,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Relay {
    pub fn new(instance: Instance, routing: Routing) -> Self {
        Self::with_rounds(instance, routing, RECHECK_EVERY)
    }

    /// With rounds of checks this far apart: long, for a test that shows something happened
    /// without one.
    pub fn with_rounds(instance: Instance, routing: Routing, rounds: Duration) -> Self {
        Self {
            hub: Arc::new(Hub {
                instance,
                routing,
                rounds,
                tickets: Mutex::default(),
                hosts: Mutex::default(),
                dirty: Mutex::default(),
                recheck: Notify::new(),
                stopping: AtomicBool::new(false),
                next_link: AtomicU64::new(0),
            }),
        }
    }

    /// This instance.
    pub fn instance(&self) -> &Instance {
        &self.hub.instance
    }

    pub fn routing(&self) -> Routing {
        self.hub.routing
    }

    /// Where a socket that names `route` belongs: `None` for here, or the instance to send it on
    /// to. A route that cannot name an instance is refused.
    pub fn socket_route(
        &self,
        route: Option<&str>,
        replayed: bool,
    ) -> Result<Option<Instance>, RemoteError> {
        let Some(route) = route else {
            return Ok(None);
        };
        let route = Instance::new(route).map_err(|_| {
            RemoteError::new(ErrorCode::InvalidRequest, "That route names no instance.")
        })?;
        let here = self.hub.routing == Routing::Single || replayed || route == self.hub.instance;
        Ok((!here).then_some(route))
    }

    /// Close every socket, for a restart. New sockets are refused from here on.
    pub fn shutdown(&self) {
        self.hub.stopping.store(true, Ordering::SeqCst);
        for link in lock(&self.hub.hosts).drain().map(|(_, link)| link) {
            link.close_all(CloseReason::Restarting);
            link.out.close(CloseReason::Restarting);
        }
        self.hub.recheck.notify_one();
    }

    /// Check `account`'s sockets again now: something of it was just revoked.
    pub fn nudge(&self, account: &AccountId) {
        lock(&self.hub.dirty).insert(account.clone());
        self.hub.recheck.notify_one();
    }

    /// Whether a host is connected now. For tests and for the host list's honesty.
    pub fn connected(&self, host: &HostId) -> bool {
        lock(&self.hub.hosts).contains_key(host)
    }

    fn issue(&self, role: Role) -> Result<RelayTicket, RemoteError> {
        let now = Instant::now();
        let mut tickets = lock(&self.hub.tickets);
        if tickets.len() >= MAX_PENDING_TICKETS {
            tickets.retain(|_, pending| pending.expires_at > now);
        }
        if tickets.len() >= MAX_PENDING_TICKETS {
            return Err(RemoteError::new(
                ErrorCode::RateLimited,
                "The relay is busy. Try again in a minute.",
            ));
        }
        let minted = tokens::mint(Kind::Ticket);
        tickets.insert(
            minted.hash,
            Pending {
                role,
                expires_at: now + Duration::from_millis(TICKET_TTL_MS),
            },
        );
        Ok(RelayTicket {
            ticket: minted.secret,
            expires_in_ms: TICKET_TTL_MS,
            route: (self.hub.routing != Routing::Single).then(|| self.hub.instance.0.clone()),
        })
    }

    /// Spend a presented ticket: what it was issued for, if it is one and still good.
    fn redeem(&self, ticket: &Secret) -> Option<Role> {
        let hash = tokens::presented(Kind::Ticket, ticket.expose())?;
        let pending = lock(&self.hub.tickets).remove(&hash)?;
        (pending.expires_at > Instant::now()).then_some(pending.role)
    }

    fn accounts(&self) -> Vec<AccountId> {
        let hosts = lock(&self.hub.hosts);
        let accounts: HashSet<_> = hosts.values().map(|l| l.account.clone()).collect();
        accounts.into_iter().collect()
    }

    fn links_of(&self, account: &AccountId) -> Vec<Arc<HostLink>> {
        lock(&self.hub.hosts)
            .values()
            .filter(|l| l.account == *account)
            .cloned()
            .collect()
    }

    /// Put `link` in place of any earlier link of the same host, which is closed.
    fn attach_host(&self, link: &Arc<HostLink>) {
        let earlier = lock(&self.hub.hosts).insert(link.host.clone(), Arc::clone(link));
        if let Some(earlier) = earlier {
            earlier.close_all(CloseReason::HostOffline);
            earlier.out.close(CloseReason::Replaced);
        }
    }

    /// Tell `host`'s link here, if there is one, that a device asks to pair through `grant`.
    pub fn tell_pairing(&self, host: &HostId, grant: &GrantId) {
        let link = lock(&self.hub.hosts).get(host).cloned();
        if let Some(link) = link {
            link.tell_pairing(grant);
        }
    }

    /// Take `link` away, if it is still the host's current one, and close its devices.
    fn detach_host(&self, link: &HostLink) {
        {
            let mut hosts = lock(&self.hub.hosts);
            if hosts.get(&link.host).is_some_and(|l| l.id == link.id) {
                hosts.remove(&link.host);
            }
        }
        link.close_all(CloseReason::HostOffline);
    }
}

impl HostLink {
    /// Tell the host of a pairing request, unless it was told already.
    fn tell_pairing(&self, grant: &GrantId) {
        if !lock(&self.told).insert(grant.clone()) {
            return;
        }
        let notice = HostNotice::PairingRequested {
            grant: grant.clone(),
        };
        let text = serde_json::to_vec(&notice).expect("serialises");
        self.out
            .push(link_message(&LinkFrame::Text(NOTICE_CHANNEL, &text)));
    }

    /// Close every device on this link, and forget them.
    fn close_all(&self, reason: CloseReason) {
        for device in lock(&self.channels).drain().map(|(_, d)| d) {
            device.out.close(reason);
        }
    }

    /// Close one device's channel, and tell the host.
    fn close_channel(&self, channel: Channel, reason: CloseReason) {
        if let Some(device) = lock(&self.channels).remove(&channel) {
            device.out.close(reason);
            self.out
                .push(link_message(&LinkFrame::Close(channel, reason)));
        }
    }
}

fn link_message(frame: &LinkFrame<'_>) -> Message {
    Message::Binary(frame.encode().into())
}

fn json_message(value: &impl serde::Serialize) -> Message {
    Message::Text(serde_json::to_string(value).expect("serialises").into())
}

/// What is to be sent to one socket: bounded by bytes, closed at most once.
struct Outbox {
    queue: Mutex<Queue>,
    ready: Notify,
}

#[derive(Default)]
struct Queue {
    messages: VecDeque<Message>,
    bytes: usize,
    close: Option<CloseReason>,
}

enum Next {
    Send(Message),
    Close(CloseReason),
}

impl Outbox {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Queue::default()),
            ready: Notify::new(),
        }
    }

    /// Queue `message`, unless the socket is closing or it would go over [`MAX_QUEUED`].
    fn push(&self, message: Message) -> bool {
        let size = message_len(&message);
        let mut queue = lock(&self.queue);
        if queue.close.is_some() || queue.bytes + size > MAX_QUEUED {
            return false;
        }
        queue.bytes += size;
        queue.messages.push_back(message);
        drop(queue);
        self.ready.notify_one();
        true
    }

    /// Close the socket for `reason`. What is still queued is dropped. The first reason given
    /// is the one that stands.
    fn close(&self, reason: CloseReason) {
        let mut queue = lock(&self.queue);
        if queue.close.is_none() {
            queue.close = Some(reason);
            queue.messages.clear();
            queue.bytes = 0;
        }
        drop(queue);
        self.ready.notify_one();
    }

    /// Why the socket is closing; `Gone` if nothing said.
    fn reason(&self) -> CloseReason {
        lock(&self.queue).close.unwrap_or(CloseReason::Gone)
    }

    /// Once the socket is closing: why.
    async fn closed(&self) -> CloseReason {
        loop {
            if let Some(reason) = lock(&self.queue).close {
                return reason;
            }
            self.ready.notified().await;
        }
    }

    async fn next(&self) -> Next {
        loop {
            {
                let mut queue = lock(&self.queue);
                if let Some(reason) = queue.close {
                    return Next::Close(reason);
                }
                if let Some(message) = queue.messages.pop_front() {
                    queue.bytes -= message_len(&message);
                    return Next::Send(message);
                }
            }
            self.ready.notified().await;
        }
    }
}

fn message_len(message: &Message) -> usize {
    match message {
        Message::Text(text) => text.len(),
        Message::Binary(bytes) => bytes.len(),
        _ => 0,
    }
}

impl<R: Records> ControlService<R> {
    /// `POST /v1/relay/tickets`. A device's ticket is issued by the instance holding its host's
    /// link: from another, the answer is to send the request there, unless it was sent on once
    /// already (`replayed`).
    pub async fn ticket(
        &self,
        caller: &Caller,
        request: &TicketRequest,
        replayed: bool,
        now: i64,
    ) -> Result<Ticketed, RemoteError> {
        if self.relay.hub.stopping.load(Ordering::SeqCst) {
            return Err(restarting());
        }
        let host = self
            .records
            .host(&caller.account, request.host())
            .await
            .map_err(unreadable)?
            .ok_or_else(RemoteError::not_found)?;
        let role = match request {
            TicketRequest::Host { .. } => {
                if caller.kind != DeviceKind::Host || host.device != caller.device {
                    return Err(RemoteError::not_found());
                }
                Role::Host {
                    account: caller.account.clone(),
                    host: host.id,
                    device: caller.device.clone(),
                }
            }
            TicketRequest::Device { .. } => {
                let grant = self
                    .records
                    .grants(
                        &caller.account,
                        GrantFilter::ForDevice(caller.device.clone()),
                    )
                    .await
                    .map_err(unreadable)?
                    .into_iter()
                    .find(|v| {
                        v.grant.host == host.id
                            && v.grant.status == GrantStatus::Approved
                            && !v.device_revoked
                    })
                    .ok_or_else(|| {
                        RemoteError::new(
                            ErrorCode::Forbidden,
                            "This device has no grant on that host. Pair with it first.",
                        )
                    })?;
                if let Some(there) = self.link_elsewhere(&host.id, replayed, now).await? {
                    return Ok(Ticketed::Elsewhere(there));
                }
                Role::Device {
                    account: caller.account.clone(),
                    host: host.id,
                    device: caller.device.clone(),
                    grant: grant.grant.id,
                }
            }
        };
        self.relay.issue(role).map(Ticketed::Here)
    }

    /// The other instance believed to hold `host`'s link, when requests can be sent there.
    async fn link_elsewhere(
        &self,
        host: &HostId,
        replayed: bool,
        now: i64,
    ) -> Result<Option<Instance>, RemoteError> {
        let hub = &self.relay.hub;
        if hub.routing == Routing::Single || replayed {
            return Ok(None);
        }
        let place = self.records.link_place(host).await.map_err(unreadable)?;
        Ok(place
            .filter(|p| now - p.seen_at <= PLACE_LAPSES_AFTER_MS && p.instance != hub.instance.0)
            .and_then(|p| Instance::new(p.instance).ok()))
    }

    /// Serve one relay socket, from its first message to its close.
    pub async fn serve_socket(self: Arc<Self>, socket: WebSocket) {
        let (mut sink, mut stream) = socket.split();
        if self.relay.hub.stopping.load(Ordering::SeqCst) {
            return refuse(&mut sink, restarting(), CloseReason::Restarting).await;
        }
        let first = tokio::time::timeout(Duration::from_millis(FIRST_MESSAGE_MS), stream.next());
        let presented = match first.await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Presented>(&text).ok(),
            _ => None,
        };
        let Some(role) = presented.and_then(|p| self.relay.redeem(&p.ticket)) else {
            return refuse(
                &mut sink,
                RemoteError::new(
                    ErrorCode::Unauthenticated,
                    "That ticket is not good. Ask for a new one.",
                ),
                CloseReason::Unauthenticated,
            )
            .await;
        };
        // Checked again as it connects: a ticket can outlive what it was issued for by a minute.
        let (Role::Host { account, .. } | Role::Device { account, .. }) = &role;
        self.relay.nudge(account);
        match role {
            Role::Host {
                account,
                host,
                device,
            } => {
                let link = Arc::new(HostLink {
                    id: self.relay.hub.next_link.fetch_add(1, Ordering::Relaxed),
                    place: uuid::Uuid::new_v4().simple().to_string(),
                    account,
                    host: host.clone(),
                    device,
                    out: Outbox::new(),
                    channels: Mutex::new(HashMap::new()),
                    // Channel 0 is the link's own, for notices: no device is given it.
                    next_channel: AtomicU32::new(NOTICE_CHANNEL + 1),
                    told: Mutex::new(HashSet::new()),
                });
                // Noted before it is attached, so that every round finds the place of each link it
                // judges (`keep_places`).
                let place = LinkPlace {
                    host: host.clone(),
                    instance: self.relay.hub.instance.0.clone(),
                    link: link.place.clone(),
                    seen_at: now_ms(),
                };
                if let Err(error) = self.records.put_link(place).await {
                    tracing::error!("relay: records: {error}");
                    return refuse(
                        &mut sink,
                        RemoteError::new(
                            ErrorCode::Internal,
                            "The server could not take the link. Connect again in a moment.",
                        ),
                        CloseReason::Restarting,
                    )
                    .await;
                }
                link.out
                    .push(json_message(&Greeting::Connected { host: host.clone() }));
                self.relay.attach_host(&link);
                tracing::info!(%host, "relay: host connected");
                // What already waits for it, rather than at the next round.
                tell_pending(&self).await;
                let reason = pump(sink, stream, &link.out, |message| {
                    host_message(&link, message)
                })
                .await;
                self.relay.detach_host(&link);
                if let Err(error) = self.records.drop_link(&host, &link.place).await {
                    tracing::error!("relay: records: {error}");
                }
                tracing::info!(%host, reason = reason.text(), "relay: host left");
            }
            Role::Device {
                account: _,
                host,
                device,
                grant,
            } => {
                let Some(link) = lock(&self.relay.hub.hosts).get(&host).cloned() else {
                    return refuse(
                        &mut sink,
                        RemoteError::new(
                            ErrorCode::HostOffline,
                            "That host is not connected. It may be asleep or offline.",
                        ),
                        CloseReason::HostOffline,
                    )
                    .await;
                };
                let channel = link.next_channel.fetch_add(1, Ordering::Relaxed);
                let joined = Arc::new(DeviceLink {
                    channel,
                    device: device.clone(),
                    grant: grant.clone(),
                    out: Outbox::new(),
                });
                let full = {
                    let mut channels = lock(&link.channels);
                    let full = channels.len() >= MAX_DEVICES_PER_HOST;
                    if !full {
                        channels.insert(channel, Arc::clone(&joined));
                    }
                    full
                };
                if full {
                    return refuse(
                        &mut sink,
                        RemoteError::new(
                            ErrorCode::RateLimited,
                            "Too many devices are connected to that host.",
                        ),
                        CloseReason::Overloaded,
                    )
                    .await;
                }
                joined
                    .out
                    .push(json_message(&Greeting::Connected { host: host.clone() }));
                let open = ChannelOpen {
                    device: device.clone(),
                    grant,
                };
                if !link.out.push(link_message(&LinkFrame::Open(channel, open))) {
                    joined.out.close(CloseReason::Overloaded);
                }
                tracing::info!(%host, %device, channel, "relay: device connected");
                let reason = pump(sink, stream, &joined.out, |message| {
                    device_message(&link, &joined, message)
                })
                .await;
                link.close_channel(channel, CloseReason::Gone);
                tracing::info!(%host, %device, channel, reason = reason.text(), "relay: device left");
            }
        }
    }
}

/// A message from a host: a frame for one of its channels.
fn host_message(link: &HostLink, message: Message) -> Result<(), CloseReason> {
    let Message::Binary(bytes) = message else {
        return Err(CloseReason::Invalid);
    };
    let frame = LinkFrame::decode(&bytes).map_err(|_| CloseReason::Invalid)?;
    let channel = frame.channel();
    // A channel the relay closed a moment ago: what the host sent on it is dropped.
    let Some(device) = lock(&link.channels).get(&channel).cloned() else {
        return match frame {
            LinkFrame::Open(..) => Err(CloseReason::Invalid),
            _ => Ok(()),
        };
    };
    let message = match frame {
        LinkFrame::Open(..) => return Err(CloseReason::Invalid),
        LinkFrame::Close(_, reason) => {
            lock(&link.channels).remove(&channel);
            device.out.close(match reason {
                CloseReason::Gone => CloseReason::Refused,
                other => other,
            });
            return Ok(());
        }
        LinkFrame::Text(_, text) => Message::Text(
            String::from_utf8(text.to_vec())
                .map_err(|_| CloseReason::Invalid)?
                .into(),
        ),
        LinkFrame::Binary(_, bytes) => Message::Binary(bytes.to_vec().into()),
    };
    if !device.out.push(message) {
        link.close_channel(channel, CloseReason::Overloaded);
    }
    Ok(())
}

/// A message from a device: text for its host, on its channel.
fn device_message(
    link: &HostLink,
    device: &DeviceLink,
    message: Message,
) -> Result<(), CloseReason> {
    let Message::Text(text) = message else {
        return Err(CloseReason::Invalid);
    };
    if !link.out.push(link_message(&LinkFrame::Text(
        device.channel,
        text.as_bytes(),
    ))) {
        // The host is not keeping up. This device waits for nothing: it goes, and tries again.
        return Err(CloseReason::Overloaded);
    }
    Ok(())
}

/// Carry a socket until it closes: what `out` holds is written by a task of its own, and what
/// arrives is handed to `on_message`. Returns why it closed.
async fn pump(
    mut sink: Sink,
    mut stream: futures_util::stream::SplitStream<WebSocket>,
    out: &Outbox,
    mut on_message: impl FnMut(Message) -> Result<(), CloseReason>,
) -> CloseReason {
    let writing = async {
        let mut ping = tokio::time::interval(Duration::from_millis(PING_EVERY_MS));
        ping.tick().await;
        loop {
            tokio::select! {
                next = out.next() => match next {
                    // A send to a socket that stopped reading can wait for ever: closing wins.
                    Next::Send(message) => tokio::select! {
                        sent = sink.send(message) => {
                            if sent.is_err() {
                                return CloseReason::Gone;
                            }
                        }
                        reason = out.closed() => return reason,
                    },
                    Next::Close(reason) => {
                        let _ = sink.send(close_message(reason)).await;
                        let _ = sink.close().await;
                        return reason;
                    }
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
                        return CloseReason::Gone;
                    }
                }
            }
        }
    };
    let reading = async {
        let silence = Duration::from_millis(SILENCE_MS);
        loop {
            match tokio::time::timeout(silence, stream.next()).await {
                Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) | Err(_) => {
                    out.close(CloseReason::Gone);
                    return;
                }
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                Ok(Some(Ok(message))) => {
                    if let Err(reason) = on_message(message) {
                        out.close(reason);
                        return;
                    }
                }
            }
        }
    };
    // Reading stops when the socket does, or breaks a rule; writing then sends the close. A
    // socket that stopped reading can leave writing stuck in a send: it gets a moment, not
    // longer, so the socket is let go of either way.
    tokio::pin!(writing);
    tokio::select! {
        reason = &mut writing => reason,
        () = reading => tokio::time::timeout(CLOSING, writing)
            .await
            .unwrap_or_else(|_| out.reason()),
    }
}

type Sink = futures_util::stream::SplitSink<WebSocket, Message>;

/// Say why a socket is not let in, and close it.
async fn refuse(sink: &mut Sink, error: RemoteError, reason: CloseReason) {
    let _ = sink.send(json_message(&Greeting::Refused(error))).await;
    let _ = sink.send(close_message(reason)).await;
    let _ = sink.close().await;
}

fn close_message(reason: CloseReason) -> Message {
    Message::Close(Some(CloseFrame {
        code: reason.code(),
        reason: reason.text().into(),
    }))
}

fn restarting() -> RemoteError {
    RemoteError::new(
        ErrorCode::HostOffline,
        "The server is restarting. Connect again in a moment.",
    )
}

/// Check `account`'s sockets against the records, and close those no longer in force.
async fn recheck<R: Records>(service: &ControlService<R>, account: &AccountId) {
    let links = service.relay.links_of(account);
    if links.is_empty() {
        return;
    }
    let records = &service.records;
    let devices = match records.devices(account).await {
        Ok(devices) => devices,
        Err(error) => {
            tracing::error!("relay: records: {error}");
            return;
        }
    };
    let signed_in = |id: &DeviceId| devices.iter().any(|d| d.id == *id && d.revoked.is_none());
    for link in links {
        let host = match records.host(account, &link.host).await {
            Ok(host) => host,
            Err(error) => {
                tracing::error!("relay: records: {error}");
                return;
            }
        };
        if !host.is_some_and(|h| h.device == link.device) || !signed_in(&link.device) {
            tracing::info!(host = %link.host, "relay: host no longer in force");
            link.close_all(CloseReason::Revoked);
            link.out.close(CloseReason::Revoked);
            continue;
        }
        let joined: Vec<_> = lock(&link.channels).values().cloned().collect();
        for device in joined {
            let view = match records
                .grants(account, GrantFilter::One(device.grant.clone()))
                .await
            {
                Ok(views) => views.into_iter().next(),
                Err(error) => {
                    tracing::error!("relay: records: {error}");
                    return;
                }
            };
            let in_force = view.is_some_and(|v| {
                v.grant.status == GrantStatus::Approved
                    && !v.device_revoked
                    && v.grant.device == device.device
                    && v.grant.host == link.host
            });
            if !in_force {
                tracing::info!(host = %link.host, device = %device.device, "relay: grant no longer in force");
                link.close_channel(device.channel, CloseReason::Revoked);
            }
        }
    }
}

/// Note again that this instance holds its links, and close each one a later link of the same
/// host took on another instance.
async fn keep_places<R: Records>(service: &ControlService<R>) {
    let hub = &service.relay.hub;
    // Taken before the records are read: each of these was noted before it was attached, so a
    // place missing from what comes back was taken since, or is going with its closing link.
    let here: Vec<Arc<HostLink>> = lock(&hub.hosts).values().cloned().collect();
    let kept = match service
        .records
        .keep_links(hub.instance.as_str(), now_ms())
        .await
    {
        Ok(kept) => kept,
        Err(error) => {
            tracing::error!("relay: records: {error}");
            return;
        }
    };
    for link in here {
        if !kept
            .iter()
            .any(|p| p.host == link.host && p.link == link.place)
        {
            tracing::info!(host = %link.host, "relay: host connected again elsewhere");
            link.close_all(CloseReason::HostOffline);
            link.out.close(CloseReason::Replaced);
        }
    }
}

/// Tell this instance's hosts of the pairing requests waiting for them that they were not told
/// of: those made through another instance, or before the host connected.
async fn tell_pending<R: Records>(service: &ControlService<R>) {
    let hub = &service.relay.hub;
    match service
        .records
        .pending_on_links(hub.instance.as_str(), now_ms())
        .await
    {
        Ok(pending) => {
            for (host, grant) in pending {
                service.relay.tell_pairing(&host, &grant);
            }
        }
        Err(error) => tracing::error!("relay: records: {error}"),
    }
}

/// Check every connected account every [`RECHECK_EVERY`], and nudged ones at once, until the
/// relay shuts down. Each round notes again where this instance's links are, and tells their
/// hosts of pairing requests made elsewhere.
pub(crate) fn start_rechecks<R: Records + 'static>(service: Arc<ControlService<R>>) {
    tokio::spawn(async move {
        let mut round = tokio::time::interval(service.relay.hub.rounds);
        let mut signals = Some(service.records.signals());
        loop {
            let heard = async {
                match &mut signals {
                    Some(signals) => Some(signals.recv().await),
                    None => std::future::pending().await,
                }
            };
            let accounts = tokio::select! {
                _ = round.tick() => {
                    keep_places(&service).await;
                    tell_pending(&service).await;
                    service.relay.accounts()
                }
                () = service.relay.hub.recheck.notified() => {
                    lock(&service.relay.hub.dirty).drain().collect()
                }
                heard = heard => match heard {
                    Some(Ok(Signal::Recheck(account))) => vec![account],
                    Some(Ok(Signal::Pairing(host, grant))) => {
                        service.relay.tell_pairing(&host, &grant);
                        Vec::new()
                    }
                    // Started listening, or missed some: everything, as a round does.
                    Some(Ok(Signal::RecheckAll) | Err(broadcast::error::RecvError::Lagged(_))) => {
                        tell_pending(&service).await;
                        service.relay.accounts()
                    }
                    Some(Err(broadcast::error::RecvError::Closed)) | None => {
                        signals = None;
                        Vec::new()
                    }
                },
            };
            if service.relay.hub.stopping.load(Ordering::SeqCst) {
                return;
            }
            for account in accounts {
                recheck(&service, &account).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role() -> Role {
        Role::Host {
            account: AccountId("a".into()),
            host: HostId::new("h").unwrap(),
            device: DeviceId::new("d").unwrap(),
        }
    }

    #[test]
    fn a_ticket_works_once_and_not_once_it_lapses() {
        let relay = Relay::default();
        let ticket = relay.issue(role()).unwrap();
        assert!(ticket.ticket.expose().starts_with("ys1.rl."));
        assert_eq!(relay.redeem(&ticket.ticket), Some(role()));
        assert_eq!(relay.redeem(&ticket.ticket), None, "spent");

        let late = relay.issue(role()).unwrap();
        for pending in lock(&relay.hub.tickets).values_mut() {
            pending.expires_at = Instant::now();
        }
        assert_eq!(relay.redeem(&late.ticket), None, "lapsed");
        assert!(
            lock(&relay.hub.tickets).is_empty(),
            "a lapsed one is spent too"
        );

        // Another kind of credential is no ticket, even if its hash were kept.
        let access = tokens::mint(Kind::Access);
        lock(&relay.hub.tickets).insert(
            access.hash,
            Pending {
                role: role(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        assert_eq!(relay.redeem(&access.secret), None);
    }

    #[test]
    fn a_ticket_names_its_instance_only_when_requests_can_be_sent_there() {
        let single = Relay::new(Instance::new("a").unwrap(), Routing::Single);
        assert_eq!(single.issue(role()).unwrap().route, None);
        let routed = Relay::new(Instance::new("a").unwrap(), Routing::FlyReplay);
        assert_eq!(routed.issue(role()).unwrap().route.as_deref(), Some("a"));
    }

    #[test]
    fn a_socket_is_sent_to_its_route_once() {
        let routed = Relay::new(Instance::new("a").unwrap(), Routing::FlyReplay);
        let b = Instance::new("b").unwrap();
        assert_eq!(routed.socket_route(None, false).unwrap(), None);
        assert_eq!(routed.socket_route(Some("a"), false).unwrap(), None, "here");
        assert_eq!(routed.socket_route(Some("b"), false).unwrap(), Some(b));
        assert_eq!(
            routed.socket_route(Some("b"), true).unwrap(),
            None,
            "sent on once already: answered here"
        );
        let error = routed.socket_route(Some("b/../c"), false).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);

        // With nothing to send it there, everything is answered here.
        let single = Relay::new(Instance::new("a").unwrap(), Routing::Single);
        assert_eq!(single.socket_route(Some("b"), false).unwrap(), None);
    }

    #[test]
    fn only_the_proxy_says_a_request_was_sent_on() {
        let mut headers = HeaderMap::new();
        assert!(!Routing::FlyReplay.replayed(&headers));
        headers.insert("fly-replay-src", "instance=b".parse().unwrap());
        assert!(Routing::FlyReplay.replayed(&headers));
        assert!(!Routing::Single.replayed(&headers), "nothing sends it on");
        let mut fallen_back = HeaderMap::new();
        fallen_back.insert("fly-preferred-instance-unavailable", "b".parse().unwrap());
        assert!(Routing::FlyReplay.replayed(&fallen_back));
    }

    #[test]
    fn tickets_waiting_are_bounded_and_lapsed_ones_make_room() {
        let relay = Relay::default();
        for _ in 0..MAX_PENDING_TICKETS {
            relay.issue(role()).unwrap();
        }
        let error = relay.issue(role()).unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        for pending in lock(&relay.hub.tickets).values_mut().take(10) {
            pending.expires_at = Instant::now();
        }
        relay.issue(role()).unwrap();
    }
}

//! The relay: carries a device's traffic to its host and back, both connected out to here. See
//! [`remote_proto::relay`] for the messages and the framing.
//!
//! **Who gets through.** A ticket is issued only to a signed-in device: for a host's link, to the
//! device the host is signed in as; for a device, only with a grant on that host in force here.
//! What a ticket was issued for is all the socket that presents it can do. The host then checks
//! its own record before it answers anything ([`ChannelOpen`] names the grant).
//!
//! **Revocation reaches live sockets.** Every connected account is checked again every
//! [`RECHECK_EVERY`], and at once when this process revokes something: a device, a grant, a
//! host, or a grant replaced by a new approval. A socket whose device, grant or host is no
//! longer in force is closed with [`CloseReason::Revoked`], and its host is told to drop the
//! channel. Another process's revocation (the operator's `revoke-account`) is seen at the next
//! check.
//!
//! **Nothing waits on a slow socket.** What is to be sent to each socket waits in its own queue,
//! bounded by bytes ([`MAX_QUEUED`]). A device that falls that far behind is closed with
//! [`CloseReason::Overloaded`] and attaches again for a snapshot; its host and the other devices
//! carry on. Reading and writing are separate tasks, so a socket that does not read never stops
//! the relay from reading it, or anything else.
//!
//! **Nothing is kept.** Tickets and queues live in this process's memory and go with the
//! socket, or the process. What a socket carries is never logged: only who connected, to which
//! host, and why a socket closed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::relay::{
    Channel, ChannelOpen, CloseReason, Greeting, LinkFrame, Presented, RelayTicket, TicketRequest,
    FIRST_MESSAGE_MS, MAX_DEVICES_PER_HOST, MAX_QUEUED, PING_EVERY_MS, SILENCE_MS, TICKET_TTL_MS,
};
use remote_proto::{DeviceId, ErrorCode, GrantId, HostId, RemoteError};
use tokio::sync::Notify;

use crate::records::{AccountId, GrantFilter, GrantStatus, Records};
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
#[derive(Clone, Default)]
pub struct Relay {
    hub: Arc<Hub>,
}

#[derive(Default)]
struct Hub {
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
    account: AccountId,
    host: HostId,
    device: DeviceId,
    out: Outbox,
    channels: Mutex<HashMap<Channel, Arc<DeviceLink>>>,
    next_channel: AtomicU32,
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
    /// `POST /v1/relay/tickets`.
    pub async fn ticket(
        &self,
        caller: &Caller,
        request: &TicketRequest,
    ) -> Result<RelayTicket, RemoteError> {
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
                Role::Device {
                    account: caller.account.clone(),
                    host: host.id,
                    device: caller.device.clone(),
                    grant: grant.grant.id,
                }
            }
        };
        self.relay.issue(role)
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
                    account,
                    host: host.clone(),
                    device,
                    out: Outbox::new(),
                    channels: Mutex::new(HashMap::new()),
                    next_channel: AtomicU32::new(1),
                });
                link.out
                    .push(json_message(&Greeting::Connected { host: host.clone() }));
                self.relay.attach_host(&link);
                tracing::info!(%host, "relay: host connected");
                let reason = pump(sink, stream, &link.out, |message| {
                    host_message(&link, message)
                })
                .await;
                self.relay.detach_host(&link);
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

/// Check every connected account every [`RECHECK_EVERY`], and nudged ones at once, until the
/// relay shuts down.
pub(crate) fn start_rechecks<R: Records + 'static>(service: Arc<ControlService<R>>) {
    tokio::spawn(async move {
        let mut round = tokio::time::interval(RECHECK_EVERY);
        loop {
            let accounts = tokio::select! {
                _ = round.tick() => service.relay.accounts(),
                () = service.relay.hub.recheck.notified() => {
                    lock(&service.relay.hub.dirty).drain().collect()
                }
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

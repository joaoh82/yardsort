//! This computer as a client of other hosts: a device of kind `computer` at a Yardsort server,
//! paired with hosts as a phone is, and showing what each lets it see. See
//! docs/design/35-remote-r4-r5.md §3, "The desktop as a client".
//!
//! **A device of its own.** Signing in here makes a second device beside this computer's host
//! device ([`crate::account`]), with its own refresh credential under [`refresh_entry`]: the server
//! lets only host devices register hosts, and only device tickets reach other hosts. Either can be
//! signed in or out without the other.
//!
//! **What it is let do is the host's person's to say.** It asks to watch and to type ([`ASKED`]),
//! as a phone does; the person at that host approves at their own screen or terminal, and may let
//! it only watch. Nothing here approves anything.
//!
//! **One link per host it may reach**, kept open while the process runs and opened again, waiting
//! longer each time, when it drops. Through each, what the host shows is read again when its
//! activity moves and every [`REREAD_EVERY`], and kept in memory for the window to show,
//! with the link's state: online, connecting, or offline since when. What was last read stays shown
//! while the host is away, said to be from then. Nothing a host sends is written to disk here.
//!
//! **A terminal is watched through the same link** ([`terminal`]): attached, attached again after
//! the link drops, and typed into only once its person here takes control. Work is started there
//! through it too ([`start`]), by that host's ids.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod start;
pub mod terminal;

use remote_client::channel::Followed;
use remote_client::{Channel, Client, Session};
use remote_proto::account::{Credentials, Secret, ServerDescription};
use remote_proto::pairing::{normalize_code, GrantState, GrantSummary, OfferKey, PairingRequest};
use remote_proto::{
    ActivityEvent, ActivityListParams, ActivitySubscribeParams, ErrorCode, GrantId,
    HostDescription, HostId, ProjectSummary, Scope, SessionListParams, SessionPage,
    WorkspaceListParams, WorkspaceSummary, MAX_PAGE,
};
use yardsort_core::remote::host::CredentialStore;
use yardsort_core::store::{RemoteComputerRow, Store};

use crate::account::{failed, now_ms, refused_here, Failure, Result};

/// What this computer asks a host for: all it uses, as a phone asks. The host's person may let it
/// only watch.
pub const ASKED: [Scope; 5] = [
    Scope::WorkspaceRead,
    Scope::ActivityRead,
    Scope::MemoryRead,
    Scope::TerminalRead,
    Scope::TerminalControl,
];
/// How often what a host shows is read again, when its activity has not moved.
pub const REREAD_EVERY: Duration = Duration::from_secs(20);
/// The newest of a host's activity kept to show.
pub const ACTIVITY_SHOWN: usize = 50;
/// The workspaces of a host whose conversations are read, the first on its list.
const WORKSPACES_READ: usize = 50;
/// The grants are asked for again at most this often when the window looks.
const RELIST_EVERY: Duration = Duration::from_secs(10);
/// How often the grants are asked for again while one waits for the host's person: the answer
/// is given there, and nothing tells this computer of it.
pub const WAITING_RELIST: Duration = Duration::from_secs(3);
/// How often they are asked for again otherwise, so one taken back is seen to be.
pub const IDLE_RELIST: Duration = Duration::from_secs(60);
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// A link that lasted this long was a good one: the next failure starts the waits over.
const STEADY: Duration = Duration::from_secs(60);

/// Where this computer's client refresh credential is kept: apart from the host's, per device.
pub fn refresh_entry(device: &str) -> String {
    format!("remote-computer-refresh-{}", device.to_ascii_lowercase())
}

/// Where a client computer keeps what it is, and how the window hears that something changed.
pub trait ComputerHome: Send + Sync + 'static {
    fn computer(&self) -> &Computer;
    fn store(&self) -> &Store;
    /// Where the client's refresh credential is kept.
    fn credentials(&self) -> Result<Box<dyn CredentialStore>>;
    /// Who this is, for a host's list of devices: `yardsort-desktop/0.23.0 (linux)`.
    fn client_name(&self) -> String;
    /// A host's link or what it shows changed, or the list of hosts did.
    fn changed(&self);
}

/// The session, the hosts paired with, and a link to each it may reach.
#[derive(Default)]
pub struct Computer {
    session: tokio::sync::Mutex<Option<Session>>,
    known: Mutex<Known>,
    /// Wakes the task that lists the hosts again ([`relist`]).
    relist: tokio::sync::Notify,
    /// Wakes every link to read what its host shows again: something was started there.
    reread: tokio::sync::Notify,
    lister: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Default)]
struct Known {
    grants: Vec<GrantSummary>,
    listed_at: Option<Instant>,
    /// Why the hosts could not be listed, when they could not.
    problem: Option<Failure>,
    links: HashMap<HostId, HostLink>,
}

struct HostLink {
    view: Arc<Mutex<HostView>>,
    /// The channel while it is open, for the terminals watched through it.
    channel: Open,
    task: tokio::task::JoinHandle<()>,
}

/// The channel to a host while it is open, for the terminals watched through it to wait on.
type Open = tokio::sync::watch::Sender<Option<Arc<Channel>>>;

impl Drop for HostLink {
    fn drop(&mut self) {
        self.task.abort();
        // A terminal watched through it holds it too: it closes now, and nothing opens another.
        if let Some(channel) = self.channel.send_replace(None) {
            channel.close();
        }
    }
}

/// How a host's link stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Online,
    /// Since when, and why, in a few words.
    Offline {
        since_ms: i64,
        why: String,
    },
}

/// A host as this computer last saw it.
#[derive(Debug, Clone)]
pub struct HostView {
    pub link: Link,
    /// What it showed when last read; `None` until it has been read once.
    pub shown: Option<Shown>,
}

/// What a host showed, when it was read.
#[derive(Debug, Clone)]
pub struct Shown {
    pub host: HostDescription,
    pub projects: Vec<ProjectSummary>,
    /// Each workspace, with its conversations and other terminals.
    pub workspaces: Vec<(WorkspaceSummary, SessionPage)>,
    /// The newest first, at most [`ACTIVITY_SHOWN`].
    pub activity: Vec<ActivityEvent>,
    pub read_at_ms: i64,
}

/// A host this computer asked to be let in to, by the server's word, with what it shows.
#[derive(Debug, Clone)]
pub struct Paired {
    pub grant: GrantSummary,
    /// For an approved grant: its link and what it last showed.
    pub view: Option<HostView>,
}

/// Signed in at `server`: keep it, as this computer's client device.
pub async fn sign_in<H: ComputerHome>(
    home: &Arc<H>,
    client: Client,
    server: &ServerDescription,
    name: &str,
    signed_in: Credentials,
) -> Result<RemoteComputerRow> {
    // Kept before anything else: from here on, it is the only way back in.
    keep_refresh(home.as_ref(), signed_in.device.as_str(), &signed_in.refresh)
        .map_err(|e| Failure::new("remote_credentials", e))?;
    let earlier = home.store().remote_computer().map_err(store_failed)?;
    let row = RemoteComputerRow {
        server_url: client.address().as_str().trim_end_matches('/').to_owned(),
        server_id: server.server.to_string(),
        device_id: signed_in.device.to_string(),
        name: name.to_owned(),
        signed_in_at: now_ms(),
    };
    let mut session = Session::new(client.clone(), signed_in);
    if let Some(earlier) = earlier.filter(|e| e.device_id != row.device_id) {
        // The device it was before is of no use now. At the same server, it is signed out there.
        if earlier.server_id == row.server_id {
            if let (Ok(device), Ok(access)) = (
                remote_proto::DeviceId::new(earlier.device_id.clone()),
                access(home.as_ref(), &mut session).await,
            ) {
                let _ = client.revoke_device(&access, &device).await;
            }
        }
        let _ = forget_refresh(home.as_ref(), &earlier.device_id);
    }
    home.store()
        .set_remote_computer(&row)
        .map_err(store_failed)?;
    *home.computer().session.lock().await = Some(session);
    *lock(home.computer()) = Known::default();
    let _ = refresh(home).await;
    relist(home);
    Ok(row)
}

/// Sign this computer's client device out at the server, and forget it here. Done here even
/// when the server cannot be reached, which the error then says.
pub async fn sign_out(home: &impl ComputerHome) -> Result<()> {
    let Some(row) = home.store().remote_computer().map_err(store_failed)? else {
        return Ok(());
    };
    let told = async {
        let mut slot = home.computer().session.lock().await;
        let session = ensure(home, &mut slot, &row).await?;
        let access = access(home, session).await?;
        let client = Client::new(&row.server_url).map_err(failed)?;
        let device = remote_proto::DeviceId::new(row.device_id.clone())
            .map_err(|e| Failure::new("remote_store", e.to_string()))?;
        client.revoke_device(&access, &device).await.map_err(failed)
    }
    .await;
    *home.computer().session.lock().await = None;
    if let Some(lister) = home
        .computer()
        .lister
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        lister.abort();
    }
    *lock(home.computer()) = Known::default();
    let _ = forget_refresh(home, &row.device_id);
    home.store()
        .forget_remote_computer()
        .map_err(store_failed)?;
    home.changed();
    told
}

/// Ask the host whose code this is to let this computer watch it. The answer is its person's:
/// the grant waits until they give it.
pub async fn pair<H: ComputerHome>(home: &Arc<H>, code: &str) -> Result<GrantSummary> {
    let Some(code) = normalize_code(code) else {
        return Err(Failure::new(
            "remote_code",
            "A pairing code is eight letters and digits, like ABCD-EFGH.",
        ));
    };
    let (client, access) = signed_in(home.as_ref()).await?;
    let request = PairingRequest {
        offer: OfferKey::Code(Secret::new(code)),
        scopes: ASKED.into_iter().collect(),
    };
    let grant = client
        .pair(&access, &request)
        .await
        .map_err(|error| match error.code() {
            Some(ErrorCode::NotFound) => Failure::new(
                "remote_code",
                "No computer is showing that code. Check it, or show a new one there.",
            ),
            _ => refused_here(error),
        })?;
    let _ = refresh(home).await;
    // Waking the lister, which now has a pairing to wait on, and asks more often until answered.
    relist(home);
    Ok(grant)
}

/// Stop reaching a host, and stop listing the pairing: one still in force is taken back at the
/// server first. The server goes on listing grants it has taken back or refused, so the pairing is
/// also kept as forgotten here, and a new pairing with the host is a new grant that shows.
pub async fn forget<H: ComputerHome>(home: &Arc<H>, grant: &str) -> Result<()> {
    let id = GrantId::new(grant.to_owned())
        .map_err(|_| Failure::new("remote_grant", "No such pairing."))?;
    let in_force = lock(home.computer())
        .grants
        .iter()
        .any(|g| g.id == id && matches!(g.state, GrantState::Approved | GrantState::Pending));
    if in_force {
        let (client, access) = signed_in(home.as_ref()).await?;
        client
            .revoke_grant(&access, &id)
            .await
            .map_err(refused_here)?;
    }
    home.store()
        .forget_remote_pairing(grant)
        .map_err(store_failed)?;
    refresh(home).await
}

/// Ask the server which hosts this computer is paired with, and keep a link to each it may
/// reach: one is opened for each grant newly approved, and dropped for each gone.
pub async fn refresh<H: ComputerHome>(home: &Arc<H>) -> Result<()> {
    let listed = async {
        let (client, access) = signed_in(home.as_ref()).await?;
        client.grants(&access).await.map_err(refused_here)
    }
    .await;
    let mut known = lock(home.computer());
    known.listed_at = Some(Instant::now());
    match listed {
        Ok(page) => {
            known.problem = None;
            let forgotten = home.store().forgotten_remote_pairings().unwrap_or_default();
            known.grants = newest_per_host(page.grants);
            known
                .grants
                .retain(|g| !forgotten.iter().any(|id| id == g.id.as_str()));
            let approved: Vec<HostId> = known
                .grants
                .iter()
                .filter(|g| g.state == GrantState::Approved)
                .map(|g| g.host.clone())
                .collect();
            known.links.retain(|host, _| approved.contains(host));
            for host in approved {
                if let std::collections::hash_map::Entry::Vacant(slot) = known.links.entry(host) {
                    let view = Arc::new(Mutex::new(HostView {
                        link: Link::Connecting,
                        shown: None,
                    }));
                    let channel = Open::new(None);
                    let task = tokio::spawn(watch(
                        Arc::clone(home),
                        slot.key().clone(),
                        Arc::clone(&view),
                        channel.clone(),
                    ));
                    slot.insert(HostLink {
                        view,
                        channel,
                        task,
                    });
                }
            }
            drop(known);
            home.changed();
            Ok(())
        }
        Err(problem) => {
            // Signed out at the server: nothing it was paired with can be reached now.
            if problem.is_signed_out() {
                known.grants.clear();
                known.links.clear();
            }
            known.problem = Some(problem.clone());
            drop(known);
            home.changed();
            Err(problem)
        }
    }
}

/// Start at launch, when signed in: list the hosts, and reach those it may. Needs a Tokio runtime.
pub fn start<H: ComputerHome>(home: &Arc<H>) {
    if matches!(home.store().remote_computer(), Ok(Some(_))) {
        relist(home);
    }
}

/// List the hosts again soon, on a task of its own: what a link learns, that its grant was taken
/// back, and what the window sees going stale, both come here.
fn relist<H: ComputerHome>(home: &Arc<H>) {
    let computer = home.computer();
    let mut lister = computer.lister.lock().unwrap_or_else(|e| e.into_inner());
    if lister.as_ref().is_none_or(|task| task.is_finished()) {
        let home = Arc::clone(home);
        *lister = Some(tokio::spawn(async move {
            loop {
                let _ = refresh(&home).await;
                let waiting = lock(home.computer())
                    .grants
                    .iter()
                    .any(|g| g.state == GrantState::Pending);
                let every = if waiting { WAITING_RELIST } else { IDLE_RELIST };
                tokio::select! {
                    () = home.computer().relist.notified() => {}
                    () = tokio::time::sleep(every) => {}
                }
            }
        }));
    } else {
        computer.relist.notify_one();
    }
}

/// The hosts paired with, as last listed, and why they could not be listed again, if so. Asks
/// again, in the background, when the list is older than [`RELIST_EVERY`].
pub fn paired<H: ComputerHome>(home: &Arc<H>) -> (Vec<Paired>, Option<Failure>) {
    let known = lock(home.computer());
    let stale = known
        .listed_at
        .is_none_or(|at| at.elapsed() >= RELIST_EVERY);
    let paired = known
        .grants
        .iter()
        .map(|grant| Paired {
            grant: grant.clone(),
            view: known
                .links
                .get(&grant.host)
                .map(|link| link.view.lock().expect("host view").clone()),
        })
        .collect();
    let problem = known.problem.clone();
    drop(known);
    if stale && matches!(home.store().remote_computer(), Ok(Some(_))) {
        relist(home);
    }
    (paired, problem)
}

/// One host as this computer last saw it, when it may reach it.
pub fn view(home: &impl ComputerHome, host: &str) -> Option<HostView> {
    let known = lock(home.computer());
    known
        .links
        .iter()
        .find(|(id, _)| id.as_str() == host)
        .map(|(_, link)| link.view.lock().expect("host view").clone())
}

/// One grant per host: the one in force, else the newest.
fn newest_per_host(mut grants: Vec<GrantSummary>) -> Vec<GrantSummary> {
    let rank = |g: &GrantSummary| match g.state {
        GrantState::Approved => 0,
        GrantState::Pending => 1,
        _ => 2,
    };
    grants.sort_by(|a, b| {
        a.host
            .as_str()
            .cmp(b.host.as_str())
            .then(rank(a).cmp(&rank(b)))
            .then(b.created_at.cmp(&a.created_at))
    });
    grants.dedup_by(|later, kept| later.host == kept.host);
    grants.sort_by(|a, b| a.host_name.cmp(&b.host_name));
    grants
}

// --- a host's link -------------------------------------------------------------------------------

/// Keep a link to `host` for as long as this task runs.
async fn watch<H: ComputerHome>(
    home: Arc<H>,
    host: HostId,
    view: Arc<Mutex<HostView>>,
    open: Open,
) {
    let mut backoff = FIRST_BACKOFF;
    loop {
        set_link(home.as_ref(), &view, Link::Connecting);
        let began = Instant::now();
        let why = match reach(&home, &host, &view, &open).await {
            Ok(why) | Err(why) => why,
        };
        if let Some(channel) = open.send_replace(None) {
            channel.close();
        }
        set_link(
            home.as_ref(),
            &view,
            Link::Offline {
                since_ms: now_ms(),
                why,
            },
        );
        if began.elapsed() >= STEADY {
            backoff = FIRST_BACKOFF;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn set_link(home: &impl ComputerHome, view: &Mutex<HostView>, link: Link) {
    let mut view = view.lock().expect("host view");
    if view.link != link {
        view.link = link;
        drop(view);
        home.changed();
    }
}

/// One connection to `host`, until it ends: why it ended, in a few words.
async fn reach<H: ComputerHome>(
    home: &Arc<H>,
    host: &HostId,
    view: &Mutex<HostView>,
    open: &Open,
) -> std::result::Result<String, String> {
    let (client, access) = signed_in(home.as_ref()).await.map_err(|f| f.message)?;
    let channel = match Channel::open(&client, &access, host, &home.client_name()).await {
        Ok(channel) => channel,
        Err(error) => {
            // Taken back meanwhile: the list says so, and drops this link.
            if matches!(
                error.code(),
                Some(ErrorCode::Forbidden | ErrorCode::NotFound | ErrorCode::Unauthenticated)
            ) {
                home.computer().relist.notify_one();
            }
            return Err(match error.code() {
                Some(ErrorCode::HostOffline) => "it is not connected".to_owned(),
                _ => error.to_string(),
            });
        }
    };
    let channel = Arc::new(channel);
    open.send_replace(Some(Arc::clone(&channel)));
    set_link(home.as_ref(), view, Link::Online);
    let follows = channel
        .host()
        .capabilities
        .iter()
        .any(|op| op == "activity.subscribe");
    let mut subscription = if follows {
        channel
            .subscribe(ActivitySubscribeParams::default())
            .await
            .ok()
    } else {
        None
    };
    loop {
        match read(&channel).await {
            Ok(shown) => {
                view.lock().expect("host view").shown = Some(shown);
                home.changed();
            }
            Err(_) if !channel.is_open() => {}
            Err(error) => return Err(error.to_string()),
        }
        // Until its activity moves, or a while has passed.
        let moved = async {
            match subscription.as_mut() {
                Some(subscription) => match subscription.next().await {
                    Some(Followed::Events(_)) => true,
                    _ => std::future::pending().await,
                },
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            end = channel.ended() => return Ok(end.text),
            _ = moved => {}
            () = home.computer().reread.notified() => {}
            () = tokio::time::sleep(REREAD_EVERY) => {}
        }
    }
}

/// What the host shows, read through `channel`.
async fn read(channel: &Channel) -> remote_client::channel::Result<Shown> {
    let page = channel
        .workspaces(WorkspaceListParams {
            limit: Some(MAX_PAGE),
            ..WorkspaceListParams::default()
        })
        .await?;
    let mut workspaces = Vec::new();
    let mut activity = Vec::new();
    for workspace in page.workspaces.into_iter().take(WORKSPACES_READ) {
        let sessions = channel
            .sessions(SessionListParams {
                workspace: workspace.id.clone(),
                include_ended: true,
                cursor: None,
                limit: Some(10),
            })
            .await?;
        if let Ok(recent) = channel
            .activity(ActivityListParams {
                workspace: workspace.id.clone(),
                session: None,
                cursor: None,
                limit: Some(20),
            })
            .await
        {
            activity.extend(recent.events);
        }
        workspaces.push((workspace, sessions));
    }
    activity.sort_by_key(|event| std::cmp::Reverse(event.occurred_at));
    activity.truncate(ACTIVITY_SHOWN);
    Ok(Shown {
        host: channel.host().clone(),
        projects: page.projects,
        workspaces,
        activity,
        read_at_ms: now_ms(),
    })
}

// --- the session ---------------------------------------------------------------------------------

fn lock(computer: &Computer) -> std::sync::MutexGuard<'_, Known> {
    computer.known.lock().unwrap_or_else(|e| e.into_inner())
}

/// The server's client and an access credential, when signed in.
async fn signed_in(home: &impl ComputerHome) -> Result<(Client, Secret)> {
    let row = home
        .store()
        .remote_computer()
        .map_err(store_failed)?
        .ok_or_else(|| {
            Failure::new(
                "remote_not_signed_in",
                "This computer is not signed in to reach other computers.",
            )
        })?;
    let mut slot = home.computer().session.lock().await;
    let session = ensure(home, &mut slot, &row).await?;
    let access = access(home, session).await?;
    Ok((Client::new(&row.server_url).map_err(failed)?, access))
}

/// The session, resumed from the kept refresh credential after a restart.
async fn ensure<'a>(
    home: &impl ComputerHome,
    slot: &'a mut Option<Session>,
    row: &RemoteComputerRow,
) -> Result<&'a mut Session> {
    if slot.is_none() {
        let refresh = home
            .credentials()?
            .get(&refresh_entry(&row.device_id))
            .map_err(|e| Failure::new("remote_credentials", e))?
            .ok_or_else(|| Failure::signed_out("This computer's sign-in is missing."))?;
        let client = Client::new(&row.server_url).map_err(failed)?;
        let session = Session::resume(client, &refresh)
            .await
            .map_err(refused_here)?;
        let session = slot.insert(session);
        // The kept one is spent now. Should this fail, the session carries on with the new one
        // until the process ends, and the error says why the next start will be signed out.
        keep_refresh(home, &row.device_id, session.refresh_credential())
            .map_err(|e| Failure::new("remote_credentials", e))?;
        return Ok(session);
    }
    Ok(slot.as_mut().expect("just filled"))
}

/// An access credential, keeping any new refresh credential under the session's device.
async fn access(home: &impl ComputerHome, session: &mut Session) -> Result<Secret> {
    let device = session.device.clone();
    session
        .access(|refresh| keep_refresh(home, device.as_str(), refresh))
        .await
        .map_err(refused_here)
}

fn keep_refresh(
    home: &impl ComputerHome,
    device: &str,
    refresh: &Secret,
) -> std::result::Result<(), String> {
    home.credentials()
        .map_err(|e| e.message)?
        .set(&refresh_entry(device), refresh)
}

fn forget_refresh(home: &impl ComputerHome, device: &str) -> std::result::Result<(), String> {
    home.credentials()
        .map_err(|e| e.message)?
        .delete(&refresh_entry(device))
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new("remote_store", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(host: &str, state: GrantState, created_at: i64) -> GrantSummary {
        GrantSummary {
            id: GrantId::new(format!("g-{host}-{created_at}")).unwrap(),
            host: HostId::new(host).unwrap(),
            host_name: host.to_uppercase(),
            device: remote_proto::DeviceId::new("d").unwrap(),
            device_name: "This computer".into(),
            device_kind: None,
            state,
            requested: ASKED.into_iter().collect(),
            scopes: Default::default(),
            projects: None,
            created_at,
            decided_at: None,
            expires_in_ms: None,
        }
    }

    /// Asked twice, once refused and once let in: the host shows once, as let in.
    #[test]
    fn each_host_shows_once_by_the_grant_in_force_else_the_newest() {
        let shown = newest_per_host(vec![
            grant("b", GrantState::Denied, 3),
            grant("a", GrantState::Denied, 5),
            grant("a", GrantState::Approved, 1),
            grant("b", GrantState::Pending, 2),
            grant("c", GrantState::Revoked, 1),
            grant("c", GrantState::Denied, 4),
        ]);
        let states: Vec<_> = shown
            .iter()
            .map(|g| (g.host.as_str().to_owned(), g.state, g.created_at))
            .collect();
        assert_eq!(
            states,
            [
                ("a".to_owned(), GrantState::Approved, 1),
                ("b".to_owned(), GrantState::Pending, 2),
                ("c".to_owned(), GrantState::Denied, 4),
            ]
        );
    }
}

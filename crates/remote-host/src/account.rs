//! The host's account at its server: signing in, the session and its rotating refresh credential,
//! registering as a host, pairing, and the devices let in. The desktop app and a headless host
//! (`yardsort-host`) both run this; each says where its records, settings and credentials are
//! through [`Home`], and how a device asking is told to a person through [`Home::asked`].
//!
//! **One process holds the session.** A refresh credential works once: presenting a spent one
//! signs the host out (docs/design/27-remote-r2.md §3). So whatever approves, revokes or pairs has
//! to go through the process that holds [`Remote`], never hold a copy of the credential.
//!
//! **Nothing here decides by itself.** [`answer`] carries a person's answer to the server; what
//! calls it is the window, or the operator at a headless host's own terminal. Nothing an agent is
//! given reaches it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use remote_client::{Client, ClientError, Session};
use remote_proto::account::{Credentials, Secret, ServerDescription};
use remote_proto::pairing::{
    GrantDecision, GrantState, GrantSummary, HostRegistration, OFFER_TTL_MS,
};
use remote_proto::relay::HostNotice;
use remote_proto::{DeviceId, ErrorCode, GrantId, HostId, ProjectGrant, ProjectId, Scope};
use yardsort_core::remote::host::{self, refresh_entry, CredentialStore};
use yardsort_core::remote::serve::Context;
use yardsort_core::settings::{LeftHost, RemoteSettings, SettingsFile};
use yardsort_core::store::{RemoteHostRow, Store};

use crate::{Access, AccessError, AccessFuture, Connector, Serving, Status};

/// What viewing gives: everything a device can read, nothing it can type.
pub const VIEW: [Scope; 4] = [
    Scope::WorkspaceRead,
    Scope::ActivityRead,
    Scope::MemoryRead,
    Scope::TerminalRead,
];

/// Where a host keeps what it is: its records, its settings and its credential, and what a
/// device's channel answers from.
pub trait Home: Send + Sync + 'static {
    /// The session and the link, kept for as long as the process runs.
    fn remote(&self) -> &Remote;
    fn store(&self) -> &Store;
    fn settings(&self) -> &SettingsFile;
    /// Where the host's refresh credential is kept.
    fn credentials(&self) -> Result<Box<dyn CredentialStore>>;
    /// Run `f` with what a channel answers from. Blocking: called on a channel's own thread.
    fn serve(&self, f: &mut dyn FnMut(&Context<'_>));
    /// A device asks to be let in, and the server has it waiting: tell the person here. Never
    /// answers it.
    fn asked(&self, request: Request);
}

/// The host's session with its server, and the link to the server's relay.
#[derive(Default)]
pub struct Remote {
    session: tokio::sync::Mutex<Option<Session>>,
    connector: Mutex<Option<Connector>>,
}

/// Why something failed: a stable code, and words for a person. Signed out at the server is
/// `remote_signed_out`, whose words say why but not what to do, which is each program's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn signed_out(why: impl Into<String>) -> Self {
        Self::new(SIGNED_OUT, why)
    }

    pub fn is_signed_out(&self) -> bool {
        self.code == SIGNED_OUT
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub const SIGNED_OUT: &str = "remote_signed_out";

pub type Result<T> = std::result::Result<T, Failure>;

/// A device that redeemed this host's code and waits for an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub grant_id: String,
    pub device_name: String,
    /// It asked to type into terminals, not only watch.
    pub wants_control: bool,
    pub expires_in_ms: u32,
}

/// The host's person's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Deny,
    Allow {
        /// Typing into terminals too, if the device asked for it.
        control: bool,
        /// Only these projects; all of them when absent.
        projects: Option<Vec<String>>,
    },
}

/// A pairing offer: the code to type, and the link a QR code holds, good once until it lapses.
#[derive(Debug, Clone)]
pub struct Offer {
    pub code: String,
    /// `yardsort://pair?server=…&secret=…`.
    pub link: String,
    pub expires_in_ms: u32,
}

pub fn failed(error: ClientError) -> Failure {
    let code = match error.code() {
        Some(ErrorCode::Unauthenticated) => SIGNED_OUT,
        Some(ErrorCode::RateLimited) => "remote_rate_limited",
        Some(_) => "remote_refused",
        None => match error {
            ClientError::Invalid(_) => "remote_address",
            ClientError::TimedOut => "remote_timed_out",
            ClientError::NotKept(_) => "remote_credentials",
            _ => "remote_unreachable",
        },
    };
    Failure::new(code, error.to_string())
}

/// A request made with this computer's own credential. Refused, it was signed out at the server;
/// the server's own words ("Sign in again.") are for any client.
pub fn refused_here(error: ClientError) -> Failure {
    if error.code() == Some(ErrorCode::Unauthenticated) {
        Failure::signed_out("This computer was signed out at the server.")
    } else {
        failed(error)
    }
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new("remote_store", error.to_string())
}

fn keep_refresh(
    home: &impl Home,
    device: &str,
    refresh: &Secret,
) -> std::result::Result<(), String> {
    home.credentials()
        .map_err(|e| e.message)?
        .set(&refresh_entry(device), refresh)
}

fn forget_refresh(home: &impl Home, device: &str) -> std::result::Result<(), String> {
    home.credentials()
        .map_err(|e| e.message)?
        .delete(&refresh_entry(device))
}

/// An access credential, keeping any new refresh credential under the session's device.
async fn access(home: &impl Home, session: &mut Session) -> Result<Secret> {
    let device = session.device.clone();
    session
        .access(|refresh| keep_refresh(home, device.as_str(), refresh))
        .await
        .map_err(refused_here)
}

/// Keep a link to the relay, when Remote is on: at start, and once it is turned on. Needs a
/// Tokio runtime: call it from async code.
pub fn connect<H: Home>(home: &Arc<H>) {
    let Ok(Some(row)) = home.store().remote_host() else {
        return;
    };
    let Ok(host) = HostId::new(row.host_id) else {
        return;
    };
    let connector = Connector::start(
        host,
        Arc::new(HomeAccess(home.clone())),
        Arc::new(HomeServing(home.clone())),
    );
    // An earlier one stops as it is dropped.
    let earlier = match home.remote().connector.lock() {
        Ok(mut slot) => slot.replace(connector),
        Err(_) => None,
    };
    drop(earlier);
}

/// Close the link, and with it every device's channel.
pub async fn disconnect(home: &impl Home) {
    let connector = home
        .remote()
        .connector
        .lock()
        .ok()
        .and_then(|mut c| c.take());
    if let Some(connector) = connector {
        connector.stop().await;
    }
}

/// Where the link stands; `None` when it is not kept.
pub fn link(home: &impl Home) -> Option<Status> {
    home.remote()
        .connector
        .lock()
        .ok()
        .and_then(|c| c.as_ref().map(Connector::status))
}

/// Close at once every channel whose grant is gone here.
pub fn recheck(home: &impl Home) {
    if let Some(connector) = home
        .remote()
        .connector
        .lock()
        .ok()
        .as_deref()
        .and_then(Option::as_ref)
    {
        connector.recheck();
    }
}

/// The host's access credential, from its session with the server.
struct HomeAccess<H>(Arc<H>);

impl<H: Home> Access for HomeAccess<H> {
    fn access(&self) -> AccessFuture<'_> {
        Box::pin(async move {
            let home = self.0.as_ref();
            let row = home
                .store()
                .remote_host()
                .map_err(|e| AccessError::Unavailable(e.to_string()))?
                .ok_or_else(|| AccessError::SignedOut("Remote is off.".into()))?;
            let mut session = home.remote().session.lock().await;
            let session = ensure(home, &mut session, &row)
                .await
                .map_err(access_error)?;
            let secret = access(home, session).await.map_err(access_error)?;
            Ok((session.client.clone(), secret))
        })
    }
}

fn access_error(failure: Failure) -> AccessError {
    if failure.is_signed_out() {
        AccessError::SignedOut(failure.message)
    } else {
        AccessError::Unavailable(failure.message)
    }
}

struct HomeServing<H>(Arc<H>);

impl<H: Home> Serving for HomeServing<H> {
    fn with(&self, f: &mut dyn FnMut(&Context<'_>)) {
        self.0.serve(f);
    }

    fn told(&self, notice: HostNotice) {
        let HostNotice::PairingRequested { grant } = notice else {
            return;
        };
        let home = self.0.clone();
        // On the link's own task, which must not wait on the server.
        tokio::spawn(async move {
            if let Err(error) = asked(home.as_ref(), &grant).await {
                eprintln!("remote: a pairing request: {}", error.message);
            }
        });
    }
}

/// The relay said `grant` asks: tell the person here, if the server has it waiting. The notice is
/// only a hint; what is told is what the server answers this host.
async fn asked(home: &impl Home, grant: &GrantId) -> Result<()> {
    let row = enabled(home)?;
    let waiting = host_grants(home, &row)
        .await?
        .into_iter()
        .find(|g| g.id == *grant && g.state == GrantState::Pending);
    if let Some(waiting) = waiting {
        home.asked(request(waiting));
    }
    Ok(())
}

fn request(grant: GrantSummary) -> Request {
    Request {
        wants_control: grant.requested.contains(&Scope::TerminalControl),
        grant_id: grant.id.to_string(),
        device_name: grant.device_name,
        expires_in_ms: grant
            .expires_in_ms
            .unwrap_or_default()
            .min(OFFER_TTL_MS as u64) as u32,
    }
}

/// Before signing in as `name` at `server_url`: the name checked, so one the server would refuse
/// leaves no device behind, and the server asked who it is.
pub async fn prepare(server_url: &str, name: &str) -> Result<(Client, ServerDescription)> {
    HostRegistration {
        host: None,
        name: name.to_owned(),
    }
    .validate()
    .map_err(|e| Failure::new("remote_name", e.message))?;
    let client = Client::new(server_url).map_err(failed)?;
    let server = client.describe().await.map_err(failed)?;
    Ok((client, server))
}

/// Signed in as `signed_in` at `server`: register this computer as a host there, as the host it
/// already was if it was one, and keep it. Then keep a link to the relay.
pub async fn enable<H: Home>(
    home: &Arc<H>,
    client: Client,
    server: &ServerDescription,
    name: &str,
    signed_in: Credentials,
) -> Result<RemoteHostRow> {
    // Kept before anything else: from here on, it is the only way back in.
    keep_refresh(home.as_ref(), signed_in.device.as_str(), &signed_in.refresh)
        .map_err(|e| Failure::new("remote_credentials", e))?;
    let mut session = Session::new(client.clone(), signed_in);
    let access = access(home.as_ref(), &mut session).await?;

    let earlier = home.store().remote_host().map_err(store_failed)?;
    let left = home.settings().get().remote.left_host;
    let was = host_before(earlier.as_ref(), left.as_ref(), server.server.as_str());
    let again = was.as_ref().map(|w| w.host.clone());
    let registration = |host| HostRegistration {
        host,
        name: name.to_owned(),
    };
    let registered = match client
        .register_host(&access, &registration(again.clone()))
        .await
    {
        // The host was removed meanwhile, from here or elsewhere: start over as a new one.
        Err(e) if again.is_some() && e.code() == Some(ErrorCode::NotFound) => {
            client.register_host(&access, &registration(None)).await
        }
        other => other,
    };
    let registered = match registered {
        Ok(registered) => registered,
        Err(error) => {
            // Signed in, but not a host: sign that device out again rather than leave it.
            let _ = client.revoke_device(&access, &session.device).await;
            let _ = forget_refresh(home.as_ref(), session.device.as_str());
            return Err(failed(error));
        }
    };
    if let Some(was) = &was {
        // The device this computer was signed in as before is signed out: it has no use now.
        if was.device != session.device {
            let _ = client.revoke_device(&access, &was.device).await;
        }
        // Turned off, it let its devices go, though the server could not be told then.
        if was.turned_off && was.host == registered.id {
            if let Ok(page) = client.host_grants(&access, &was.host).await {
                for grant in page.grants {
                    let _ = client.revoke_grant(&access, &grant.id).await;
                }
            }
        }
    }
    let row = RemoteHostRow {
        server_url: client.address().as_str().trim_end_matches('/').to_owned(),
        server_id: server.server.to_string(),
        host_id: registered.id.to_string(),
        device_id: session.device.to_string(),
        name: registered.name,
        enabled_at: now_ms(),
    };
    home.store().set_remote_host(&row).map_err(store_failed)?;
    if let Some(earlier) = earlier.filter(|h| h.device_id != row.device_id) {
        let _ = forget_refresh(home.as_ref(), &earlier.device_id);
    }
    // Only a convenience for next time: Remote is on whether or not it could be written.
    let _ = home.settings().update(|s| {
        s.remote = RemoteSettings {
            server_url: Some(row.server_url.clone()),
            name: Some(row.name.clone()),
            left_host: None,
        }
    });
    *home.remote().session.lock().await = Some(session);
    connect(home);
    Ok(row)
}

/// Turn Remote off: remove this host at the server and sign its device out there, then forget
/// both here. Done here even when the server cannot be reached, which the error then says.
pub async fn disable(home: &impl Home) -> Result<()> {
    let Some(row) = home.store().remote_host().map_err(store_failed)? else {
        return Ok(());
    };
    // Every device is let go of first: nothing comes through while Remote goes off.
    disconnect(home).await;
    let at_server = async {
        let mut session = home.remote().session.lock().await;
        let session = ensure(home, &mut session, &row).await?;
        let access = access(home, session).await?;
        let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
        session
            .client
            .remove_host(&access, &host)
            .await
            .map_err(refused_here)?;
        session
            .client
            .revoke_device(&access, &session.device)
            .await
            .map_err(refused_here)
    }
    .await;
    *home.remote().session.lock().await = None;
    home.store().forget_remote_host().map_err(store_failed)?;
    forget_refresh(home, &row.device_id).map_err(|e| Failure::new("remote_credentials", e))?;
    // Not removed there, so remembered here: turned on again, it picks up where this leaves it.
    let left = at_server.is_err().then(|| LeftHost {
        server_id: row.server_id.clone(),
        host_id: row.host_id.clone(),
        device_id: row.device_id.clone(),
    });
    let _ = home.settings().update(|s| s.remote.left_host = left);
    if let Err(error) = at_server {
        // Signed out at the server, there is nothing to tell anyone: it is off, and that is all.
        if !error.is_signed_out() {
            return Err(Failure::new(
                "remote_off_here_only",
                format!(
                    "Remote is off on this computer, but the server could not be told: {}. \
                     Sign this computer out from another device if it still shows there.",
                    error.message
                ),
            ));
        }
    }
    Ok(())
}

/// Make a pairing offer: a code to type, and a link for a QR code, good once for five minutes.
pub async fn pair(home: &impl Home) -> Result<Offer> {
    let row = enabled(home)?;
    let mut session = home.remote().session.lock().await;
    let session = ensure(home, &mut session, &row).await?;
    let access = access(home, session).await?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    let offer = session
        .client
        .offer(&access, &host)
        .await
        .map_err(refused_here)?;
    let mut link = url::Url::parse("yardsort://pair").map_err(store_failed)?;
    link.query_pairs_mut()
        .append_pair("server", &row.server_url)
        .append_pair("secret", offer.secret.expose());
    Ok(Offer {
        code: offer.code.expose().to_owned(),
        link: link.into(),
        expires_in_ms: offer.expires_in_ms.min(OFFER_TTL_MS as u64) as u32,
    })
}

/// Devices waiting for an answer, once the devices let in here have been brought in line with the
/// server's word on which are still in force. The server can take a device off this host's list,
/// never put one on it.
pub async fn requests(home: &impl Home) -> Result<Vec<Request>> {
    let row = enabled(home)?;
    let grants = host_grants(home, &row).await?;
    let in_force: Vec<String> = grants
        .iter()
        .filter(|g| g.state == GrantState::Approved)
        .map(|g| g.id.to_string())
        .collect();
    home.store()
        .retain_remote_grants(&in_force)
        .map_err(store_failed)?;
    Ok(grants
        .into_iter()
        .filter(|g| g.state == GrantState::Pending)
        .map(request)
        .collect())
}

/// Answer a device that asked to be let in. Allowing gives it what it asked for that viewing
/// (and, with `control`, typing) covers, on the projects chosen; never more than it asked for.
pub async fn answer(home: &impl Home, grant_id: String, answer: Answer) -> Result<()> {
    let row = enabled(home)?;
    let id =
        GrantId::new(grant_id).map_err(|_| Failure::new("remote_refused", "No such request."))?;
    let asked = host_grants(home, &row)
        .await?
        .into_iter()
        .find(|g| g.id == id && g.state == GrantState::Pending)
        .ok_or_else(|| {
            Failure::new(
                "remote_refused",
                "That device is no longer waiting. Show a new code to pair it.",
            )
        })?;
    let decision = match answer {
        Answer::Deny => None,
        Answer::Allow { control, projects } => {
            let mut offered: BTreeSet<Scope> = VIEW.into();
            if control {
                offered.insert(Scope::TerminalControl);
            }
            let scopes: BTreeSet<Scope> = asked.requested.intersection(&offered).copied().collect();
            if scopes.is_empty() {
                return Err(Failure::new(
                    "remote_refused",
                    "That device asked for nothing this computer gives.",
                ));
            }
            let projects = match projects {
                None => ProjectGrant::All,
                Some(ids) => ProjectGrant::Only(
                    ids.into_iter()
                        .map(ProjectId::new)
                        .collect::<std::result::Result<_, _>>()
                        .map_err(|_| Failure::new("remote_refused", "Not a project."))?,
                ),
            };
            Some(GrantDecision { scopes, projects })
        }
    };
    let decided = {
        let mut session = home.remote().session.lock().await;
        let session = ensure(home, &mut session, &row).await?;
        let access = access(home, session).await?;
        session
            .client
            .decide(&access, &id, decision.as_ref())
            .await
            .map_err(refused_here)?
    };
    if let (Some(decision), GrantState::Approved) = (&decision, decided.state) {
        // What this host's person chose, as they chose it: the server's copy is not trusted for
        // what it says was given.
        host::keep_approved(
            home.store(),
            &decided.id,
            &decided.device,
            &decided.device_name,
            &decision.scopes,
            &decision.projects,
            now_ms(),
        )
        .map_err(store_failed)?;
    }
    Ok(())
}

/// Take a device's grant away. Refused here at once; the server is told too.
pub async fn revoke(home: &impl Home, grant_id: String) -> Result<()> {
    let row = enabled(home)?;
    home.store()
        .drop_remote_grant(&grant_id)
        .map_err(store_failed)?;
    // Its channel closes now, not at the next check.
    recheck(home);
    let id =
        GrantId::new(grant_id).map_err(|_| Failure::new("remote_refused", "No such device."))?;
    let mut session = home.remote().session.lock().await;
    let session = ensure(home, &mut session, &row).await?;
    let access = access(home, session).await?;
    session
        .client
        .revoke_grant(&access, &id)
        .await
        .map_err(refused_here)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// This computer as a host, or why Remote is off.
pub fn enabled(home: &impl Home) -> Result<RemoteHostRow> {
    home.store()
        .remote_host()
        .map_err(store_failed)?
        .ok_or_else(|| Failure::new("remote_off", "Remote is off on this computer."))
}

async fn host_grants(home: &impl Home, row: &RemoteHostRow) -> Result<Vec<GrantSummary>> {
    let mut session = home.remote().session.lock().await;
    let session = ensure(home, &mut session, row).await?;
    let access = access(home, session).await?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    Ok(session
        .client
        .host_grants(&access, &host)
        .await
        .map_err(refused_here)?
        .grants)
}

/// The host this computer already is at a server, about to be signed in to.
#[derive(Debug, PartialEq, Eq)]
struct HostBefore {
    host: HostId,
    device: DeviceId,
    /// Turned off since, so its devices were let go here, whatever the server still has.
    turned_off: bool,
}

fn host_before(
    earlier: Option<&RemoteHostRow>,
    left: Option<&LeftHost>,
    server: &str,
) -> Option<HostBefore> {
    let was = |host: &str, device: &str, turned_off| {
        Some(HostBefore {
            host: HostId::new(host.to_owned()).ok()?,
            device: DeviceId::new(device.to_owned()).ok()?,
            turned_off,
        })
    };
    match (earlier, left) {
        (Some(on), _) if on.server_id == server => was(&on.host_id, &on.device_id, false),
        (None, Some(left)) if left.server_id == server => was(&left.host_id, &left.device_id, true),
        _ => None,
    }
}

/// The session, resumed from the kept refresh credential after a restart.
async fn ensure<'a>(
    home: &impl Home,
    slot: &'a mut Option<Session>,
    row: &RemoteHostRow,
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

#[cfg(test)]
mod tests {
    use remote_proto::RemoteError;

    use super::*;

    const HOST: &str = "7f3c1a2b-0d4e-4f5a-8b6c-1d2e3f4a5b6c";
    const DEVICE: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

    fn on_at(server: &str) -> RemoteHostRow {
        RemoteHostRow {
            server_url: "https://remote.example.com".into(),
            server_id: server.into(),
            host_id: HOST.into(),
            device_id: DEVICE.into(),
            name: "Workstation".into(),
            enabled_at: 1,
        }
    }

    fn left_at(server: &str) -> LeftHost {
        LeftHost {
            server_id: server.into(),
            host_id: HOST.into(),
            device_id: DEVICE.into(),
        }
    }

    /// Signed in again at the same server, a computer is the host it was: a second one beside it
    /// would mean pairing again, and an entry on the phone that never comes back.
    #[test]
    fn signing_in_again_is_the_same_host_and_its_devices_stay_only_if_it_stayed_on() {
        let host = HostId::new(HOST.to_owned()).unwrap();
        let device = DeviceId::new(DEVICE.to_owned()).unwrap();
        let before = |turned_off| {
            Some(HostBefore {
                host: host.clone(),
                device: device.clone(),
                turned_off,
            })
        };
        // Still on, and signed out: it keeps its devices.
        assert_eq!(host_before(Some(&on_at("srv")), None, "srv"), before(false));
        // Turned off with the server not told: the same host, with its devices let go.
        assert_eq!(
            host_before(None, Some(&left_at("srv")), "srv"),
            before(true)
        );
        // Still on says more than what was left behind.
        assert_eq!(
            host_before(Some(&on_at("srv")), Some(&left_at("srv")), "srv"),
            before(false)
        );
        // Another server has never heard of it.
        assert_eq!(host_before(Some(&on_at("srv")), None, "other"), None);
        assert_eq!(host_before(None, Some(&left_at("srv")), "other"), None);
        assert_eq!(host_before(None, None, "srv"), None);
    }

    #[test]
    fn a_refusal_of_this_computers_credential_says_it_was_signed_out() {
        let refused = |code| ClientError::Remote(RemoteError::new(code, "Sign in again."));
        let failure = refused_here(refused(ErrorCode::Unauthenticated));
        assert!(failure.is_signed_out());
        assert_eq!(
            failure.message,
            "This computer was signed out at the server."
        );
        // The link's own line is the reason, with nothing added.
        assert!(matches!(
            access_error(failure),
            AccessError::SignedOut(why) if why == "This computer was signed out at the server."
        ));
        assert_eq!(
            refused_here(refused(ErrorCode::NotFound)).code,
            "remote_refused"
        );
    }
}

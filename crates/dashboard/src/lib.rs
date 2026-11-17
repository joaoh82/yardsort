//! The host dashboard: a web page a computer serves to itself, for looking after it as a host.
//!
//! It shows how the host and the computer stand, installs agents, sets Git up, clones
//! repositories and runs the commands that set a server up. It is not another Yardsort: there
//! are no workspaces, terminals or agents' conversations in it.
//!
//! `yardsort-host serve` serves it unless told not to, and the desktop app serves it when
//! turned on in Settings. Either gives what it knows of itself through [`Source`].
//!
//! **Who can reach it.** It listens on 127.0.0.1 alone, at a port few things use, and answers
//! only requests addressed to a loopback name or to its own tunnel. A browser must sign in with
//! a one-time code, which is handed out only where a person is ([`Dashboard::sign_in`]): a
//! terminal of their own, or the app's window. Beyond this computer it is reached by an SSH
//! forward, or by a rustunnel tunnel ([`tunnel`]) when one is turned on.
//!
//! **What it adds for whoever is signed in**: nothing that this computer's user could not do at
//! a shell of their own. It lets no device in, approves no note and turns no sync on, so the
//! guard on those (`crates/host/src/guard.rs`) holds whatever happens here.
//!
//! See `docs/design/47-host-dashboard.md` and `docs/guide/host-dashboard.md`.

pub mod agents;
mod auth;
mod git;
mod http;
pub mod jobs;
mod system;
pub mod tunnel;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use yardsort_core::env::ShellEnv;
use yardsort_core::settings::{SettingsFile, TunnelSettings};

pub use auth::{CODE_GOOD_FOR, SESSION_GOOD_FOR};

/// How many ports are tried, from the configured one up, when it is taken.
pub const PORTS_TRIED: u16 = 10;

/// What the program serving the dashboard knows of itself. Every method may block: they are
/// called off the async runtime.
pub trait Source: Send + Sync + 'static {
    /// How the host stands.
    fn about(&self) -> About;
    /// The environment programs are started in: the user's login shell's.
    fn env(&self) -> Arc<ShellEnv>;
    fn settings(&self) -> &SettingsFile;
    /// The projects here.
    fn projects(&self) -> Vec<Project>;
    /// Clone a repository into the folder for clones, and add it as a project.
    fn clone_project(&self, remote: &str) -> Result<Project, String>;
}

/// How the host stands, as `yardsort-host status` says it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct About {
    /// `yardsort-host`, or `Yardsort` for the app.
    pub program: String,
    pub version: String,
    pub pid: u32,
    /// Remote, when this computer is signed in to a server.
    pub remote: Option<RemoteLink>,
    /// Terminals running.
    pub terminals: usize,
    /// What the operator should know, in sentences: an earlier daemon kept, say.
    pub notices: Vec<String>,
    /// What to do about Remote being off, in a sentence.
    pub remote_off_hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteLink {
    pub server_url: String,
    /// What phones call this computer.
    pub name: String,
    /// The link to the relay, in words.
    pub link: String,
    /// Devices let in.
    pub devices: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

pub struct Options {
    /// Where the dashboard keeps who is signed in: a folder of its own in the profile.
    pub state_dir: PathBuf,
}

/// A way in, for one browser, once.
#[derive(Debug, Clone)]
pub struct SignIn {
    /// As typed: `XXXX-XXXX-XXXX-XXXX`.
    pub code: String,
    /// The address on this computer, with the code in it.
    pub local: String,
    /// The address through the tunnel, with the code in it, while the tunnel is up.
    pub public: Option<String>,
    pub good_for: Duration,
}

/// How the dashboard itself stands.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
    /// `http://127.0.0.1:<port>`.
    pub address: String,
    pub port: u16,
    pub tunnel: tunnel::State,
    /// What the tunnel is set to.
    pub tunnel_wanted: TunnelWanted,
    /// Browsers signed in.
    pub signed_in: usize,
}

/// The tunnel's settings, as the page and the command line read them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelWanted {
    pub enabled: bool,
    pub subdomain: Option<String>,
    pub region: Option<String>,
    pub server: Option<String>,
}

impl From<&TunnelSettings> for TunnelWanted {
    fn from(settings: &TunnelSettings) -> Self {
        Self {
            enabled: settings.enabled,
            subdomain: settings.subdomain.clone(),
            region: settings.region.clone(),
            server: settings.server.clone(),
        }
    }
}

pub(crate) struct Inner {
    pub source: Arc<dyn Source>,
    pub auth: auth::Auth,
    pub jobs: jobs::Jobs,
    pub tunnel: tunnel::Tunnel,
    pub machine: system::Reader,
    pub port: u16,
    /// Each agent's version: asking seven programs for theirs takes a while.
    pub versions: Mutex<Vec<KnownVersion>>,
}

/// What a program said its version was, when it was last asked.
pub(crate) struct KnownVersion {
    pub program: PathBuf,
    /// When the program's file was last changed: changed since, it is asked again.
    pub changed: Option<std::time::SystemTime>,
    pub version: Option<String>,
}

impl Inner {
    pub fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn standing(&self) -> Standing {
        Standing {
            address: self.address(),
            port: self.port,
            tunnel: self.tunnel.state(),
            tunnel_wanted: (&self.source.settings().get().dashboard.tunnel).into(),
            signed_in: self.auth.signed_in(),
        }
    }

    /// Keep `wanted` in the settings, and make the tunnel so. Blocking.
    pub fn set_tunnel(&self, wanted: TunnelSettings) -> Result<(), String> {
        let wanted = TunnelSettings {
            subdomain: wanted
                .subdomain
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty()),
            region: wanted
                .region
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            server: wanted
                .server
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            enabled: wanted.enabled,
        };
        tunnel::validate(&wanted)?;
        self.source
            .settings()
            .update(|settings| settings.dashboard.tunnel = wanted.clone())
            .map_err(|error| format!("The settings could not be saved: {error}"))?;
        self.tunnel.apply(&wanted, self.port, self.source.env());
        Ok(())
    }
}

/// The dashboard, served until it is stopped or dropped.
pub struct Dashboard {
    inner: Arc<Inner>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    served: Option<tokio::task::JoinHandle<()>>,
}

impl Dashboard {
    /// Listen on 127.0.0.1, at the settings' port or the first free one of the ten from it, and
    /// start the tunnel if the settings want one. Needs a Tokio runtime to be running.
    pub async fn start(source: Arc<dyn Source>, options: Options) -> Result<Self, String> {
        let settings = source.settings().get().dashboard;
        let first = settings.port();
        let mut listener = None;
        let mut why = String::new();
        for port in (first..=first.saturating_add(PORTS_TRIED - 1)).take(usize::from(PORTS_TRIED)) {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(bound) => {
                    listener = Some((bound, port));
                    break;
                }
                Err(error) => why = error.to_string(),
            }
        }
        let Some((listener, port)) = listener else {
            return Err(format!(
                "None of the ports {first} to {} on 127.0.0.1 could be listened on ({why}). Set \
                 another under [dashboard] in settings.toml, as port.",
                first.saturating_add(PORTS_TRIED - 1)
            ));
        };
        if let Err(error) = auth::make_private_dir(&options.state_dir) {
            return Err(format!(
                "Cannot use {}: {error}",
                options.state_dir.display()
            ));
        }
        let inner = Arc::new(Inner {
            source,
            auth: auth::Auth::open(&options.state_dir),
            jobs: jobs::Jobs::default(),
            tunnel: tunnel::Tunnel::default(),
            machine: system::Reader::default(),
            port,
            versions: Mutex::new(Vec::new()),
        });
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let router = http::router(inner.clone());
        let served = tokio::spawn(async move {
            let serving = axum::serve(listener, router).with_graceful_shutdown(async move {
                let _ = stopped.await;
            });
            if let Err(error) = serving.await {
                eprintln!("yardsort: the dashboard stopped: {error}");
            }
        });
        if settings.tunnel.enabled {
            // Off this thread: the environment is the login shell's, and resolving it runs one.
            let inner = inner.clone();
            tokio::task::spawn_blocking(move || {
                inner
                    .tunnel
                    .apply(&settings.tunnel, inner.port, inner.source.env());
            });
        }
        Ok(Self {
            inner,
            stop: Some(stop),
            served: Some(served),
        })
    }

    /// `http://127.0.0.1:<port>`.
    pub fn address(&self) -> String {
        self.inner.address()
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn standing(&self) -> Standing {
        self.inner.standing()
    }

    /// A one-time way in, for whoever asked: only ever call it for a person. The code lapses in
    /// [`CODE_GOOD_FOR`].
    pub fn sign_in(&self) -> SignIn {
        let code = self.inner.auth.offer();
        let public = match self.inner.tunnel.state() {
            tunnel::State::Ready { url } => {
                Some(format!("{}/#code={code}", url.trim_end_matches('/')))
            }
            _ => None,
        };
        SignIn {
            local: format!("{}/#code={code}", self.inner.address()),
            public,
            code,
            good_for: CODE_GOOD_FOR,
        }
    }

    /// Sign every browser out, and void every code. Says how many browsers that was.
    pub fn sign_everyone_out(&self) -> usize {
        self.inner.auth.end_all()
    }

    /// Keep `wanted` in the settings, and make the tunnel so. Blocking: call it off the runtime.
    pub fn set_tunnel(&self, wanted: TunnelSettings) -> Result<(), String> {
        self.inner.set_tunnel(wanted)
    }

    /// Stop serving: the tunnel ends, what the dashboard was running is stopped, and the port is
    /// let go.
    pub async fn stop(mut self) {
        self.shut();
        if let Some(mut served) = self.served.take() {
            // Connections a browser keeps open are not waited for.
            if tokio::time::timeout(Duration::from_secs(2), &mut served)
                .await
                .is_err()
            {
                served.abort();
            }
        }
        let jobs = self.inner.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let since = std::time::Instant::now();
            while jobs.jobs.busy() && since.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .await;
    }

    fn shut(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.inner.tunnel.stop();
        self.inner.jobs.stop_all();
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        self.shut();
        if let Some(served) = self.served.take() {
            served.abort();
        }
    }
}

/// Whether a program serves the dashboard: what the settings say, or else its own default.
pub fn enabled(settings: &yardsort_core::settings::DashboardSettings, by_default: bool) -> bool {
    settings.enabled.unwrap_or(by_default)
}

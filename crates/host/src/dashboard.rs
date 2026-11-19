//! The dashboard as this host serves it: on unless the settings say otherwise, at 127.0.0.1, and
//! opened up only from a terminal of the operator's own (`yardsort-host dashboard`).
//!
//! The page, its sign-in and its tunnel are `host_dashboard`'s. Here is what this host tells it
//! of itself, and what the control socket's dashboard commands do.

use std::sync::{Arc, PoisonError};

use host_dashboard::{tunnel, About, Dashboard, Options, Project, RemoteLink, Source};
use yardsort_core::env::ShellEnv;
use yardsort_core::settings::{SettingsFile, TunnelSettings};

use crate::control::{self, DashboardStatus, Reply};
use crate::home::HostHome;

/// A host with no screen serves the dashboard unless told not to.
const ON_BY_DEFAULT: bool = true;

const OFF: &str = "The dashboard is off. Turn it on with `yardsort-host dashboard on`.";

struct HostSource {
    home: Arc<HostHome>,
    remote: crate::dashboard_remote::HostRemote,
}

impl Source for HostSource {
    fn about(&self) -> About {
        let status = crate::serve::status(&self.home);
        About {
            program: "yardsort-host".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            pid: status.pid,
            remote: status.remote.map(|remote| RemoteLink {
                server_url: remote.server_url,
                name: remote.name,
                link: remote.link,
                devices: remote.devices,
            }),
            terminals: status.terminals,
            notices: status
                .earlier_daemon
                .iter()
                .map(control::EarlierDaemon::words)
                .collect(),
            remote_off_hint: "Sign it in to a Yardsort server under Remote.".into(),
        }
    }

    fn env(&self) -> Arc<ShellEnv> {
        self.home.env().clone()
    }

    fn settings(&self) -> &SettingsFile {
        &self.home.settings
    }

    fn projects(&self) -> Vec<Project> {
        self.home
            .store
            .projects()
            .unwrap_or_default()
            .into_iter()
            .map(|project| Project {
                id: project.id,
                name: project.name,
                path: project.root_path,
            })
            .collect()
    }

    fn remote(&self) -> Option<&dyn host_dashboard::Remote> {
        Some(&self.remote)
    }

    fn clone_project(&self, remote: &str) -> Result<Project, String> {
        let (id, name) = self.home.clone_project(remote).map_err(|e| e.message)?;
        let path = self
            .projects()
            .into_iter()
            .find(|project| project.id == id)
            .map(|project| project.path)
            .unwrap_or_default();
        Ok(Project { id, name, path })
    }
}

/// Serve the dashboard, if the settings want it. Says where, or why not, for `serve` to print.
pub async fn start_if_wanted(home: &Arc<HostHome>) -> Option<String> {
    if !host_dashboard::enabled(&home.settings.get().dashboard, ON_BY_DEFAULT) {
        return None;
    }
    Some(match start(home).await {
        Ok(address) => format!("Dashboard at {address}. Sign in with `yardsort-host dashboard`."),
        Err(why) => format!("The dashboard is not served: {why}"),
    })
}

async fn start(home: &Arc<HostHome>) -> Result<String, String> {
    if let Some(dashboard) = &*lock(home) {
        return Ok(dashboard.address());
    }
    let dashboard = Dashboard::start(
        Arc::new(HostSource {
            home: home.clone(),
            remote: crate::dashboard_remote::HostRemote::new(home.clone()),
        }),
        Options {
            state_dir: home.profile.data_dir.join("dashboard"),
        },
    )
    .await?;
    let address = dashboard.address();
    *lock(home) = Some(dashboard);
    Ok(address)
}

/// Stop serving it: its tunnel and what it was running end with it.
pub async fn stop(home: &HostHome) {
    let dashboard = lock(home).take();
    if let Some(dashboard) = dashboard {
        dashboard.stop().await;
    }
}

/// The dashboard as `status` shows it, while it is served.
pub fn status(home: &HostHome) -> Option<DashboardStatus> {
    lock(home).as_ref().map(standing)
}

fn standing(dashboard: &Dashboard) -> DashboardStatus {
    let standing = dashboard.standing();
    DashboardStatus {
        address: standing.address,
        port: standing.port,
        tunnel: tunnel_words(&standing.tunnel),
        signed_in: standing.signed_in,
    }
}

fn tunnel_words(state: &tunnel::State) -> String {
    match state {
        tunnel::State::Off => "off".into(),
        tunnel::State::Starting => "starting".into(),
        tunnel::State::Ready { url } => format!("up at {url}"),
        tunnel::State::Reconnecting { reason } => format!("reconnecting ({reason})"),
        tunnel::State::Failed { message, hint } => format!(
            "not up: {message}{} It is tried again in a while.",
            hint.as_ref()
                .map(|hint| format!(" {hint}"))
                .unwrap_or_default()
        ),
        tunnel::State::Missing => "not up: there is no rustunnel on this user's PATH. Install \
                                   it, run `rustunnel setup`, and turn the tunnel on again."
            .into(),
    }
}

/// Where it is, and a one-time way in. Only ever for a person: the caller has checked.
pub fn sign_in(home: &HostHome) -> Reply {
    let guard = lock(home);
    let Some(dashboard) = guard.as_ref() else {
        return Reply::Failed {
            message: OFF.into(),
        };
    };
    let sign_in = dashboard.sign_in();
    Reply::Dashboard(control::Dashboard {
        status: standing(dashboard),
        code: sign_in.code,
        local: sign_in.local,
        public: sign_in.public,
        good_for: sign_in.good_for.as_secs(),
    })
}

/// Serve it or stop serving it, from now and at every start.
pub async fn enable(home: &Arc<HostHome>, enabled: bool) -> Reply {
    if let Err(error) = home
        .settings
        .update(|settings| settings.dashboard.enabled = Some(enabled))
    {
        return Reply::Failed {
            message: format!("The settings could not be saved: {error}"),
        };
    }
    if !enabled {
        stop(home).await;
        return Reply::Done {
            message: "The dashboard is off, and stays off when the host starts.".into(),
        };
    }
    match start(home).await {
        Ok(address) => Reply::Done {
            message: format!(
                "The dashboard is on, at {address}. Sign in with `yardsort-host dashboard`."
            ),
        },
        Err(message) => Reply::Failed { message },
    }
}

/// Keep a tunnel to it, or end the one there is, from now and at every start.
pub async fn tunnel(home: &Arc<HostHome>, wanted: TunnelSettings) -> Reply {
    let home = home.clone();
    let enabled = wanted.enabled;
    // Off the runtime: starting the tunnel's client resolves the login shell's environment.
    let done = tokio::task::spawn_blocking(move || {
        let guard = lock(&home);
        let Some(dashboard) = guard.as_ref() else {
            return Err(OFF.to_owned());
        };
        dashboard.set_tunnel(wanted)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    match done {
        Err(message) => Reply::Failed { message },
        Ok(()) if enabled => Reply::Done {
            message: "The tunnel is being started. `yardsort-host dashboard` shows its address \
                      once it is up, or why it is not."
                .into(),
        },
        Ok(()) => Reply::Done {
            message: "The tunnel is off, and stays off when the host starts.".into(),
        },
    }
}

/// Sign every browser out, and void the codes not yet used.
pub fn sign_out(home: &HostHome) -> Reply {
    match lock(home).as_ref() {
        None => Reply::Failed {
            message: OFF.into(),
        },
        Some(dashboard) => match dashboard.sign_everyone_out() {
            Ok(ended) => Reply::Done {
                message: match ended {
                    0 => "No browser was signed in. Codes not yet used are void.".into(),
                    1 => "One browser was signed out.".into(),
                    n => format!("{n} browsers were signed out."),
                },
            },
            Err(message) => Reply::Failed { message },
        },
    }
}

fn lock(home: &HostHome) -> std::sync::MutexGuard<'_, Option<Dashboard>> {
    home.dashboard
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_is_said_in_words_with_what_to_do_about_it() {
        assert_eq!(tunnel_words(&tunnel::State::Off), "off");
        assert_eq!(
            tunnel_words(&tunnel::State::Ready {
                url: "https://build-box.example.test".into()
            }),
            "up at https://build-box.example.test"
        );
        let failed = tunnel_words(&tunnel::State::Failed {
            message: "Invalid token.".into(),
            hint: Some("Run rustunnel setup.".into()),
        });
        assert_eq!(
            failed,
            "not up: Invalid token. Run rustunnel setup. It is tried again in a while."
        );
        assert!(tunnel_words(&tunnel::State::Missing).contains("rustunnel setup"));
    }
}

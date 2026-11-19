//! The dashboard as this host serves it: on unless the settings say otherwise, at 127.0.0.1, and
//! opened up only from a terminal of the operator's own (`yardsort-host dashboard`).
//!
//! The page, its sign-in and its tunnel are `host_dashboard`'s. Here is what this host tells it
//! of itself, and what the control socket's dashboard commands do.

use std::sync::{Arc, PoisonError};

use host_dashboard::remote::{self, SignInState, SigningIn};
use host_dashboard::{tunnel, About, Dashboard, Options, Project, RemoteLink, Source};
use remote_client::SIGN_IN_TIMEOUT;
use remote_host::account;
use remote_proto::account::{DeviceKind, NewDevice};
use remote_proto::Platform;
use tokio::sync::{mpsc, oneshot};
use yardsort_core::env::ShellEnv;
use yardsort_core::settings::{SettingsFile, TunnelSettings};

use crate::control::{self, DashboardStatus, Reply};
use crate::home::HostHome;

/// A host with no screen serves the dashboard unless told not to.
const ON_BY_DEFAULT: bool = true;

const OFF: &str = "The dashboard is off. Turn it on with `yardsort-host dashboard on`.";

struct HostSource(Arc<HostHome>, HostRemote);

impl Source for HostSource {
    fn remote(&self) -> Option<&dyn remote::Remote> {
        Some(&self.1)
    }

    fn about(&self) -> About {
        let status = crate::serve::status(&self.0);
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
            remote_off_hint: "Sign in on the Remote tab here. Or at a terminal on this \
                              computer: stop the host, run `yardsort-host sign-in <server>`, and \
                              start it again."
                .into(),
        }
    }

    fn env(&self) -> Arc<ShellEnv> {
        self.0.env().clone()
    }

    fn settings(&self) -> &SettingsFile {
        &self.0.settings
    }

    fn projects(&self) -> Vec<Project> {
        self.0
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

    fn clone_project(&self, remote: &str) -> Result<Project, String> {
        let (id, name) = self.0.clone_project(remote).map_err(|e| e.message)?;
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
    let managing = HostRemote {
        home: home.clone(),
        runtime: tokio::runtime::Handle::current(),
    };
    let dashboard = Dashboard::start(
        Arc::new(HostSource(home.clone(), managing)),
        Options {
            state_dir: home.profile.data_dir.join("dashboard"),
        },
    )
    .await?;
    let address = dashboard.address();
    *lock(home) = Some(dashboard);
    Ok(address)
}

// ── Remote from the page ────────────────────────────────────────────────────────────────────

/// A sign-in to a server that the page started.
#[derive(Default)]
pub enum SignIn {
    #[default]
    None,
    Waiting {
        /// Which sign-in this is: one started after it takes its place.
        turn: u64,
        told: SigningIn,
        /// Where the address the browser came back to is handed in.
        pasted: mpsc::Sender<Pasted>,
    },
    /// The last one came to nothing: why.
    Failed(String),
}

/// An address pasted into the page, and where to say what came of it.
type Pasted = (String, oneshot::Sender<Result<(), String>>);

/// Remote as the dashboard's page manages it. Every method is called off the runtime, by the
/// dashboard, and waits there for the host's own async work.
struct HostRemote {
    home: Arc<HostHome>,
    runtime: tokio::runtime::Handle,
}

impl HostRemote {
    fn signing_in_now(&self) -> std::sync::MutexGuard<'_, SignIn> {
        self.home
            .signing_in
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl remote::Remote for HostRemote {
    fn devices(&self) -> Result<remote::Devices, String> {
        let home = &self.home;
        let asking = self
            .runtime
            .block_on(account::requests(home.as_ref()))
            .map_err(|f| f.message)?;
        let let_in = crate::serve::let_in(home).map_err(|f| f.message)?;
        let projects = crate::serve::project_names(home);
        Ok(remote::Devices {
            devices: let_in
                .into_iter()
                .map(|device| remote::Device {
                    grant: device.grant_id,
                    name: device.name,
                    control: device.control,
                    projects: device.projects,
                })
                .collect(),
            asking: asking
                .into_iter()
                .map(|request| remote::Asking {
                    grant: request.grant_id,
                    name: request.device_name,
                    wants_control: request.wants_control,
                    wants_workflows: request.wants_workflows,
                })
                .collect(),
            // One project is no choice.
            projects: if projects.len() > 1 {
                projects
                    .into_iter()
                    .map(|project| remote::Choice {
                        id: project.id,
                        name: project.name,
                    })
                    .collect()
            } else {
                Vec::new()
            },
        })
    }

    fn offer(&self) -> Result<remote::Offer, String> {
        let offer = self
            .runtime
            .block_on(account::pair(self.home.as_ref()))
            .map_err(|f| f.message)?;
        Ok(remote::Offer {
            code: offer.code,
            link: offer.link,
            expires_in_ms: offer.expires_in_ms,
        })
    }

    fn answer(&self, grant: &str, answer: remote::Answer) -> Result<(), String> {
        let answer = if answer.allow {
            account::Answer::Allow {
                control: answer.control,
                workflows: answer.workflows,
                projects: answer.projects,
            }
        } else {
            account::Answer::Deny
        };
        self.runtime
            .block_on(account::answer(
                self.home.as_ref(),
                grant.to_owned(),
                answer,
            ))
            .map_err(|f| f.message)
    }

    fn revoke(&self, grant: &str) -> Result<String, String> {
        let name = crate::serve::let_in(&self.home)
            .map_err(|f| f.message)?
            .into_iter()
            .find(|device| device.grant_id == grant)
            .map(|device| device.name)
            .ok_or("No such device is let in.")?;
        self.runtime
            .block_on(account::revoke(self.home.as_ref(), grant.to_owned()))
            .map_err(|f| f.message)?;
        Ok(name)
    }

    fn begin_sign_in(&self, server: &str, name: &str) -> Result<SigningIn, String> {
        if let Ok(Some(row)) = self.home.store.remote_host() {
            return Err(format!(
                "This computer is signed in already, as {} at {}. To sign in elsewhere, sign it \
                 out first with `yardsort-host sign-out`.",
                row.name, row.server_url
            ));
        }
        let (client, described, sign_in) = self.runtime.block_on(async {
            let (client, described) = account::prepare(server, name)
                .await
                .map_err(|f| f.message)?;
            let sign_in = client
                .begin_sign_in()
                .await
                .map_err(|e| account::failed(e).message)?;
            Ok::<_, String>((client, described, sign_in))
        })?;
        let told = SigningIn {
            url: sign_in.url().to_string(),
            port: sign_in.redirect_port(),
            server: server.to_owned(),
        };
        let (pasted, mut pastes) = mpsc::channel(4);
        let turn = {
            let mut now = self.signing_in_now();
            let turn = match &*now {
                SignIn::Waiting { turn, .. } => turn + 1,
                _ => 1,
            };
            // One started before this ends here: its way in is dropped, and it sees that.
            *now = SignIn::Waiting {
                turn,
                told: told.clone(),
                pasted,
            };
            turn
        };
        let (home, name) = (self.home.clone(), name.to_owned());
        self.runtime.spawn(async move {
            // Ten minutes from now, however many wrong addresses are pasted meanwhile.
            let until = tokio::time::Instant::now() + SIGN_IN_TIMEOUT;
            let (code, reply) = loop {
                let left = until.saturating_duration_since(tokio::time::Instant::now());
                tokio::select! {
                    // A port forwarded from the browser's computer brings it here by itself.
                    returned = sign_in.returned(left) => match returned {
                        Ok(code) => break (code, None),
                        Err(error) => {
                            ended(&home, turn, Err(account::failed(error).message));
                            return;
                        }
                    },
                    paste = pastes.recv() => match paste {
                        // Given up, or another was started in its place.
                        None => return,
                        Some((address, reply)) => match sign_in.pasted(&address) {
                            Ok(code) => break (code, Some(reply)),
                            // A wrong paste is said, and waited past.
                            Err(error) => {
                                let _ = reply.send(Err(error.to_string()));
                            }
                        },
                    },
                }
            };
            let done = async {
                let device = NewDevice {
                    name: name.clone(),
                    kind: DeviceKind::Host,
                    platform: Platform::current(),
                };
                let signed_in = sign_in
                    .redeem(&client, device, code)
                    .await
                    .map_err(|e| account::failed(e).message)?;
                account::enable(&home, client, &described, &name, signed_in)
                    .await
                    .map_err(|f| f.message)?;
                // Serving already: the link to the relay starts now.
                account::connect(&home);
                Ok::<_, String>(())
            }
            .await;
            ended(&home, turn, done.clone());
            if let Some(reply) = reply {
                let _ = reply.send(done);
            }
        });
        Ok(told)
    }

    fn signing_in(&self) -> SignInState {
        match &*self.signing_in_now() {
            SignIn::None => SignInState::None,
            SignIn::Waiting { told, .. } => SignInState::Waiting(told.clone()),
            SignIn::Failed(message) => SignInState::Failed {
                message: message.clone(),
            },
        }
    }

    fn finish_sign_in(&self, address: &str) -> Result<(), String> {
        // Not held while waiting: the sign-in's own end takes this lock to say it is over.
        let pasted = match &*self.signing_in_now() {
            SignIn::Waiting { pasted, .. } => pasted.clone(),
            _ => return Err("No sign-in is waiting. Start one first.".into()),
        };
        let (reply, said) = oneshot::channel();
        let gone = || "That sign-in is over. Start another.".to_owned();
        self.runtime.block_on(async {
            pasted
                .send((address.to_owned(), reply))
                .await
                .map_err(|_| gone())?;
            said.await.map_err(|_| gone())?
        })
    }

    fn cancel_sign_in(&self) {
        *self.signing_in_now() = SignIn::None;
    }
}

/// A sign-in's end: nothing is waiting any more, and a failure is kept to be said. One that was
/// given up or replaced meanwhile changes nothing.
fn ended(home: &HostHome, turn: u64, done: Result<(), String>) {
    let mut now = home
        .signing_in
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if matches!(&*now, SignIn::Waiting { turn: waiting, .. } if *waiting == turn) {
        *now = match done {
            Ok(()) => SignIn::None,
            Err(why) => SignIn::Failed(why),
        };
    }
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

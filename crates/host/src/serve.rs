//! `yardsort-host serve`: the host, until it is stopped.
//!
//! It holds the profile's lock (as the app does, so the two never share a sign-in), connects to
//! the profile's terminal daemon or starts one, settles the records, drains the activity inbox as
//! it fills, keeps the link to the server's relay when signed in, and answers this computer's own
//! commands on the control socket. Stopped (`SIGTERM`, `SIGINT`), it closes the link and the
//! socket and exits; the daemon and the programs in it carry on, and the next `serve` finds them.

use std::sync::{Arc, OnceLock};

use pty_host::{HostEvent, SessionState};
use remote_host::account::{self, Answer};
use remote_proto::{ProjectGrant, Scope};
use tokio::net::{UnixListener, UnixStream};
use yardsort_core::presence::{AppLock, Holder};
use yardsort_core::remote::host;
use yardsort_core::runtime;

use crate::control::{
    self, Ask, Device, EarlierDaemon, Line, Project, RemoteStatus, Reply, Status,
};
use crate::home::HostHome;
use crate::profile::Profile;

pub async fn run(profile: Profile) -> Result<(), String> {
    let lock = AppLock::take_as(&profile.data_dir, Holder::Host)
        .map_err(|e| format!("Cannot use {}: {e}", profile.data_dir.display()))?
        .ok_or_else(|| {
            format!(
                "Yardsort is already running on {}: the app, or another `yardsort-host serve`.",
                profile.data_dir.display()
            )
        })?;
    let (listener, socket) = control::listen(&profile.control_sockets())?;
    let listener = UnixListener::from_std(listener).map_err(|e| e.to_string())?;

    // The daemon's events need the home, which needs the daemon: filled in just below.
    let shared: Arc<OnceLock<Arc<HostHome>>> = Arc::new(OnceLock::new());
    let events = shared.clone();
    let data_dir = profile.data_dir.clone();
    let connected = tokio::task::spawn_blocking(move || {
        yardsort_core::daemon::connect(
            &data_dir,
            Arc::new(move |event| {
                let HostEvent::Exited { id, exit } = &event else {
                    return;
                };
                if let Some(home) = events.get() {
                    runtime::exited(&home.store, &home.profile.data_dir, &id.0, exit);
                    // An agent's last hooks ran just before it exited.
                    home.look_again();
                }
            }),
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(problem) = &connected.status.problem {
        eprintln!("yardsort-host: the terminal daemon: {problem}");
    }
    if !connected.status.running {
        return Err(
            "No terminal daemon is running for this profile, and none could be started. \
             See the daemon's log in the profile."
                .into(),
        );
    }
    let alive: Vec<String> = connected
        .host
        .list()
        .into_iter()
        .map(|session| session.id.0)
        .collect();
    let home = HostHome::open(profile, Some(connected))?;
    let settled = runtime::settle_at_start(&home.store, &home.profile.data_dir, &alive)?;
    runtime::track_seen(&home.store, &settled.pull_requests, || {
        yardsort_core::git::Git::new(home.env()).ok()
    });
    let watching = {
        let dir = home.profile.data_dir.clone();
        // Weak: the watcher's thread must not keep the host alive past its stop.
        let home = Arc::downgrade(&home);
        runtime::watch_inbox(&dir.clone(), move |cursors| {
            let Some(home) = home.upgrade() else {
                return 0;
            };
            let grok_home = home
                .settings
                .get()
                .activity
                .capture_grok
                .then(|| runtime::grok_home(home.env()))
                .flatten();
            runtime::drain(&home.store, &dir, grok_home.as_deref(), cursors, || {
                yardsort_core::git::Git::new(home.env()).ok()
            })
            .pending
        })
    };
    match watching {
        Ok(watcher) => {
            let _ = home.watcher.set(watcher);
        }
        Err(error) => eprintln!("yardsort-host: not watching the activity inbox: {error}"),
    }
    let _ = shared.set(home.clone());

    match home.store.remote_host() {
        Ok(Some(row)) => {
            account::connect(&home);
            println!(
                "Serving as {} at {}. Pair a device with `yardsort-host pair`.",
                row.name, row.server_url
            );
        }
        _ => println!(
            "Serving, with Remote off: sign in with `yardsort-host sign-in <server>`, after \
             stopping this."
        ),
    }

    let stopped = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|e| e.to_string())?;
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        Ok::<_, String>(())
    };
    let accepting = async {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let home = home.clone();
                    tokio::spawn(async move { answer(&home, stream).await });
                }
                Err(error) => eprintln!("yardsort-host: the control socket: {error}"),
            }
        }
    };
    let outcome = tokio::select! {
        stopped = stopped => stopped,
        () = accepting => Ok(()),
    };
    println!("Stopping. The terminals and the programs in them keep running.");
    account::disconnect(home.as_ref()).await;
    let _ = std::fs::remove_file(&socket);
    drop(lock);
    outcome
}

/// One command's connection.
async fn answer(home: &Arc<HostHome>, stream: UnixStream) {
    let daemon = home.terminals.as_ref().and_then(|t| t.status.pid);
    // Checked before the stream is split: the operating system's word on who is at the other end.
    let refusal = crate::guard::refusal(&stream, daemon);
    let mut line = Line::new(stream);
    let reply = match line.next::<Ask>().await {
        Ok(Some(Ask::Pair)) => match refusal {
            None => return pair(home, &mut line).await,
            Some(message) => Reply::Failed { message },
        },
        Ok(Some(ask)) => one(home, ask)
            .await
            .unwrap_or_else(|f| Reply::Failed { message: f.message }),
        Ok(None) => return,
        Err(message) => Reply::Failed { message },
    };
    let _ = line.send(&reply).await;
}

async fn one(home: &Arc<HostHome>, ask: Ask) -> account::Result<Reply> {
    Ok(match ask {
        Ask::Status => Reply::Status(status(home)),
        Ask::Devices => Reply::Devices {
            devices: devices(home).await?,
        },
        Ask::Revoke { device } => {
            let devices = devices(home).await?;
            let wanted = device.to_lowercase();
            let found: Vec<&Device> = devices
                .iter()
                .filter(|d| d.grant_id.starts_with(&wanted) || d.name.to_lowercase() == wanted)
                .collect();
            let [one] = found.as_slice() else {
                return Err(account::Failure::new(
                    "remote_refused",
                    if found.is_empty() {
                        format!("No device {device:?} is let in. See `yardsort-host devices`.")
                    } else {
                        format!("More than one device is {device:?}. Use the id `devices` shows.")
                    },
                ));
            };
            account::revoke(home.as_ref(), one.grant_id.clone()).await?;
            Reply::Revoked {
                name: one.name.clone(),
            }
        }
        Ask::SignOut => {
            account::disconnect(home.as_ref()).await;
            account::disable(home.as_ref()).await?;
            Reply::SignedOut
        }
        Ask::Pair | Ask::Answer { .. } => Reply::Failed {
            message: "Not here.".into(),
        },
    })
}

fn status(home: &HostHome) -> Status {
    let terminals = home
        .terminals
        .as_ref()
        .map(|t| {
            t.host
                .list()
                .iter()
                .filter(|s| matches!(s.state, SessionState::Running))
                .count()
        })
        .unwrap_or(0);
    let remote = home
        .store
        .remote_host()
        .ok()
        .flatten()
        .map(|row| RemoteStatus {
            server_url: row.server_url,
            name: row.name,
            link: match account::link(home) {
                None => "not kept".into(),
                Some(remote_host::Status::Connecting) => "connecting".into(),
                Some(remote_host::Status::Connected { devices }) => {
                    format!("connected, {devices} device(s) on it now")
                }
                Some(remote_host::Status::Waiting { why, retry_in }) => {
                    format!("waiting ({why}); trying again in {}s", retry_in.as_secs())
                }
                Some(remote_host::Status::Stopped { why }) => {
                    format!("stopped: {why} Sign in again with `yardsort-host sign-in`.")
                }
            },
            devices: host::local_grants(&home.store)
                .map(|g| g.len())
                .unwrap_or(0),
        });
    // Kept after an upgrade because agents run in it (`daemon::connect`): it still runs them, but
    // shares no terminal beyond this computer until it is replaced.
    let earlier_daemon = home.terminals.as_ref().and_then(|t| {
        let client = t.client.as_ref()?;
        (!client.speaks_our_protocol()).then(|| EarlierDaemon {
            version: t.status.version.clone(),
        })
    });
    Status {
        pid: std::process::id(),
        remote,
        terminals,
        earlier_daemon,
    }
}

/// The devices let in, once the server has said which are still in force.
async fn devices(home: &HostHome) -> account::Result<Vec<Device>> {
    // Only ever takes devices off: the server cannot put one on this list.
    if let Err(error) = account::requests(home).await {
        if error.code == "remote_off" {
            return Err(error);
        }
    }
    let names = project_names(home);
    Ok(host::local_grants(&home.store)
        .map_err(|e| account::Failure::new("remote_store", e.to_string()))?
        .into_iter()
        .map(|grant| Device {
            grant_id: grant.id.to_string(),
            name: grant.device_name,
            control: grant.grant.allows(Scope::TerminalControl),
            projects: match grant.grant.projects {
                ProjectGrant::All => None,
                ProjectGrant::Only(ids) => Some(
                    ids.iter()
                        .map(|id| {
                            names
                                .iter()
                                .find(|p| p.id == id.as_str())
                                .map_or_else(|| id.to_string(), |p| p.name.clone())
                        })
                        .collect(),
                ),
            },
        })
        .collect())
}

fn project_names(home: &HostHome) -> Vec<Project> {
    home.store
        .projects()
        .unwrap_or_default()
        .into_iter()
        .map(|p| Project {
            id: p.id,
            name: p.name,
        })
        .collect()
}

/// A `pair`: an offer, then each device that asks, told to the operator and answered as they
/// say. Ends once one is answered, or the offer lapses.
async fn pair(home: &Arc<HostHome>, line: &mut Line) {
    // Listening before the offer is made, so a device quick to ask is not missed.
    let mut asking = home.asking();
    let offer = match account::pair(home.as_ref()).await {
        Ok(offer) => offer,
        Err(failure) => {
            let _ = line
                .send(&Reply::Failed {
                    message: failure.message,
                })
                .await;
            return;
        }
    };
    let lapses = tokio::time::Instant::now()
        + std::time::Duration::from_millis(u64::from(offer.expires_in_ms));
    if line
        .send(&Reply::Offer {
            code: offer.code,
            link: offer.link,
            expires_in_ms: offer.expires_in_ms,
        })
        .await
        .is_err()
    {
        return;
    }
    let projects = project_names(home);
    loop {
        let request = tokio::select! {
            asked = asking.recv() => match asked {
                Ok(request) => request,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            () = tokio::time::sleep_until(lapses) => {
                let _ = line.send(&Reply::Lapsed).await;
                return;
            }
            // The operator gave up: nothing is decided.
            gone = line.next::<Ask>() => {
                if let Ok(Some(_)) = gone { continue; }
                return;
            }
        };
        let told = Reply::Asked {
            grant: request.grant_id.clone(),
            device: request.device_name.clone(),
            wants_control: request.wants_control,
            wants_workflows: request.wants_workflows,
            projects: if projects.len() > 1 {
                projects.clone()
            } else {
                Vec::new()
            },
        };
        if line.send(&told).await.is_err() {
            return;
        }
        let answered = match line.next::<Ask>().await {
            Ok(Some(Ask::Answer {
                grant,
                allow,
                control,
                workflows,
                projects,
            })) if grant == request.grant_id => {
                let answer = if allow {
                    Answer::Allow {
                        control,
                        workflows,
                        projects,
                    }
                } else {
                    Answer::Deny
                };
                account::answer(home.as_ref(), grant, answer)
                    .await
                    .map(|()| allow)
            }
            // Gone, or not an answer to this device: nothing is decided.
            _ => return,
        };
        let reply = match answered {
            Ok(allowed) => Reply::Answered { allowed },
            Err(failure) => Reply::Failed {
                message: failure.message,
            },
        };
        let _ = line.send(&reply).await;
        return;
    }
}

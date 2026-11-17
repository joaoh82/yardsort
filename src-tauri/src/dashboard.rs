//! The host dashboard, as the app serves it: off until it is turned on in Settings → Remote.
//!
//! The page, its sign-in and its tunnel are `host_dashboard`'s, the same as `yardsort-host`
//! serves. Here is what the app tells it of itself, and the commands the settings use. A way in
//! is handed out only from the window, which is where a person is: **Open the dashboard** opens
//! this computer's browser already signed in, and **Copy a sign-in link** is for a browser
//! elsewhere.

use std::sync::{Arc, PoisonError};

use host_dashboard::{tunnel, About, Dashboard, Options, Project, Source};
use pty_host::SessionState;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use yardsort_core::env::ShellEnv;
use yardsort_core::settings::SettingsFile;

use crate::error::{IpcError, IpcResult};
use crate::state::AppState;

/// The app serves no dashboard unless asked to.
const ON_BY_DEFAULT: bool = false;

/// The dashboard, while it is served, and why it is not when it should be.
#[derive(Default)]
pub struct DashboardState {
    serving: tokio::sync::Mutex<Option<Dashboard>>,
    problem: std::sync::Mutex<Option<String>>,
}

struct AppSource(AppHandle);

impl AppSource {
    fn state(&self) -> &AppState {
        self.0.state::<AppState>().inner()
    }
}

impl Source for AppSource {
    fn about(&self) -> About {
        let state = self.state();
        About {
            program: "Yardsort".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            remote: crate::remote::for_dashboard(&self.0),
            terminals: state
                .host
                .list()
                .iter()
                .filter(|session| matches!(session.state, SessionState::Running))
                .count(),
            notices: Vec::new(),
            remote_off_hint: "Turn it on in Yardsort, under Settings → Remote.".into(),
        }
    }

    fn env(&self) -> Arc<ShellEnv> {
        self.state().env()
    }

    fn settings(&self) -> &SettingsFile {
        &self.state().settings
    }

    fn projects(&self) -> Vec<Project> {
        self.state()
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
        let (id, name) =
            crate::remote::clone_for_dashboard(&self.0, remote).map_err(|e| e.message)?;
        let path = self
            .projects()
            .into_iter()
            .find(|project| project.id == id)
            .map(|project| project.path)
            .unwrap_or_default();
        Ok(Project { id, name, path })
    }
}

/// The dashboard as Settings shows it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStatus {
    /// Turned on in the settings.
    pub enabled: bool,
    /// Where it is served, on this computer, while it is.
    pub address: Option<String>,
    /// Its tunnel, in words, when one is wanted: where it is up, or why it is not.
    pub tunnel: Option<String>,
    /// Browsers signed in.
    pub signed_in: u32,
    /// Why it is turned on and not served.
    pub problem: Option<String>,
}

/// A way in for one browser, once.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSignIn {
    /// The address through the tunnel while it is up, and otherwise the one on this computer,
    /// with the code in it.
    pub link: String,
    /// The code alone, to type.
    pub code: String,
    pub good_for_minutes: u32,
}

fn off() -> IpcError {
    IpcError::new("dashboard_off", "The host dashboard is not being served.")
}

fn tunnel_words(state: &tunnel::State) -> Option<String> {
    Some(match state {
        tunnel::State::Off => return None,
        tunnel::State::Starting => "starting".into(),
        tunnel::State::Ready { url } => format!("up at {url}"),
        tunnel::State::Reconnecting { reason } => format!("reconnecting ({reason})"),
        tunnel::State::Failed { message, hint } => format!(
            "not up: {message}{}",
            hint.as_ref()
                .map(|hint| format!(" {hint}"))
                .unwrap_or_default()
        ),
        tunnel::State::Missing => {
            "not up: there is no rustunnel on your PATH. Install it and run `rustunnel setup`."
                .into()
        }
    })
}

async fn status(app: &AppHandle) -> DashboardStatus {
    let state = app.state::<AppState>();
    let enabled = host_dashboard::enabled(&state.settings.get().dashboard, ON_BY_DEFAULT);
    let standing = {
        let serving = state.dashboard.serving.lock().await;
        serving.as_ref().map(Dashboard::standing)
    };
    let problem = state
        .dashboard
        .problem
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    DashboardStatus {
        enabled,
        address: standing.as_ref().map(|s| s.address.clone()),
        tunnel: standing.as_ref().and_then(|s| tunnel_words(&s.tunnel)),
        signed_in: standing
            .as_ref()
            .map_or(0, |s| u32::try_from(s.signed_in).unwrap_or(u32::MAX)),
        problem: problem.filter(|_| enabled && standing.is_none()),
    }
}

/// Serve it, unless it is being served already. A failure is kept for Settings to show.
async fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut serving = state.dashboard.serving.lock().await;
    if serving.is_some() {
        return;
    }
    let started = Dashboard::start(
        Arc::new(AppSource(app.clone())),
        Options {
            state_dir: state.data_dir.join("dashboard"),
        },
    )
    .await;
    let mut problem = state
        .dashboard
        .problem
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match started {
        Ok(dashboard) => {
            *serving = Some(dashboard);
            *problem = None;
        }
        Err(why) => *problem = Some(why),
    }
}

async fn stop(app: &AppHandle) {
    app.state::<AppState>().dashboard.stop().await;
}

impl DashboardState {
    /// Stop serving, and wait until the port is let go.
    async fn stop(&self) {
        let dashboard = self.serving.lock().await.take();
        if let Some(dashboard) = dashboard {
            dashboard.stop().await;
        }
    }

    /// Stop serving from wherever this is called, without waiting on the async runtime when
    /// that cannot be done.
    ///
    /// A thread of the runtime's own may not block on it: Tokio panics rather than deadlock.
    /// From one, the dashboard is taken and dropped instead, which ends its tunnel's client and
    /// stops its jobs at once and lets the port go as the runtime gets to it. Anywhere else,
    /// the stop is waited for.
    fn leave(&self) {
        if tokio::runtime::Handle::try_current().is_ok() {
            // Held only by a command that is starting or stopping it this instant: then the
            // process is on its way out anyway, and takes the listener with it.
            if let Ok(mut serving) = self.serving.try_lock() {
                drop(serving.take());
            }
            return;
        }
        tauri::async_runtime::block_on(self.stop());
    }
}

/// Before the app starts itself again, from a command: stop serving and wait for the port, so
/// that the new copy finds it free. Commands run on the async runtime, where [`leaving`] could
/// not wait.
pub async fn stop_before_restart(app: &AppHandle) {
    if app.try_state::<AppState>().is_some() {
        stop(app).await;
    }
}

/// As the app starts: serve the dashboard if the settings turned it on.
pub fn start_if_wanted(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !host_dashboard::enabled(&state.settings.get().dashboard, ON_BY_DEFAULT) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { start(&app).await });
}

/// As the app leaves: stop serving, which ends the tunnel's client and anything the dashboard
/// was running. Without this they would outlive the app on macOS and Windows. Safe from any
/// thread, the runtime's own included.
pub fn leaving(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.dashboard.leave();
    }
}

#[tauri::command]
#[specta::specta]
pub async fn dashboard_status(app: AppHandle) -> IpcResult<DashboardStatus> {
    Ok(status(&app).await)
}

/// Serve the dashboard or stop serving it, now and whenever the app starts.
#[tauri::command]
#[specta::specta]
pub async fn dashboard_set_enabled(app: AppHandle, enabled: bool) -> IpcResult<DashboardStatus> {
    app.state::<AppState>()
        .settings
        .update(|settings| settings.dashboard.enabled = Some(enabled))
        .map_err(|e| IpcError::new("settings", e.to_string()))?;
    if enabled {
        start(&app).await;
    } else {
        stop(&app).await;
    }
    Ok(status(&app).await)
}

async fn sign_in(app: &AppHandle) -> IpcResult<host_dashboard::SignIn> {
    let state = app.state::<AppState>();
    let serving = state.dashboard.serving.lock().await;
    Ok(serving.as_ref().ok_or_else(off)?.sign_in())
}

/// Open the dashboard in this computer's browser, signed in.
#[tauri::command]
#[specta::specta]
pub async fn dashboard_open(app: AppHandle) -> IpcResult<()> {
    let sign_in = sign_in(&app).await?;
    app.opener()
        .open_url(sign_in.local.as_str(), None::<&str>)
        .map_err(|e| IpcError::new("dashboard_browser", e.to_string()))
}

/// A one-time link that signs a browser in, for the person at the window to take elsewhere.
#[tauri::command]
#[specta::specta]
pub async fn dashboard_sign_in_link(app: AppHandle) -> IpcResult<DashboardSignIn> {
    let sign_in = sign_in(&app).await?;
    Ok(DashboardSignIn {
        link: sign_in.public.unwrap_or(sign_in.local),
        code: sign_in.code,
        good_for_minutes: u32::try_from(sign_in.good_for.as_secs() / 60).unwrap_or(u32::MAX),
    })
}

/// Sign every browser out of the dashboard, and void the links not yet used.
#[tauri::command]
#[specta::specta]
pub async fn dashboard_sign_everyone_out(app: AppHandle) -> IpcResult<DashboardStatus> {
    {
        let state = app.state::<AppState>();
        let serving = state.dashboard.serving.lock().await;
        serving
            .as_ref()
            .ok_or_else(off)?
            .sign_everyone_out()
            .map_err(|why| IpcError::new("dashboard_not_kept", why))?;
    }
    Ok(status(&app).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tunnel_that_is_not_wanted_is_not_mentioned_and_one_that_is_says_how_it_stands() {
        assert_eq!(tunnel_words(&tunnel::State::Off), None);
        assert_eq!(
            tunnel_words(&tunnel::State::Ready {
                url: "https://desk.example.test".into()
            })
            .as_deref(),
            Some("up at https://desk.example.test")
        );
        assert_eq!(
            tunnel_words(&tunnel::State::Failed {
                message: "Invalid token.".into(),
                hint: Some("Run rustunnel setup.".into()),
            })
            .as_deref(),
            Some("not up: Invalid token. Run rustunnel setup.")
        );
    }

    /// A host that is nothing but its settings, to serve a dashboard in a test.
    struct Bare {
        settings: SettingsFile,
    }

    impl Source for Bare {
        fn about(&self) -> About {
            About {
                program: "Yardsort".into(),
                version: "0.0.0".into(),
                pid: std::process::id(),
                remote: None,
                terminals: 0,
                notices: Vec::new(),
                remote_off_hint: String::new(),
            }
        }

        fn env(&self) -> Arc<ShellEnv> {
            Arc::new(ShellEnv {
                vars: std::env::vars().collect(),
                source: yardsort_core::env::EnvSource::Process,
                warning: None,
            })
        }

        fn settings(&self) -> &SettingsFile {
            &self.settings
        }

        fn projects(&self) -> Vec<Project> {
            Vec::new()
        }

        fn clone_project(&self, _remote: &str) -> Result<Project, String> {
            Err("Not here.".into())
        }
    }

    /// A state serving a dashboard on a port of its own, and that port.
    fn serving(dir: &std::path::Path) -> (DashboardState, u16) {
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let path = dir.join("settings.toml");
        std::fs::write(&path, format!("[dashboard]\nport = {port}\n")).unwrap();
        let state = DashboardState::default();
        let dashboard = tauri::async_runtime::block_on(Dashboard::start(
            Arc::new(Bare {
                settings: SettingsFile::load(path),
            }),
            Options {
                state_dir: dir.join("dashboard"),
            },
        ))
        .expect("the dashboard starts");
        assert_eq!(dashboard.port(), port);
        *state.serving.blocking_lock() = Some(dashboard);
        (state, port)
    }

    fn answers(port: u16) -> bool {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    }

    /// Restarting for an update, and replacing the daemon, leave from a command, which runs on
    /// the async runtime: waiting on the runtime there panics.
    #[test]
    fn leaving_from_a_command_on_the_runtime_stops_serving_and_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (state, port) = serving(dir.path());
        assert!(answers(port));
        let state = Arc::new(state);
        let on_the_runtime = state.clone();
        tauri::async_runtime::block_on(tauri::async_runtime::spawn(async move {
            on_the_runtime.leave();
        }))
        .expect("leaving did not panic");
        assert!(state.serving.blocking_lock().is_none());
        let since = std::time::Instant::now();
        while answers(port) {
            assert!(
                since.elapsed() < std::time::Duration::from_secs(10),
                "still listening"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn leaving_as_the_app_exits_waits_until_the_port_is_let_go() {
        let dir = tempfile::tempdir().unwrap();
        let (state, port) = serving(dir.path());
        assert!(answers(port));
        // The exit hook's thread: not one of the runtime's.
        state.leave();
        assert!(!answers(port), "still listening after leaving");
        // And a restart from a command waits the same way, on the runtime.
        let (state, port) = serving(dir.path());
        tauri::async_runtime::block_on(tauri::async_runtime::spawn(async move {
            state.stop().await;
            assert!(!answers(port), "still listening after the stop was awaited");
        }))
        .unwrap();
    }

    #[test]
    fn the_app_serves_no_dashboard_until_it_is_turned_on() {
        let mut settings = yardsort_core::settings::DashboardSettings::default();
        assert!(!host_dashboard::enabled(&settings, ON_BY_DEFAULT));
        settings.enabled = Some(true);
        assert!(host_dashboard::enabled(&settings, ON_BY_DEFAULT));
    }
}

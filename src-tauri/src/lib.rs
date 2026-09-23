//! Yardsort core.
//!
//! The frontend holds no truth: state lives here and the webview renders it. `commands` is the
//! whole IPC surface and stays thin — real work belongs in the domain modules.

mod assist;
mod changes;
mod commands;
#[cfg(target_os = "linux")]
mod display;
mod preflight;
mod publish;
mod quit;
mod sessions;
mod state;
mod terminal;
mod updates;

// The core is its own crate, so the `ys` CLI can use it without linking a webview — see
// `yardsort_core`. It is re-exported under the names this crate has always used, which is why
// `crate::store`, `crate::git` and the rest still resolve everywhere below.
pub use yardsort_core::{
    daemon, env, error, forge, git, harness, legacy, program, settings, store,
};

/// The domain modules whose commands live here but whose logic lives in the core.
mod projects {
    pub mod commands;
    pub use yardsort_core::projects::*;
}

mod workspaces {
    pub mod commands;
    pub use yardsort_core::workspaces::*;
}

pub use yardsort_core::daemon::run_daemon_and_exit_if_asked;
pub use yardsort_core::env::print_env_and_exit_if_asked;

use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder, Event};

/// Where the generated TypeScript bindings live, relative to this crate.
#[cfg(any(debug_assertions, test))]
const BINDINGS_PATH: &str = "../src/lib/bindings.ts";

fn ipc_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_info,
            commands::bench_report,
            projects::commands::projects_list,
            projects::commands::project_open,
            projects::commands::project_create,
            projects::commands::project_remove,
            projects::commands::projects_reorder,
            projects::commands::ui_state_load,
            projects::commands::ui_state_save,
            workspaces::commands::harnesses_list,
            workspaces::commands::harness_save,
            workspaces::commands::harness_reset,
            workspaces::commands::harness_preview,
            workspaces::commands::harness_test,
            workspaces::commands::settings_get,
            workspaces::commands::settings_save_workspaces,
            workspaces::commands::settings_save_general,
            workspaces::commands::project_branches,
            workspaces::commands::workspace_create,
            workspaces::commands::workspace_delete,
            changes::commands::workspace_changes,
            changes::commands::workspace_diff,
            changes::commands::workspace_files,
            changes::commands::workspace_file,
            changes::commands::workspace_watch,
            publish::commands::workspace_publish_state,
            publish::commands::workspace_commit,
            publish::commands::workspace_push,
            publish::commands::workspace_open_pull_request,
            publish::commands::project_pull_requests,
            changes::commands::open_in_editor,
            assist::commands::assist_status,
            assist::commands::assist_save_key,
            assist::commands::assist_forget_key,
            assist::commands::assist_test_key,
            assist::commands::assist_save_settings,
            assist::commands::assist_review,
            assist::commands::assist_suggest,
            sessions::sessions_list,
            sessions::session_resume,
            sessions::session_fork,
            sessions::session_forget,
            workspaces::commands::workspace_archive,
            workspaces::commands::workspace_restore,
            workspaces::commands::workspace_rename,
            preflight::preflight,
            updates::update_check,
            updates::update_install,
            terminal::env_info,
            commands::daemon_status,
            quit::app_quit,
            quit::quit_cancelled,
            terminal::pty_spawn,
            terminal::pty_attach,
            terminal::pty_detach,
            terminal::pty_write,
            terminal::pty_resize,
            terminal::pty_kill,
            terminal::pty_close,
            terminal::pty_list,
        ])
        .events(collect_events![
            terminal::PtyHostEvent,
            changes::commands::WorkspaceFilesChanged,
            quit::QuitRequested
        ])
}

/// Release builds never write bindings: there is no source tree next to an installed app.
#[cfg(any(debug_assertions, test))]
fn export_bindings(builder: &Builder<tauri::Wry>) {
    builder
        .export(
            specta_typescript::Typescript::default().header("// @ts-nocheck\n/* eslint-disable */"),
            BINDINGS_PATH,
        )
        .expect("failed to export TypeScript bindings");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before anything starts GTK or a thread: the backend is read once, when GTK initialises.
    #[cfg(target_os = "linux")]
    display::choose_backend();

    let builder = ipc_builder();

    // Keep the checked-in bindings fresh while developing. CI verifies they are not stale.
    #[cfg(debug_assertions)]
    export_bindings(&builder);

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::PendingUpdate::default())
        .invoke_handler(builder.invoke_handler())
        .on_page_load(|_, payload| {
            #[cfg(target_os = "linux")]
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                display::window_is_up();
            }
            #[cfg(not(target_os = "linux"))]
            let _ = payload;
        })
        // Agents outlive the window now, so closing it is a decision, not a side effect.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if !quit::may_close_now(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .setup(move |app| {
            builder.mount_events(app);

            // `YARDSORT_DATA_DIR` keeps experiments and tests away from the real database (and
            // settings, which then sit next to it instead of in the OS config directory).
            let profile = legacy::env_var_os("DATA_DIR").map(std::path::PathBuf::from);
            let data_dir = match &profile {
                Some(dir) => dir.clone(),
                None => app.path().app_data_dir()?,
            };
            let config_dir = match &profile {
                Some(dir) => dir.clone(),
                None => app.path().app_config_dir()?,
            };
            // First launch after the rename from Switchyard: bring the user's data along.
            if profile.is_none() {
                match legacy::adopt_switchyard_data(&data_dir, "yardsort.db", &config_dir) {
                    Ok(adopted) => adopted
                        .iter()
                        .for_each(|what| eprintln!("carried over from Switchyard: {what}")),
                    Err(error) => eprintln!("could not carry Switchyard data over: {error}"),
                }
            }
            // The `ys` CLI has to find this same directory without Tauri to ask, so it works the
            // rules out itself (`yardsort_core::paths`). If the two ever disagree the CLI would
            // quietly look at the wrong database, so say so loudly here rather than there.
            for (what, ours, theirs) in [
                ("data", &data_dir, yardsort_core::paths::data_dir()),
                ("config", &config_dir, yardsort_core::paths::config_dir()),
            ] {
                if theirs.as_ref() != Some(ours) {
                    eprintln!(
                        "warning: the {what} directory is {} but yardsort_core::paths says {:?} — \
                         the ys CLI will need --data-dir",
                        ours.display(),
                        theirs,
                    );
                }
            }

            let database = data_dir.join("yardsort.db");
            let store = store::Store::open(&database)
                .map_err(|e| format!("cannot open {}: {e}", database.display()))?;
            let settings = settings::SettingsFile::load(config_dir.join("settings.toml"));

            // Find the daemon for this profile, or start one. Agents from a previous run of the
            // app are still in it, which is why this comes before the records are settled.
            let handle = app.handle().clone();
            let connected = daemon::connect(
                &data_dir,
                std::sync::Arc::new(move |event| {
                    // Settle the record first, so a client reacting to the event reads the truth.
                    if let pty_host::HostEvent::Exited { id, exit } = &event {
                        if let Some(state) = handle.try_state::<state::AppState>() {
                            let _ = state
                                .store
                                .end_session_by_pty(&id.0, Some(i64::from(exit.code)));
                        }
                    }
                    let _ = terminal::PtyHostEvent(event).emit(&handle);
                }),
            );
            // Rows still marked running whose process is *not* among these died with the
            // previous run; the rest are conversations that never stopped.
            let alive: Vec<String> = connected
                .host
                .list()
                .into_iter()
                .map(|session| session.id.0)
                .collect();
            store
                .end_interrupted_sessions(&alive)
                .map_err(|e| format!("cannot tidy session records: {e}"))?;

            app.manage(state::AppState::new(connected, store, settings));

            // Warm the login-shell environment now, so the first terminal doesn't wait for it.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                handle.state::<state::AppState>().env();
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Yardsort");
}

#[cfg(test)]
mod tests {
    /// `bun run bindings` runs this to regenerate `src/lib/bindings.ts` without launching the app.
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::ipc_builder());
    }
}

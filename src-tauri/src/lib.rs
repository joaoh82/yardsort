//! Yardsort core.
//!
//! The frontend holds no truth: state lives here and the webview renders it. `commands` is the
//! whole IPC surface and stays thin — real work belongs in the domain modules.

mod activity;
mod assist;
mod changes;
mod commands;
#[cfg(target_os = "linux")]
mod display;
mod draft;
mod memory;
mod outcomes;
mod preflight;
mod publish;
mod quit;
mod sessions;
mod state;
mod terminal;
mod updates;
mod usage;
mod workflows;
mod ys;

// The core is its own crate, so the `ys` CLI can use it without linking a webview — see
// `yardsort_core`. It is re-exported under the names this crate has always used, which is why
// `crate::store`, `crate::git` and the rest still resolve everywhere below.
pub use yardsort_core::{
    daemon, env, error, forge, git, harness, legacy, program, settings, store, tasks,
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

pub use yardsort_core::activity::hook::run_hook_and_exit_if_asked;
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
            projects::commands::project_automation_get,
            projects::commands::project_automation_save,
            projects::commands::workspace_run,
            projects::commands::project_create,
            projects::commands::project_clone,
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
            changes::commands::workspace_save_file,
            changes::commands::workspace_watch,
            publish::commands::workspace_publish_state,
            publish::commands::workspace_commit,
            publish::commands::workspace_push,
            publish::commands::workspace_open_pull_request,
            publish::commands::workspace_merge_pull_request,
            publish::commands::project_pull_requests,
            publish::pull_requests::pull_request_summary,
            publish::pull_requests::pull_request_changes,
            publish::pull_requests::pull_request_diff,
            publish::pull_requests::pull_request_comment,
            publish::pull_requests::pull_request_line_comments,
            publish::pull_requests::pull_request_line_comment,
            publish::pull_requests::pull_request_note_helper,
            publish::pull_requests::pull_request_send_note,
            publish::pull_requests::pull_request_note_text,
            publish::pull_requests::pull_request_merge,
            publish::pull_requests::pull_request_close,
            publish::pull_requests::pull_request_reopen,
            publish::pull_requests::pull_request_prepare_branch,
            publish::tasks::project_tasks,
            publish::tasks::task_detail,
            publish::conflicts::workspace_conflict_helper,
            publish::conflicts::workspace_resolve_conflicts,
            draft::commands::draft_status,
            draft::commands::workflow_writer_status,
            draft::commands::workflow_save_writer,
            draft::commands::draft_commit_message,
            draft::commands::draft_pull_request,
            draft::commands::draft_save_key,
            draft::commands::draft_forget_key,
            draft::commands::draft_save_settings,
            changes::commands::open_in_editor,
            changes::commands::workspace_reveal_file,
            assist::commands::assist_status,
            assist::commands::assist_save_key,
            assist::commands::assist_forget_key,
            assist::commands::assist_test_key,
            assist::commands::assist_save_settings,
            assist::commands::assist_review,
            assist::commands::assist_suggest,
            activity::activity_timeline,
            activity::activity_diagnostics,
            activity::activity_clear,
            activity::workspace_provenance,
            activity::workspace_handoff,
            memory::memory_get,
            memory::memory_write,
            memory::memory_edit,
            memory::memory_decide,
            memory::memory_share,
            memory::memory_waiting,
            memory::memory_check,
            outcomes::outcomes_get,
            outcomes::outcome_label,
            outcomes::outcomes_agents,
            activity::settings_save_activity,
            sessions::sessions_list,
            sessions::session_context,
            sessions::session_resume,
            sessions::session_fork,
            sessions::session_forget,
            workspaces::commands::workspace_archive,
            workspaces::commands::workspace_restore,
            workspaces::commands::workspace_rename,
            workspaces::commands::project_untracked_worktrees,
            workspaces::commands::workspaces_import,
            workspaces::commands::workspace_forget,
            preflight::preflight,
            updates::update_check,
            updates::update_install,
            ys::ys_status,
            ys::ys_install,
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
            workflows::commands::workflow_list,
            workflows::commands::workflow_check,
            workflows::commands::workflow_save,
            workflows::commands::workflow_copy,
            workflows::commands::workflow_remove,
            workflows::commands::workflow_runs,
            workflows::commands::workflow_run_steps,
            workflows::commands::workflow_preview,
            workflows::commands::workflow_start,
            workflows::commands::workflow_cancel,
            workflows::commands::workflow_describe,
            usage::usage_machine,
            usage::usage_tokens,
            usage::settings_save_usage,
        ])
        .events(collect_events![
            terminal::PtyHostEvent,
            changes::commands::WorkspaceFilesChanged,
            activity::ActivityChanged,
            quit::QuitRequested,
            workflows::SessionStarted,
            workflows::commands::WorkflowRunsChanged
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
    // First: everything below, and every process started from here, finds the profile by it.
    #[cfg(debug_assertions)]
    use_a_development_profile();

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
            if let Err(reason) = start(app) {
                refuse_to_start(app.handle(), &reason.to_string());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running Yardsort")
        .run(|app, event| {
            // A start that failed has no window left, only its dialog. Stay until it is closed.
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                if app.try_state::<StartFailed>().is_some() {
                    api.prevent_exit();
                }
            }
        });
}

/// A development build keeps to a profile of its own unless `YARDSORT_DATA_DIR` names one. It can
/// carry migrations no release has yet, and a database it upgrades is one the installed release
/// then refuses: on 2026-09-29 that kept 0.12.0 from starting at all. Setting the variable, not
/// just using the directory, is what makes the daemon, agents' hooks and `ys` agree with the app.
#[cfg(debug_assertions)]
fn use_a_development_profile() {
    if legacy::env_var_os("DATA_DIR").is_some() {
        return;
    }
    let Some(dir) = yardsort_core::paths::development_data_dir() else {
        return;
    };
    eprintln!(
        "development build: using the profile in {} (set YARDSORT_DATA_DIR to choose another)",
        dir.display()
    );
    std::env::set_var("YARDSORT_DATA_DIR", dir);
}

/// Everything the app needs before its window can be used. An error here is shown to the user by
/// [`refuse_to_start`], so say what failed in words they can act on.
fn start(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
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
    let store = store::Store::open(&database).map_err(|e| store_failure(&database, &e))?;
    let settings = settings::SettingsFile::load(config_dir.join("settings.toml"));

    // Find the daemon for this profile, or start one. Agents from a previous run of the
    // app are still in it, which is why this comes before the records are settled.
    let handle = app.handle().clone();
    let spool_root = data_dir.clone();
    let connected = daemon::connect(
        &data_dir,
        std::sync::Arc::new(move |event| {
            // Settle the record first, so a client reacting to the event reads the truth.
            if let pty_host::HostEvent::Exited { id, exit } = &event {
                if let Some(state) = handle.try_state::<state::AppState>() {
                    let _ = state
                        .store
                        .end_session_by_pty(&id.0, Some(i64::from(exit.code)));
                    yardsort_core::activity::record_exit(
                        &state.store,
                        &id.0,
                        &yardsort_core::activity::ExitFacts::from(exit),
                        yardsort_core::activity::Via::Live,
                    );
                    // The daemon kept this exit for us too; take it out of the spool
                    // now rather than find it as a duplicate at the next start. And
                    // an agent's last hooks ran just before it exited.
                    yardsort_core::activity::import_spool(&state.store, &spool_root);
                    activity::drain_inbox(&handle, &state.store, &spool_root);
                }
            }
            // A workflow waiting for an agent to settle hears it here, before the window.
            if let Some(driver) = handle.try_state::<workflows::Driver>() {
                driver.observe(&event);
            }
            let _ = terminal::PtyHostEvent(event).emit(&handle);
        }),
    );
    // What agents reported while no window was open.
    let inbox = yardsort_core::activity::import_inbox(&store, &data_dir);
    if inbox.imported > 0 || inbox.unlinked > 0 || inbox.unreadable > 0 || inbox.dropped > 0 {
        eprintln!(
            "activity inbox: {} imported, {} duplicate, {} unlinked, {} unreadable, {} dropped",
            inbox.imported, inbox.duplicates, inbox.unlinked, inbox.unreadable, inbox.dropped
        );
    }
    // Exits the daemon kept while no window was open come first: a conversation that
    // ended cleanly in the meantime must keep its exit code rather than be counted as
    // interrupted below.
    let imported = yardsort_core::activity::import_spool(&store, &data_dir);
    if imported.imported > 0 || imported.unreadable > 0 || imported.dropped > 0 {
        eprintln!(
            "exit spool: {} imported, {} duplicate, {} unmatched, {} unreadable, {} dropped",
            imported.imported,
            imported.duplicates,
            imported.unmatched,
            imported.unreadable,
            imported.dropped
        );
    }
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
    yardsort_core::activity::end_interrupted(&store, &alive);
    yardsort_core::activity::prune(&store);

    app.manage(state::AppState::new(
        data_dir.clone(),
        connected,
        store,
        settings,
    ));

    // Hooks write to the inbox whenever an agent does something; watch it, so the
    // timeline moves while the agent works rather than when it exits.
    match activity::watch_inbox(app.handle().clone(), &data_dir) {
        Ok(watcher) => {
            *app.state::<state::AppState>().inbox_watcher.lock().unwrap() = Some(watcher);
        }
        Err(error) => eprintln!("not watching the activity inbox: {error}"),
    }

    // Workflow runs, queued here or by `ys`, are moved on by a thread of their own.
    workflows::start(app.handle(), &data_dir);

    // Warm the login-shell environment now, so the first terminal doesn't wait for it.
    // Then bring a `ys` an older version installed up to this one. Not from a development
    // build, which would put itself on the user's PATH, unless `YARDSORT_YS_DIR` says where.
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let env = handle.state::<state::AppState>().env();
        if cfg!(debug_assertions) && legacy::env_var_os("YS_DIR").is_none() {
            return;
        }
        if let Some(what) = ys::refresh(&ys::Layout::detect(&env), &env) {
            eprintln!("ys: {what}");
        }
    });
    Ok(())
}

/// Managed only when [`start`] failed: the app is waiting for its error dialog to be closed.
struct StartFailed;

/// Tauri turns an error from the setup hook into a panic, and release builds abort on panic: the
/// process dies without a word, and when it was started from a launcher nobody sees its stderr.
/// That is how a database written by a newer Yardsort once looked like a crash on every start.
/// So say why in a dialog instead, and exit once it is closed.
fn refuse_to_start(app: &tauri::AppHandle, reason: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    eprintln!("yardsort: cannot start: {reason}");
    // Whatever failed, it was not the display server: do not pin this version to X11 for it.
    #[cfg(target_os = "linux")]
    display::start_failed();
    app.manage(StartFailed);
    // Nothing behind the window works without the state `start` did not finish, so no page.
    for window in app.webview_windows().into_values() {
        let _ = window.destroy();
    }
    let handle = app.clone();
    app.dialog()
        .message(reason)
        .title("Yardsort cannot start")
        .kind(MessageDialogKind::Error)
        .show(move |_| {
            // Not `handle.exit(1)`: under `run` the code does not reach the process, and a
            // launcher or script should see that the start failed.
            handle.cleanup_before_exit();
            std::process::exit(1);
        });
}

/// Why the database would not open, for the person looking at the dialog.
fn store_failure(database: &std::path::Path, error: &store::StoreError) -> String {
    match error {
        // Only an older Yardsort opening data a newer one has upgraded gets here. The store
        // refuses before it writes anything, so the data is as the newer version left it.
        store::StoreError::TooNew { .. } => {
            let copy = store::readable_copy_before_upgrade(database)
                .map(|copy| {
                    format!(
                        "To keep using this version instead, there is a copy from before the \
                         upgrade at {}. It has nothing done since; Troubleshooting → \
                         \"Yardsort cannot start\" explains how to use it.\n\n",
                        copy.display()
                    )
                })
                .unwrap_or_default();
            format!(
                "Your projects and workspaces were last opened by a newer version of Yardsort, \
                 and this one ({}) cannot read them.\n\n\
                 Install the latest Yardsort from https://yardsort.sh to carry on. Nothing has \
                 been changed or lost.\n\n\
                 {copy}{}: {error}",
                env!("CARGO_PKG_VERSION"),
                database.display()
            )
        }
        _ => format!("cannot open {}: {error}", database.display()),
    }
}

#[cfg(test)]
mod tests {
    /// `bun run bindings` runs this to regenerate `src/lib/bindings.ts` without launching the app.
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::ipc_builder());
    }

    /// Opening a link in the browser needs a URL **scope**, not just the command.
    ///
    /// `opener:allow-open-url` on its own enables the command with an empty scope, which denies
    /// every URL — and denies it silently, in the webview console, where nothing in CI looks.
    /// That shipped: release notes, the welcome screen's install links and Ctrl+clicking a URL
    /// in a terminal were all dead. This is the guard, because the failure has no other alarm.
    #[test]
    fn opening_a_url_is_allowed_for_http_and_https() {
        let capabilities = include_str!("../capabilities/default.json");
        let parsed: serde_json::Value = serde_json::from_str(capabilities).expect("valid JSON");
        let opener = parsed["permissions"]
            .as_array()
            .expect("permissions is a list")
            .iter()
            .find(|entry| entry["identifier"] == "opener:allow-open-url")
            .expect("opener:allow-open-url must carry a scope, not be a bare string");

        let allowed: Vec<&str> = opener["allow"]
            .as_array()
            .expect("an allow list")
            .iter()
            .filter_map(|entry| entry["url"].as_str())
            .collect();
        assert!(allowed.contains(&"https://*"), "got {allowed:?}");
        assert!(allowed.contains(&"http://*"), "got {allowed:?}");
    }

    /// An older Yardsort started on data a newer one upgraded used to abort without a word. What
    /// it says instead has to tell the person what to do, and that their data is safe.
    #[test]
    fn a_database_from_a_newer_version_is_explained_not_just_reported() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("yardsort.db");
        drop(super::store::Store::open(&database).unwrap());
        rusqlite::Connection::open(&database)
            .unwrap()
            .pragma_update(None, "user_version", 999)
            .unwrap();

        let error = match super::store::Store::open(&database) {
            Err(error) => error,
            Ok(_) => panic!("a database from the future must be refused"),
        };
        let message = super::store_failure(&database, &error);

        assert!(message.contains("newer version of Yardsort"), "{message}");
        assert!(message.contains(env!("CARGO_PKG_VERSION")), "{message}");
        assert!(message.contains("Install the latest Yardsort"), "{message}");
        assert!(
            message.contains("Nothing has been changed or lost"),
            "{message}"
        );
        assert!(
            message.contains(&database.display().to_string()),
            "{message}"
        );
        assert!(
            !message.contains("copy from before"),
            "there is none: {message}"
        );
    }

    /// With a copy from before the upgrade, the dialog says there is a way back to this version.
    #[test]
    fn a_database_from_a_newer_version_points_at_the_copy_from_before() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("yardsort.db");
        let copy = super::store::copy_before_upgrade(&database);
        drop(super::store::Store::open(&copy).unwrap());
        drop(super::store::Store::open(&database).unwrap());
        rusqlite::Connection::open(&database)
            .unwrap()
            .pragma_update(None, "user_version", 999)
            .unwrap();

        let error = match super::store::Store::open(&database) {
            Err(error) => error,
            Ok(_) => panic!("a database from the future must be refused"),
        };
        let message = super::store_failure(&database, &error);

        assert!(
            message.contains("copy from before the upgrade"),
            "{message}"
        );
        assert!(message.contains(&copy.display().to_string()), "{message}");
    }
}

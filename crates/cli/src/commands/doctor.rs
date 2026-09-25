//! `ys doctor` — where everything is, and whether it can be reached.
//!
//! Mostly here because resolving the data directory without Tauri is the one thing that could
//! silently go wrong on a platform nobody has tried this on yet. It answers "is `ys` looking at
//! the same Yardsort you are?" without needing the app.

use std::path::PathBuf;

use serde::Serialize;
use yardsort_core::paths;

use crate::{table, Failure, Output, Yardsort};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    data_dir: Option<String>,
    config_dir: Option<String>,
    database: Option<String>,
    database_found: bool,
    daemon_running: bool,
    daemon_version: Option<String>,
    daemon_endpoint: Option<String>,
    projects: Option<usize>,
    harnesses_installed: Vec<String>,
    /// Recorded activity: events and runs in the database, exits waiting in the daemon's spool.
    activity_events: Option<i64>,
    activity_runs: Option<i64>,
    spool_dir: Option<String>,
    spool_pending: Option<usize>,
}

pub fn run(data_dir: Option<PathBuf>, out: &Output) -> Result<(), Failure> {
    let resolved = data_dir.clone().or_else(paths::data_dir);
    let database = resolved.as_ref().map(|dir| dir.join("yardsort.db"));
    // A profile named on the command line keeps its settings beside its database, as `--data-dir`
    // and `YARDSORT_DATA_DIR` both mean "this whole profile lives here".
    let config = match &data_dir {
        Some(dir) => Some(dir.clone()),
        None => paths::config_dir(),
    };

    let mut report = Report {
        data_dir: resolved.as_ref().map(|d| d.display().to_string()),
        config_dir: config.map(|d| d.display().to_string()),
        database: database.as_ref().map(|d| d.display().to_string()),
        database_found: database.as_ref().is_some_and(|d| d.is_file()),
        daemon_running: false,
        daemon_version: None,
        daemon_endpoint: None,
        projects: None,
        harnesses_installed: Vec::new(),
        activity_events: None,
        activity_runs: None,
        spool_dir: None,
        spool_pending: None,
    };

    // Everything past this point needs the database, and not having one is the thing most worth
    // reporting clearly — so it is a finding, not an error.
    if report.database_found {
        let ys = Yardsort::open(data_dir)?;
        report.projects = Some(ys.store.projects()?.len());
        if let Some(client) = ys.daemon() {
            let info = client.daemon_info();
            report.daemon_running = true;
            report.daemon_version = Some(info.version.clone());
        }
        report.daemon_endpoint = Some(yardsort_core::daemon::endpoint_for(&ys.data_dir));
        if let Ok((events, runs)) = ys.store.activity_counts() {
            report.activity_events = Some(events);
            report.activity_runs = Some(runs);
        }
        let spool = pty_ipc::spool::Spool::new(yardsort_core::activity::spool_dir(&ys.data_dir));
        report.spool_dir = Some(spool.dir().display().to_string());
        report.spool_pending = spool.entries().ok().map(|entries| entries.len());
        let cwd = std::env::current_dir().unwrap_or_default();
        report.harnesses_installed = yardsort_core::harness::resolve_all(&ys.settings.harnesses)
            .into_iter()
            .filter(|r| ys.env().find_program(&r.def.command, &cwd).is_some())
            .map(|r| r.def.id)
            .collect();
    }

    out.emit(&report, || {
        let yes_no = |b: bool| if b { "yes" } else { "no" }.to_owned();
        let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".to_owned());
        let mut rows = vec![
            vec!["data dir".to_owned(), or_unknown(&report.data_dir)],
            vec!["config dir".to_owned(), or_unknown(&report.config_dir)],
            vec![
                "database".to_owned(),
                format!(
                    "{} ({})",
                    or_unknown(&report.database),
                    if report.database_found {
                        "found"
                    } else {
                        "NOT FOUND"
                    }
                ),
            ],
        ];
        if report.database_found {
            rows.push(vec![
                "projects".to_owned(),
                report.projects.unwrap_or(0).to_string(),
            ]);
            rows.push(vec![
                "daemon".to_owned(),
                match &report.daemon_version {
                    Some(version) => format!("running, version {version}"),
                    None => "not running".to_owned(),
                },
            ]);
            rows.push(vec![
                "socket".to_owned(),
                or_unknown(&report.daemon_endpoint),
            ]);
            rows.push(vec![
                "agents on PATH".to_owned(),
                if report.harnesses_installed.is_empty() {
                    "none".to_owned()
                } else {
                    report.harnesses_installed.join(", ")
                },
            ]);
            rows.push(vec![
                "activity".to_owned(),
                format!(
                    "{} events, {} runs",
                    report.activity_events.unwrap_or(0),
                    report.activity_runs.unwrap_or(0)
                ),
            ]);
            rows.push(vec![
                "exit spool".to_owned(),
                format!(
                    "{} ({} waiting)",
                    or_unknown(&report.spool_dir),
                    report.spool_pending.unwrap_or(0)
                ),
            ]);
        }
        let _ = yes_no;
        table(&rows);
        if !report.database_found {
            println!("\nNo database there. Open Yardsort once to create one, or pass --data-dir.");
        }
    })
}

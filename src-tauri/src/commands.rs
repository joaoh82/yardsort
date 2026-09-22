//! The IPC surface. Every function here is callable from the webview.

use serde::Serialize;
use specta::Type;

/// Static facts about the running app, shown in the UI and useful in bug reports.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// `linux`, `macos` or `windows`.
    pub os: String,
    pub arch: String,
    pub debug: bool,
    pub dev: DevFlags,
}

/// Switches for measuring and debugging, read from the environment. Always empty in release
/// builds.
#[derive(Debug, Clone, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DevFlags {
    /// `YARDSORT_BENCH`: a shell script to run in a terminal while frame times are recorded;
    /// the app prints the result and exits. See `docs/design/07-terminal-benchmarks.md`.
    pub bench: Option<String>,
    /// `YARDSORT_BENCH_LATENCY`: time keystroke round trips in a terminal instead of running a
    /// script, then print the result and exit. Takes precedence over `bench`.
    pub bench_latency: bool,
    /// `YARDSORT_RENDERER`: force the terminal renderer (`webgl` or `dom`).
    pub renderer: Option<String>,
}

impl DevFlags {
    fn from_env() -> Self {
        if !cfg!(debug_assertions) {
            return Self::default();
        }
        let var =
            |suffix| crate::legacy::env_var_os(suffix).map(|v| v.to_string_lossy().into_owned());
        Self {
            bench: var("BENCH"),
            bench_latency: var("BENCH_LATENCY").is_some_and(|value| !value.is_empty()),
            renderer: var("RENDERER"),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Yardsort".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        debug: cfg!(debug_assertions),
        dev: DevFlags::from_env(),
    }
}

/// Receives the result of a `YARDSORT_BENCH` run, prints it as one line of JSON and quits.
#[tauri::command]
#[specta::specta]
pub fn bench_report(app: tauri::AppHandle, report: String) {
    if cfg!(debug_assertions) {
        println!("YARDSORT_BENCH_RESULT {report}");
        app.exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_this_build() {
        let info = app_info();
        assert_eq!(info.name, "Yardsort");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(["linux", "macos", "windows"].contains(&info.os.as_str()));
        assert!(!info.arch.is_empty());
    }
}

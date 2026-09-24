//! Yardsort's core, with no user interface attached.
//!
//! Everything here is shared by every client: the SQLite store, git, projects and workspaces,
//! settings, the harness definitions, the environment programs are launched in, and the terminal
//! daemon this process talks to. The desktop app ([`yardsort_lib`](../yardsort_lib/index.html))
//! adds the webview and the Tauri commands on top; the `ys` CLI is a second client of the same
//! core.
//!
//! The rule that keeps it that way: **nothing in this crate may depend on Tauri.** Errors are
//! still `serde`/`specta` shaped, because the app's generated TypeScript bindings are built from
//! these types, but that is reflection, not a UI.

pub mod assist;
pub mod daemon;
pub mod draft;
pub mod env;
pub mod error;
pub mod forge;
pub mod git;
pub mod harness;
pub mod launch;
pub mod legacy;
pub mod paths;
pub mod program;
pub mod project_automation;
pub mod projects;
pub mod settings;
pub mod store;
pub mod workspaces;

//! The app's side of a workspace's changes: the commands, the file tree and the watcher. What
//! changed, and how a diff is read, is the core's (`yardsort_core::changes`), shared with `ys`.

pub mod commands;
pub mod files;
pub mod watch;

pub use yardsort_core::changes::*;

//! Finding a command-line program on the user's `PATH` and starting it the way Yardsort always
//! starts programs: with an argv array, in the user's environment, with nothing attached to
//! stdin and no console window flashing up on Windows.
//!
//! [`Git`](crate::git::Git) and [`Gh`](crate::forge::Gh) are both this plus their own arguments.
//! It lives here so the platform quirks are written once: a second copy is a second thing to
//! remember when one of them changes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::env::ShellEnv;

#[derive(Debug, Clone)]
pub struct Program {
    path: PathBuf,
    env: Vec<(String, String)>,
    /// The resolved environment replaces the inherited one rather than adding to it.
    clear_env: bool,
}

impl Program {
    /// Look for `name` on the user's `PATH`, as their login shell resolved it.
    pub fn find(env: &ShellEnv, name: &str) -> Option<Self> {
        let cwd = std::env::current_dir().unwrap_or_default();
        Some(Self::at(env.find_program(name, &cwd)?, env))
    }

    /// The same, for a program whose path is already known.
    pub fn at(path: PathBuf, env: &ShellEnv) -> Self {
        Self {
            path,
            env: env
                .vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            clear_env: env.replaces_inherited(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A [`Command`] ready to have arguments added. Callers add their own environment on top.
    pub fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(&self.path);
        if self.clear_env {
            command.env_clear();
        }
        command
            .current_dir(cwd)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            // Nothing here is interactive: a program that decides to ask a question must fail
            // instead of waiting for an answer nobody can give it.
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// Helpers for tests, in this crate and in the app's. Behind a feature so they never reach a
/// release build: `yardsort-core = { features = ["testing"] }` in dev-dependencies.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::*;

    /// Write `contents` to `path` and make it executable: a stand-in `gh`, `ys` or agent for a
    /// test to run.
    ///
    /// A child process does the writing, not this one. Tests run on many threads, and some of
    /// them start real processes: every fork copies the descriptors this process has open at
    /// that moment, and the copies stay open in the child until it execs — longer for a PTY
    /// child, which sets up its terminal first. A stub being written here is among them, and
    /// running it in that window fails with `ETXTBSY`, "Text file busy": Linux refuses to run a
    /// file that any process holds open for writing. That is how
    /// `forge::tests::closing_and_reopening_ask_for_nothing_more` stopped a release check on
    /// 2026-10-21. A descriptor this process never had cannot be copied.
    #[cfg(unix)]
    pub fn executable(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        use std::os::unix::ffi::OsStrExt;
        let path = path.as_ref();
        let status = Command::new("sh")
            .arg("-c")
            .arg(r#"printf '%s' "$1" > "$2" && chmod 755 "$2""#)
            .arg("sh")
            .arg(std::ffi::OsStr::from_bytes(contents.as_ref()))
            .arg(path)
            .stdin(Stdio::null())
            .status()
            .expect("sh starts");
        assert!(status.success(), "could not write {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    /// Stubs are run while other tests are forking. Threads here fork without pause, each child
    /// keeping its copies of our descriptors a moment before it execs, as a PTY child does; a
    /// stub written with `std::fs::write` meanwhile is caught open by one of those forks within
    /// a few dozen tries, and running it fails with "Text file busy".
    #[cfg(unix)]
    #[test]
    fn a_stub_runs_at_once_while_other_tests_are_forking() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        // `spawn` waits for its child to exec, so one thread forks once per hold; four give
        // four forks per hold, and a fork in flight for almost every write.
        let forking: Vec<_> = (0..4)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut children = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let mut command = Command::new("true");
                        // A hook forces a plain fork, and keeps the child between fork and exec.
                        unsafe {
                            command.pre_exec(|| {
                                std::thread::sleep(Duration::from_millis(2));
                                Ok(())
                            });
                        }
                        children.push(command.spawn().unwrap());
                    }
                    for mut child in children {
                        child.wait().unwrap();
                    }
                })
            })
            .collect();

        let stub = dir.path().join("stub");
        for round in 0..200 {
            super::testing::executable(&stub, format!("#!/bin/sh\necho {round}\n"));
            let output = Command::new(&stub)
                .output()
                .unwrap_or_else(|error| panic!("round {round}: {error}"));
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                round.to_string()
            );
        }
        stop.store(true, Ordering::Relaxed);
        for thread in forking {
            thread.join().unwrap();
        }
    }
}

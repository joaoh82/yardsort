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

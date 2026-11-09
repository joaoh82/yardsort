//! Starting work for a device, and keeping the files it sends: the host's side of
//! [`remote_proto::start`].
//!
//! A device names a harness by its id, a project, a workspace or a conversation by theirs, and a
//! first message.
//! Everything else is this host's own: the harness runs as its settings say, a new workspace is
//! made from the project's default branch and named after the message, exactly as the window's
//! composer and `ys workspace new` do. What a device sends to upload goes in this profile's
//! `uploads` folder, outside every worktree, so no branch picks it up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pty_host::{SessionInfo, TermSize, TerminalHost};
use remote_proto::{clip, HarnessSummary, MAX_LABEL, MAX_TEXT};

use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::harness::{self, HarnessDef};
use crate::launch::{HarnessRequest, Launch, Launcher};
use crate::sessions::Continue;
use crate::settings::Settings;
use crate::store::Store;
use crate::workspaces::Workspaces;

/// What an agent a device started is told its terminal is, until someone opens it: as wide as
/// one started by `ys`, which is drawn to fit once anybody watches.
pub const STARTED_SIZE: TermSize = TermSize {
    cols: 120,
    rows: 30,
};

/// How long an uploaded file is kept. An agent reads it in the turn it was given in.
pub const UPLOADS_KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What a device's channel asks of the host to start work and keep files. The app answers it
/// with a [`Starter`] made from its own state, as do the tests.
pub trait Starting: Send + Sync {
    /// The harnesses a device may start, the default first.
    fn harnesses(&self) -> Vec<HarnessSummary>;
    /// A new workspace in `project`, with `harness` given `prompt`: its id, and the agent.
    fn workspace(
        &self,
        project: &str,
        prompt: &str,
        harness: &str,
    ) -> IpcResult<(String, SessionInfo)>;
    /// Another agent in `workspace`.
    fn session(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
    ) -> IpcResult<SessionInfo>;
    /// The conversation recorded as `session` again, under its record, as the window's
    /// **Resume** starts it.
    fn resume(&self, session: &str, prompt: Option<&str>) -> IpcResult<SessionInfo>;
    /// The harnesses whose program is found, as configured: what says which conversations
    /// [`Self::resume`] could start again.
    fn definitions(&self) -> Vec<HarnessDef>;
    /// This profile's data folder: uploads go in its `uploads` folder.
    fn data_dir(&self) -> PathBuf;
}

/// Everything starting takes, borrowed for one start.
pub struct Starter<'a> {
    pub store: &'a Store,
    pub git: &'a Git,
    pub terminals: &'a dyn TerminalHost,
    pub env: &'a Arc<ShellEnv>,
    pub settings: &'a Settings,
    pub worktree_root: &'a Path,
    pub data_dir: &'a Path,
}

impl Starter<'_> {
    /// The enabled harnesses whose program is found, in the settings' order: the first is the
    /// one a device gets when it names none, as `ys workspace new` does.
    pub fn harnesses(&self) -> Vec<HarnessSummary> {
        let cwd = std::env::current_dir().unwrap_or_default();
        harness::resolve_all(&self.settings.harnesses)
            .into_iter()
            .filter(|h| h.def.enabled)
            .filter(|h| self.env.find_program(&h.def.command, &cwd).is_some())
            .filter(|h| h.def.id.chars().count() <= MAX_LABEL)
            .map(|h| HarnessSummary {
                id: h.def.id,
                name: clip(&h.def.label, MAX_TEXT),
            })
            .collect()
    }

    pub fn workspace(
        &self,
        project: &str,
        prompt: &str,
        harness: &str,
    ) -> IpcResult<(String, SessionInfo)> {
        let row = Workspaces {
            env: self.env,
            store: self.store,
            git: self.git,
            worktree_root: self.worktree_root,
            settings: &self.settings.workspaces,
        }
        .create(project, None, prompt)?;
        let session = self.session(&row.id, harness, Some(prompt))?;
        Ok((row.id, session))
    }

    pub fn definitions(&self) -> Vec<HarnessDef> {
        definitions(&self.settings.harnesses, self.env)
    }

    pub fn resume(&self, session: &str, prompt: Option<&str>) -> IpcResult<SessionInfo> {
        self.launcher().continue_session(
            session,
            Continue::Resume,
            STARTED_SIZE,
            prompt.filter(|p| !p.trim().is_empty()).map(str::to_owned),
        )
    }

    fn launcher(&self) -> Launcher<'_> {
        Launcher {
            store: self.store,
            host: self.terminals,
            env: self.env,
            harnesses: &self.settings.harnesses,
            activity: &self.settings.activity,
            launched_by: crate::activity::LaunchedBy::App,
            data_dir: self.data_dir,
        }
    }

    pub fn session(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
    ) -> IpcResult<SessionInfo> {
        self.launcher().in_workspace(
            workspace,
            Launch::Harness(HarnessRequest {
                id: harness.to_owned(),
                model: None,
                effort: None,
                prompt: prompt.filter(|p| !p.trim().is_empty()).map(str::to_owned),
                handoff: false,
                skip_memory: false,
                quiet_memory: false,
                skip_context: false,
                fork_packet: None,
            }),
            STARTED_SIZE,
        )
    }
}

/// The configured harnesses whose program is found, turned on or not: [`Starting::definitions`].
pub fn definitions(harnesses: &[harness::HarnessOverride], env: &ShellEnv) -> Vec<HarnessDef> {
    let cwd = std::env::current_dir().unwrap_or_default();
    harness::resolve_all(harnesses)
        .into_iter()
        .map(|h| h.def)
        .filter(|def| env.find_program(&def.command, &cwd).is_some())
        .collect()
}

/// The folder a workspace's uploads go in.
pub fn uploads_dir(data_dir: &Path, workspace: &str) -> PathBuf {
    data_dir.join("uploads").join(workspace)
}

/// A new file for an upload in `dir`, named after `name` and when it came: `<millis>-<stem>.<ext>`,
/// with only letters, digits, `-` and `_` kept of the name. Files older than [`UPLOADS_KEPT`] go
/// first.
pub fn new_upload(dir: &Path, name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    std::fs::create_dir_all(dir)?;
    forget_old(dir);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem, ext),
        None => (name, ""),
    };
    let keep = |text: &str, most: usize| -> String {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(most)
            .collect()
    };
    let stem = Some(keep(stem, 40)).filter(|s| !s.is_empty());
    let ext = keep(ext, 8).to_ascii_lowercase();
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let mut base = format!("{millis}-{}", stem.as_deref().unwrap_or("upload"));
    for n in 1.. {
        if n > 1 {
            base = format!("{millis}-{}-{n}", stem.as_deref().unwrap_or("upload"));
        }
        let path = dir.join(if ext.is_empty() {
            base.clone()
        } else {
            format!("{base}.{ext}")
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100 => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("the loop returns")
}

fn forget_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > UPLOADS_KEPT);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Why a start failed, as a device is told it. The host's own messages can name folders, so
/// only those known not to are passed on.
pub fn start_error(error: &IpcError) -> remote_proto::RemoteError {
    use remote_proto::{ErrorCode, RemoteError};
    match error.code.as_str() {
        // What the window says when a conversation cannot be resumed, which names a harness
        // at most.
        "no_base_branch" | "unknown_workspace" | "cannot_continue" => {
            RemoteError::new(ErrorCode::InvalidRequest, error.message.clone())
        }
        "unknown_session" => RemoteError::not_found(),
        "host_unreachable" => RemoteError::new(
            ErrorCode::Internal,
            "The computer could not tell whether that conversation still runs, so it was not \
             resumed. Try again.",
        ),
        "unknown_harness" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That agent is not one this computer can start.",
        ),
        "workspace_missing" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That workspace's folder is gone from the computer.",
        ),
        "workspace_setup_failed" => RemoteError::new(
            ErrorCode::Internal,
            "The workspace was made, but its setup failed, so no agent was started. Open it on \
             the computer to see why.",
        ),
        _ => RemoteError::new(
            ErrorCode::Internal,
            "The computer could not start that. Try again, or start it there.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_upload_keeps_only_a_safe_name_and_never_replaces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = new_upload(dir.path(), "../../etc/My Photo!.JPG").unwrap();
        let name = first.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(name.ends_with("-etcMyPhoto.jpg"), "{name}");
        assert_eq!(first.parent().unwrap(), dir.path());
        let (second, _) = new_upload(dir.path(), "../../etc/My Photo!.JPG").unwrap();
        assert_ne!(first, second);
        let (bare, _) = new_upload(dir.path(), "...").unwrap();
        assert!(bare
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("-upload"));
    }

    #[test]
    fn old_uploads_are_forgotten_when_another_comes() {
        let dir = tempfile::tempdir().unwrap();
        let (old, file) = new_upload(dir.path(), "old.png").unwrap();
        file.set_modified(SystemTime::now() - UPLOADS_KEPT - Duration::from_secs(60))
            .unwrap();
        drop(file);
        let (new, _) = new_upload(dir.path(), "new.png").unwrap();
        assert!(!old.exists());
        assert!(new.exists());
    }
}

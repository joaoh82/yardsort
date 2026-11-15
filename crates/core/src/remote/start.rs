//! Starting work for a device, and keeping the files it sends: the host's side of
//! [`remote_proto::start`].
//!
//! A device names a harness by its id, a project, a workspace or a conversation by theirs, and a
//! first message.
//! Everything else is this host's own: the harness runs as its settings say, a new workspace is
//! made from the project's default branch and named after the message, exactly as the window's
//! composer and `ys workspace new` do. What a device sends to upload goes in this profile's
//! `uploads` folder, outside every worktree, so no branch picks it up.
//!
//! Since the protocol's minor 12 a start may say what the composer says (a name, a model, an
//! effort, a branch to start from or to open), and a device may look after what is here: list a
//! project's branches, clone a repository into this host's projects folder, start a shell, and
//! rename, archive or delete a workspace ([`remote_proto::manage`]). Each is the same call the
//! window makes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pty_host::{SessionInfo, TermSize, TerminalHost};
use remote_proto::{clip, HarnessSummary, ProjectBranches, MAX_CHOICES, MAX_LABEL, MAX_TEXT};

use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::harness::{self, HarnessDef};
use crate::launch::{HarnessRequest, Launch, Launcher};
use crate::projects::Projects;
use crate::sessions::Continue;
use crate::settings::Settings;
use crate::store::Store;
use crate::workspaces::{naming, Workspaces};

/// What an agent a device started is told its terminal is, until someone opens it: as wide as
/// one started by `ys`, which is drawn to fit once anybody watches.
pub const STARTED_SIZE: TermSize = TermSize {
    cols: 120,
    rows: 30,
};

/// How long an uploaded file is kept. An agent reads it in the turn it was given in.
pub const UPLOADS_KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A new workspace a device asked for: where, the first message, the agent, and what else the
/// composer would say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewWorkspace {
    pub project: String,
    pub prompt: String,
    pub harness: String,
    /// What to call it. Made a slug, and numbered when taken; without one, named after `prompt`.
    pub name: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The branch the new one starts from. Without it, the project's default.
    pub base_branch: Option<String>,
    /// A branch to open as it is, with no new one made.
    pub existing_branch: Option<String>,
}

/// What a device's channel asks of the host to start work and keep files. The app answers it
/// with a [`Starter`] made from its own state, as do the tests.
pub trait Starting: Send + Sync {
    /// The harnesses a device may start, the default first.
    fn harnesses(&self) -> Vec<HarnessSummary>;
    /// A new workspace with its agent: the workspace's id, and the agent.
    fn workspace(&self, new: &NewWorkspace) -> IpcResult<(String, SessionInfo)>;
    /// This host's shell in `workspace`.
    fn shell(&self, workspace: &str) -> IpcResult<SessionInfo>;
    /// The branches a new workspace in `project` can start from or open.
    fn branches(&self, project: &str) -> IpcResult<ProjectBranches>;
    /// Clone the repository at `remote` into this host's projects folder and add it: the new
    /// project's id and name.
    fn clone_project(&self, remote: &str, name: Option<&str>) -> IpcResult<(String, String)>;
    /// Change what `workspace` is called.
    fn rename(&self, workspace: &str, name: &str) -> IpcResult<()>;
    /// Put `workspace` away, keeping its branch and its record. Without `force`, uncommitted
    /// work fails it with `worktree_dirty`.
    fn archive(&self, workspace: &str, force: bool) -> IpcResult<()>;
    /// Remove `workspace`, keeping its branch. Without `force`, uncommitted work fails it with
    /// `worktree_dirty`.
    fn delete(&self, workspace: &str, force: bool) -> IpcResult<()>;
    /// What changed in `workspace`: uncommitted, and committed on its branch.
    fn changes(&self, workspace: &str) -> IpcResult<crate::changes::ChangeSet>;
    /// One folder of `workspace`'s file tree, without `.git` and what Git ignores. `dir` is
    /// inside the workspace; empty for its root.
    fn files(&self, workspace: &str, dir: &str) -> IpcResult<Vec<crate::file_tree::FileEntry>>;
    /// A file of `workspace`, on disk or on one side of a change. `path` is inside the
    /// workspace, and nothing outside it is read.
    fn content(
        &self,
        workspace: &str,
        path: &str,
        old_path: Option<&str>,
        of: remote_proto::FileOf,
    ) -> IpcResult<crate::changes::Content>;
    /// Where `workspace` stands with its remote; with `forge`, its pull request too.
    fn publish(
        &self,
        workspace: &str,
        forge: bool,
    ) -> IpcResult<remote_proto::publish::PublishState>;
    /// Commit everything changed in `workspace`.
    fn commit(
        &self,
        workspace: &str,
        message: &str,
    ) -> IpcResult<remote_proto::publish::PublishState>;
    /// Push `workspace`'s branch to the project's remote.
    fn push(&self, workspace: &str) -> IpcResult<remote_proto::publish::PublishState>;
    /// Open a pull request for `workspace`'s branch, pushing first when needed.
    fn open_pull_request(
        &self,
        workspace: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> IpcResult<remote_proto::publish::PullRequestOpened>;
    /// Merge `workspace`'s pull request, if it is still at `head_oid`.
    fn merge_pull_request(
        &self,
        workspace: &str,
        number: u32,
        head_oid: &str,
        method: remote_proto::publish::MergeMethod,
    ) -> IpcResult<()>;
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
                models: choices(h.def.models),
                efforts: choices(h.def.efforts),
            })
            .collect()
    }

    fn workspaces(&self) -> Workspaces<'_> {
        Workspaces {
            env: self.env,
            store: self.store,
            git: self.git,
            worktree_root: self.worktree_root,
            settings: &self.settings.workspaces,
        }
    }

    pub fn workspace(&self, new: &NewWorkspace) -> IpcResult<(String, SessionInfo)> {
        let workspaces = self.workspaces();
        let base = new.base_branch.as_deref();
        if let Some(base) = base {
            let project = self.store.project(&new.project)?.ok_or_else(|| {
                IpcError::new("unknown_project", "That project no longer exists.")
            })?;
            if !self
                .git
                .branch_exists(Path::new(&project.root_path), base)?
            {
                return Err(IpcError::new(
                    "unknown_branch",
                    format!("There is no branch \"{base}\"."),
                ));
            }
        }
        let row = match (&new.existing_branch, &new.name) {
            (Some(branch), _) => workspaces.open_branch(&new.project, branch)?,
            (None, Some(name)) => match naming::slugify_name(name) {
                Some(slug) => workspaces.create_named(&new.project, base, &slug)?,
                // A name with nothing a folder can be called by: the message names it instead.
                None => workspaces.create(&new.project, base, &new.prompt)?,
            },
            (None, None) => workspaces.create(&new.project, base, &new.prompt)?,
        };
        let session = self.launch(
            &row.id,
            &new.harness,
            Some(&new.prompt),
            new.model.as_deref(),
            new.effort.as_deref(),
        )?;
        Ok((row.id, session))
    }

    pub fn shell(&self, workspace: &str) -> IpcResult<SessionInfo> {
        self.launcher()
            .in_workspace(workspace, Launch::Shell, STARTED_SIZE)
    }

    pub fn branches(&self, project: &str) -> IpcResult<ProjectBranches> {
        let row = self
            .store
            .project(project)?
            .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
        let root = PathBuf::from(row.root_path);
        Ok(remote_proto::manage::bounded_branches(
            self.git.branches(&root)?,
            self.git.default_branch(&root)?,
            self.git
                .worktrees(&root)?
                .into_iter()
                .filter(|entry| !entry.prunable)
                .filter_map(|entry| entry.branch)
                .collect(),
        ))
    }

    /// The folder a device's clones go in: the setting, or `Projects` in this user's home. Made
    /// when it is not there yet.
    pub fn projects_dir(&self) -> IpcResult<PathBuf> {
        let dir = projects_dir(&self.settings.remote, self.env).ok_or_else(|| {
            IpcError::new(
                "no_projects_dir",
                "This computer has no folder for projects cloned from another device. Set one \
                 in Settings → Remote.",
            )
        })?;
        std::fs::create_dir_all(&dir).map_err(|error| {
            IpcError::new("io", format!("Cannot create {}: {error}", dir.display()))
        })?;
        Ok(dir)
    }

    pub fn clone_project(&self, remote: &str, name: Option<&str>) -> IpcResult<(String, String)> {
        let name = match name {
            Some(name) => name.trim().to_owned(),
            None => name_from_remote(remote).ok_or_else(|| {
                IpcError::new(
                    "invalid_repository",
                    "That address does not say what the repository is called.",
                )
            })?,
        };
        let parent = self.projects_dir()?;
        let added = Projects {
            store: self.store,
            git: self.git,
        }
        .clone_from(remote, &name, &parent, None)?;
        Ok((added.project.id, added.project.name))
    }

    pub fn rename(&self, workspace: &str, name: &str) -> IpcResult<()> {
        self.workspaces().rename(workspace, name).map(|_| ())
    }

    pub fn archive(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.workspaces().archive(workspace, force)
    }

    pub fn delete(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.workspaces().delete(workspace, force)
    }

    /// A workspace's folder, when it is there.
    pub(crate) fn folder(
        &self,
        workspace: &str,
    ) -> IpcResult<(crate::store::WorkspaceRow, PathBuf)> {
        let row = self.store.workspace(workspace)?.ok_or_else(|| {
            IpcError::new("unknown_workspace", "That workspace no longer exists.")
        })?;
        let root = PathBuf::from(&row.path);
        if !root.is_dir() {
            return Err(IpcError::new(
                "workspace_missing",
                "That workspace's folder is gone.",
            ));
        }
        Ok((row, root))
    }

    pub fn changes(&self, workspace: &str) -> IpcResult<crate::changes::ChangeSet> {
        let (row, root) = self.folder(workspace)?;
        crate::changes::Changes {
            git: self.git,
            root: &root,
            base_branch: row.base_branch.as_deref(),
        }
        .list()
    }

    pub fn files(&self, workspace: &str, dir: &str) -> IpcResult<Vec<crate::file_tree::FileEntry>> {
        let (_, root) = self.folder(workspace)?;
        crate::file_tree::list_dir(&root, dir, false)
    }

    pub fn content(
        &self,
        workspace: &str,
        path: &str,
        old_path: Option<&str>,
        of: remote_proto::FileOf,
    ) -> IpcResult<crate::changes::Content> {
        use crate::changes::Scope;
        use remote_proto::FileOf;
        let (row, root) = self.folder(workspace)?;
        if of == FileOf::Working {
            return crate::changes::read_working_file(&root, path);
        }
        let scope = if of == FileOf::UncommittedOld {
            Scope::Uncommitted
        } else {
            Scope::Committed
        };
        let diff = crate::changes::Changes {
            git: self.git,
            root: &root,
            base_branch: row.base_branch.as_deref(),
        }
        .diff(path, old_path, scope)?;
        Ok(if of == FileOf::CommittedNew {
            diff.new
        } else {
            diff.old
        })
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
        self.launch(workspace, harness, prompt, None, None)
    }

    fn launch(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> IpcResult<SessionInfo> {
        let said = |text: Option<&str>| {
            text.map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
        };
        self.launcher().in_workspace(
            workspace,
            Launch::Harness(HarnessRequest {
                id: harness.to_owned(),
                model: said(model),
                effort: said(effort),
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

/// A harness's models or efforts as one reply lists them: those that fit, and not too many.
fn choices(listed: Vec<String>) -> Vec<String> {
    listed
        .into_iter()
        .filter(|choice| !choice.is_empty() && choice.chars().count() <= MAX_LABEL)
        .take(MAX_CHOICES)
        .collect()
}

/// Where a device's clones go on this host: `remote.projects_dir` in the settings, with a leading
/// `~` read as this user's home, or else `Projects` in that home. `None` when there is no home
/// to put it in and none was set.
pub fn projects_dir(settings: &crate::settings::RemoteSettings, env: &ShellEnv) -> Option<PathBuf> {
    let home = env.home_dir();
    match settings.projects_dir.as_deref().map(str::trim) {
        Some(set) if !set.is_empty() => match set.strip_prefix('~') {
            Some(rest) => {
                let rest = rest.trim_start_matches(['/', '\\']);
                home.map(|home| {
                    if rest.is_empty() {
                        home
                    } else {
                        home.join(rest)
                    }
                })
            }
            None => Some(PathBuf::from(set)),
        },
        _ => home.map(|home| home.join("Projects")),
    }
}

/// What a clone of `remote` is called when nobody says: the last part of its address, without
/// `.git`.
pub fn name_from_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let rest = remote.split_once("://").map_or(remote, |(_, rest)| rest);
    // What comes after the host: `host/path`, or `user@host:path`.
    let path = rest
        .split_once('/')
        .or_else(|| rest.split_once(':'))
        .map(|(_, path)| path)?;
    let last = path.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
}

/// A project's fetch address as another computer may be told it: one it could clone, with any
/// user and password of an HTTPS address removed. A local path, a `file://` or `git://` address
/// and anything longer than a reply carries is not told.
pub fn shareable_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let cleaned = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => {
            let (authority, path) = rest.split_once('/')?;
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            format!("https://{host}/{path}")
        }
        _ => url.to_owned(),
    };
    (cleaned.len() <= remote_proto::MAX_REMOTE && crate::projects::clonable(&cleaned))
        .then_some(cleaned)
}

/// The repository an address names, for telling whether two computers' projects are the same
/// one: its host in lower case and its path, without the scheme, the user, the port and `.git`.
/// `https://github.com/acme/app.git` and `git@github.com:acme/app` are both `github.com/acme/app`.
pub fn repository_key(url: &str) -> Option<String> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _)| host);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    (!host.is_empty() && !path.is_empty()).then(|| format!("{}/{path}", host.to_ascii_lowercase()))
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
        "no_base_branch" | "unknown_workspace" | "cannot_continue" | "invalid_name"
        | "not_deletable" | "not_archivable" | "invalid_repository" | "no_projects_dir" => {
            RemoteError::new(ErrorCode::InvalidRequest, error.message.clone())
        }
        "unknown_branch" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That branch is not in the project on the computer.",
        ),
        "unknown_project" => RemoteError::not_found(),
        // Written by this host for a device, with nothing of Git's or the forge's own words.
        code if super::publish::REFUSALS.contains(&code) => {
            RemoteError::new(ErrorCode::InvalidRequest, error.message.clone())
        }
        "bad_path" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That is not a path inside the workspace.",
        ),
        "not_a_directory" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That is not a folder in the workspace.",
        ),
        "worktree_dirty" => RemoteError::new(
            ErrorCode::UncommittedWork,
            "This workspace has uncommitted changes, which would be lost.",
        ),
        "already_exists" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "The computer's projects folder already has a folder of that name. Add it as a \
             project there, or give the clone another name.",
        ),
        "clone_failed" => RemoteError::new(
            ErrorCode::InvalidRequest,
            "The computer could not clone that repository. Check that it can reach it with its \
             own Git credentials.",
        ),
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

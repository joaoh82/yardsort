//! Projects and their workspaces: the policy that sits between the store, git and the disk.

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::git::{normalize, Git, Head};
use crate::store::{ProjectRow, Store, WorkspaceRow};
use crate::tasks::delegate::TaskRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    /// The folder is gone (deleted, moved, or on an unmounted drive). The project is kept so it
    /// reappears by itself if the folder does; the user can remove it.
    pub missing: bool,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceKind {
    /// The project's own checkout. Always present, never deletable.
    Local,
    /// A git worktree on its own branch.
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub kind: WorkspaceKind,
    pub name: String,
    pub path: String,
    /// What is checked out right now, asked of git at listing time.
    pub head: Option<HeadInfo>,
    /// The folder is gone though it should be there. It can be restored from its branch, or
    /// deleted.
    pub missing: bool,
    /// Put away on purpose: no folder, but the branch and session history are kept.
    pub archived: bool,
    /// There is no branch left to bring this workspace back from: its folder is not on disk and
    /// its branch was deleted too (or it never had one). Only asked of git when the folder is
    /// already gone, and a git that will not answer counts as "the branch is still there".
    pub branch_gone: bool,
    /// The tasks it was started from, oldest first. Nearly always none or one.
    pub tasks: Vec<TaskRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HeadInfo {
    /// Branch name, or the abbreviated commit when detached.
    pub label: String,
    pub detached: bool,
    /// The branch has no commits yet.
    pub unborn: bool,
}

impl From<Head> for HeadInfo {
    fn from(head: Head) -> Self {
        let (label, detached, unborn) = match head {
            Head::Branch(name) => (name, false, false),
            Head::Unborn(name) => (name, false, true),
            Head::Detached(sha) => (sha, true, false),
        };
        Self {
            label,
            detached,
            unborn,
        }
    }
}

/// The result of adding a project: it may have been known already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddedProject {
    pub project: Project,
    pub already_known: bool,
    /// Set when the chosen folder was inside a repository and its root was added instead.
    pub opened_root_instead: bool,
    /// Set when the folder was a project removed earlier with its history kept: it is back
    /// with its workspaces and their conversations.
    pub revived: bool,
}

pub struct Projects<'a> {
    pub store: &'a Store,
    pub git: &'a Git,
}

impl Projects<'_> {
    pub fn list(&self) -> IpcResult<Vec<Project>> {
        let mut workspaces = self.store.workspaces()?;
        self.store
            .projects()?
            .into_iter()
            .map(|row| {
                let (mine, rest) = workspaces.drain(..).partition(|w| w.project_id == row.id);
                workspaces = rest;
                Ok(self.describe(row, mine))
            })
            .collect()
    }

    /// Add the repository containing `path`. With `init_git`, a folder that is not a repository
    /// is turned into one first; without it that case is the `not_a_git_repo` error, so the UI
    /// can ask.
    pub fn open(&self, path: &Path, init_git: bool) -> IpcResult<AddedProject> {
        if !path.is_dir() {
            return Err(IpcError::new(
                "not_a_directory",
                format!("{} is not a folder.", path.display()),
            ));
        }
        let chosen = normalize(path);
        let root = match self.git.repo_root(&chosen)? {
            Some(root) => root,
            None if init_git => {
                self.git.init(&chosen)?;
                self.git.initial_commit(&chosen)?;
                chosen.clone()
            }
            None => {
                return Err(IpcError::new(
                    "not_a_git_repo",
                    format!("{} is not a git repository.", chosen.display()),
                ))
            }
        };
        self.add(&root, root != chosen)
    }

    /// Create `<parent>/<name>` as a new repository with an initial commit, and add it.
    pub fn create(&self, name: &str, parent: &Path) -> IpcResult<AddedProject> {
        let name = validate_name(name)?;
        if !parent.is_dir() {
            return Err(IpcError::new(
                "not_a_directory",
                format!("{} is not a folder.", parent.display()),
            ));
        }
        let target = parent.join(name);
        if target.exists() {
            return Err(IpcError::new(
                "already_exists",
                format!("{} already exists.", target.display()),
            ));
        }

        std::fs::create_dir(&target)
            .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", target.display())))?;
        let prepared = self
            .git
            .init(&target)
            .and_then(|()| self.git.initial_commit(&target));
        if let Err(error) = prepared {
            // We made this folder a moment ago and nothing but git has touched it.
            let _ = std::fs::remove_dir_all(&target);
            return Err(error.into());
        }
        self.add(&normalize(&target), false)
    }

    /// Clone a repository — GitHub by `owner/repository`, any host by URL — into a new folder,
    /// then register its local workspace. With `upstream` — a fork's parent — that repository
    /// becomes the clone's `upstream` remote, which is where a pull request from the fork is
    /// measured against.
    pub fn clone_from(
        &self,
        repository: &str,
        name: &str,
        parent: &Path,
        upstream: Option<&str>,
    ) -> IpcResult<AddedProject> {
        let url = clone_url(repository)?;
        let upstream = upstream.map(clone_url).transpose()?;
        self.clone_repository(&url, name, parent, upstream.as_deref())
    }

    fn clone_repository(
        &self,
        url: &str,
        name: &str,
        parent: &Path,
        upstream: Option<&str>,
    ) -> IpcResult<AddedProject> {
        let name = validate_name(name)?;
        if !parent.is_dir() {
            return Err(IpcError::new(
                "not_a_directory",
                "Choose an existing destination folder.",
            ));
        }
        let target = normalize(parent).join(name);
        // Reserve the name atomically. Even an empty existing folder belongs to the user.
        std::fs::create_dir(&target).map_err(|error| {
            IpcError::new(
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    "already_exists"
                } else {
                    "io"
                },
                format!("Cannot create {}: {error}", target.display()),
            )
        })?;
        if let Err(error) = self
            .git
            .run(parent, &["clone", "--", url, &target.to_string_lossy()])
        {
            // Cloning can take a long time; somebody may have written here in the meantime.
            // Only remove an empty reservation, never recursively delete user work.
            let retained = std::fs::remove_dir(&target).is_err() && target.exists();
            return Err(IpcError::new("clone_failed", format!(
                "Could not clone the repository: {error}. Check the URL and your git credentials.{}",
                if retained { format!(" Files were kept at {}; choose another name or inspect that folder before retrying.", target.display()) } else { String::new() }
            )));
        }
        if let Some(upstream) = upstream {
            // The clone is good whatever happens here: the parent's branches are a convenience
            // for later, and fetching them can wait for a network that is there.
            self.git
                .run(&target, &["remote", "add", "upstream", "--", upstream])
                .map_err(|error| {
                    IpcError::new(
                        "clone_failed",
                        format!("Could not add the upstream remote: {error}"),
                    )
                })?;
            let _ = self.git.run(&target, &["fetch", "upstream"]);
        }
        self.add(&target, false)
    }

    fn add(&self, root: &Path, opened_root_instead: bool) -> IpcResult<AddedProject> {
        let root_path = root.to_string_lossy().into_owned();
        let (row, already_known, revived) = match self.store.project_by_root(&root_path)? {
            Some(existing) => (existing, true, false),
            None => match self.store.removed_project_by_root(&root_path)? {
                Some(removed) => {
                    self.store.revive_project(&removed.id)?;
                    (removed, false, true)
                }
                None => {
                    let name = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root_path.clone());
                    (self.store.add_project(&name, &root_path)?, false, false)
                }
            },
        };
        let workspaces = self
            .store
            .workspaces()?
            .into_iter()
            .filter(|w| w.project_id == row.id)
            .collect();
        Ok(AddedProject {
            project: self.describe(row, workspaces),
            already_known,
            opened_root_instead,
            revived,
        })
    }

    fn describe(&self, row: ProjectRow, workspaces: Vec<WorkspaceRow>) -> Project {
        let root = PathBuf::from(&row.root_path);
        let missing = !root.is_dir();
        // Asking git for the branches costs a process, so only pay it when a workspace has lost
        // its folder and the answer decides what the UI can still offer. One list serves them
        // all; the repository we cannot reach tells us nothing.
        let branches = (!missing && workspaces.iter().any(|w| !Path::new(&w.path).is_dir()))
            .then(|| self.git.branches(&root).ok())
            .flatten();
        Project {
            workspaces: workspaces
                .into_iter()
                .map(|w| self.workspace(w, &root, branches.as_deref()))
                .collect(),
            id: row.id,
            name: row.name,
            root_path: row.root_path,
            missing,
        }
    }

    /// Describe a single workspace, looking its project up to answer for its branch.
    pub fn describe_workspace(&self, row: WorkspaceRow) -> Workspace {
        let root = self
            .store
            .project(&row.project_id)
            .ok()
            .flatten()
            .map(|project| PathBuf::from(project.root_path))
            .unwrap_or_default();
        self.workspace(row, &root, None)
    }

    /// `branches` is the project's branch list when the caller already has it; without it we ask
    /// git ourselves, and only if we have to.
    fn workspace(&self, row: WorkspaceRow, root: &Path, branches: Option<&[String]>) -> Workspace {
        let path = PathBuf::from(&row.path);
        let on_disk = path.is_dir();
        let head = on_disk
            .then(|| self.git.head(&path).ok())
            .flatten()
            .map(HeadInfo::from);
        // A store that will not answer is a workspace with no badge, not one that cannot be listed.
        let tasks = self
            .store
            .workspace_tasks(&row.id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(TaskRef::from_row)
            .collect();
        Workspace {
            kind: if row.kind == "local" {
                WorkspaceKind::Local
            } else {
                WorkspaceKind::Worktree
            },
            missing: !row.archived && !on_disk,
            // `local` is the project's own checkout: it has no branch of its own to lose.
            branch_gone: row.kind != "local" && !on_disk && self.branch_gone(&row, root, branches),
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            path: row.path,
            tasks,
            head,
            archived: row.archived,
        }
    }

    /// Has the branch a vanished workspace would be restored from gone as well? A repository we
    /// cannot reach or a git that fails both answer "no": Yardsort never offers to forget a
    /// workspace because a command misbehaved.
    fn branch_gone(&self, row: &WorkspaceRow, root: &Path, branches: Option<&[String]>) -> bool {
        let Some(branch) = row.branch.as_deref() else {
            // An adopted detached worktree: nothing named it, so nothing can bring it back.
            return true;
        };
        match branches {
            Some(known) => !known.iter().any(|name| name == branch),
            None if root.is_dir() => !self.git.branch_exists(root, branch).unwrap_or(true),
            None => false,
        }
    }
}

/// Accept GitHub's HTTPS/SSH clone URLs or the convenient owner/repository form.
/// The URL `git clone` is given, from what the user typed or picked.
///
/// `owner/repository` and every GitHub spelling mean GitHub, normalized as before. Any other
/// host takes an HTTPS URL, an `ssh://` URL or git's `user@host:path` form, with as many path
/// segments as the forge wants (GitLab subgroups). Nothing else: no local path, no `file://`
/// or `git://`, no credentials in an HTTPS URL, nothing that could be read as an option.
fn clone_url(repository: &str) -> IpcResult<String> {
    let repository = repository.trim();
    let invalid = || {
        IpcError::new(
            "invalid_repository",
            "Enter a repository as owner/repository on GitHub, an HTTPS clone URL, or an SSH clone URL.",
        )
    };
    if let Some(url) = github_url(repository) {
        return url;
    }
    let (prefix, path) = if let Some((scheme, rest)) = repository.split_once("://") {
        // scheme://[user@]host[:port]/path
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        let (user, host_port) = match authority.split_once('@') {
            Some((user, host_port)) => (Some(user), host_port),
            None => (None, authority),
        };
        match (scheme, user) {
            ("https", None) => {}
            ("ssh", None | Some(_)) => {}
            _ => return Err(invalid()),
        }
        let (host, port) = match host_port.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        };
        if !valid_host(host)
            || !port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            || !user.is_none_or(valid_user)
        {
            return Err(invalid());
        }
        (format!("{scheme}://{authority}/"), path)
    } else if let Some((authority, path)) = repository.split_once(':') {
        // [user@]host:path — git's scp-like form. A Windows path (`C:\…`) has a one-letter host
        // and a path that is not one; both fall out below.
        let host = authority
            .split_once('@')
            .map_or(authority, |(_, host)| host);
        let user = authority.split_once('@').map(|(user, _)| user);
        if !valid_host(host) || !host.contains('.') || !user.is_none_or(valid_user) {
            return Err(invalid());
        }
        (format!("{authority}:"), path)
    } else {
        return Err(invalid());
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 2 || !parts.iter().all(|part| valid_segment(part)) {
        return Err(invalid());
    }
    Ok(format!("{prefix}{path}.git"))
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && !host.starts_with('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn valid_user(user: &str) -> bool {
    !user.is_empty()
        && !user.starts_with('-')
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn valid_segment(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.starts_with('-')
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// `Some` when `repository` is one of the GitHub spellings, with the verdict on it; `None` when
/// it is something else entirely.
fn github_url(repository: &str) -> Option<IpcResult<String>> {
    let (path, ssh) = if let Some(path) = [
        "https://github.com/",
        "https://www.github.com/",
        "http://github.com/",
        "http://www.github.com/",
        "github.com/",
        "www.github.com/",
    ]
    .iter()
    .find_map(|prefix| repository.strip_prefix(prefix))
    {
        (path, false)
    } else if let Some(path) = repository.strip_prefix("git@github.com:") {
        (path, true)
    } else if let Some(path) = repository.strip_prefix("ssh://git@github.com/") {
        (path, true)
    } else if !repository.contains("://") && !repository.contains(':') {
        // owner/repository, or something that is neither a URL nor a shorthand: GitHub's verdict
        // either way, since nothing else takes a bare path.
        (repository, false)
    } else {
        return None;
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2 || !parts.iter().all(|part| valid_segment(part)) {
        return Some(Err(IpcError::new(
            "invalid_repository",
            "Enter a repository as owner/repository on GitHub, an HTTPS clone URL, or an SSH clone URL.",
        )));
    }
    Some(Ok(if ssh {
        format!("git@github.com:{path}.git")
    } else {
        format!("https://github.com/{path}.git")
    }))
}

/// A project name becomes a folder name, so it has to be one on every OS we run on.
fn validate_name(name: &str) -> IpcResult<&str> {
    let name = name.trim();
    let invalid = |why: &str| {
        Err(IpcError::new(
            "invalid_name",
            format!("Project name {why}."),
        ))
    };
    if name.is_empty() {
        return invalid("cannot be empty");
    }
    if name == "." || name == ".." || name.ends_with('.') {
        return invalid("cannot be or end with a dot");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| c.is_control() || r#"/\:*?"<>|"#.contains(*c))
    {
        return invalid(&format!("cannot contain {bad:?}"));
    }
    if name.len() > 100 {
        return invalid("is too long");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::git;

    struct Fixture {
        store: Store,
        git: Git,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                store: Store::in_memory(),
                git: git(),
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn projects(&self) -> Projects<'_> {
            Projects {
                store: &self.store,
                git: &self.git,
            }
        }

        fn path(&self) -> PathBuf {
            normalize(self.dir.path())
        }
    }

    #[test]
    fn github_repository_forms_are_normalized_and_other_inputs_refused() {
        for input in [
            "owner/repo",
            " https://github.com/owner/repo.git/ ",
            "github.com/owner/repo",
            "www.github.com/owner/repo.git",
            "https://www.github.com/owner/repo",
            "http://github.com/owner/repo",
            "http://www.github.com/owner/repo.git/",
        ] {
            assert_eq!(
                clone_url(input).unwrap(),
                "https://github.com/owner/repo.git",
                "{input}"
            );
        }
        for input in [
            "git@github.com:owner/repo.git",
            "ssh://git@github.com/owner/repo",
        ] {
            assert_eq!(clone_url(input).unwrap(), "git@github.com:owner/repo.git");
        }
        for input in [
            "",
            "--upload-pack=bad",
            "a/../b",
            "a/b/tree/main",
            "a/b?token=secret",
            "a/b c",
            "file:///tmp/repo",
            "github.com.evil/a/b",
            "www.github.com.evil/a/b",
            "https://token@github.com/a/b",
            "github.com/a/b?token=secret",
        ] {
            assert_eq!(
                clone_url(input).unwrap_err().code,
                "invalid_repository",
                "{input}"
            );
        }
    }

    #[test]
    fn any_host_is_cloned_by_url_but_never_by_path_or_option() {
        for (input, url) in [
            (
                "https://gitlab.com/group/subgroup/repo",
                "https://gitlab.com/group/subgroup/repo.git",
            ),
            (
                "https://gitlab.com/group/repo.git/",
                "https://gitlab.com/group/repo.git",
            ),
            (
                "https://forge.example.org:3000/owner/repo",
                "https://forge.example.org:3000/owner/repo.git",
            ),
            (
                "ssh://git@gitlab.com/group/sub/repo.git",
                "ssh://git@gitlab.com/group/sub/repo.git",
            ),
            (
                "ssh://forgejo@codeberg.org:2222/owner/repo",
                "ssh://forgejo@codeberg.org:2222/owner/repo.git",
            ),
            (
                "git@gitlab.com:group/sub/repo.git",
                "git@gitlab.com:group/sub/repo.git",
            ),
            ("codeberg.org:owner/repo", "codeberg.org:owner/repo.git"),
            // These used to be refused as not GitHub; they are ordinary clone URLs.
            (
                "https://elsewhere.test/a/b",
                "https://elsewhere.test/a/b.git",
            ),
            (
                "https://github.com.evil/a/b",
                "https://github.com.evil/a/b.git",
            ),
        ] {
            assert_eq!(clone_url(input).unwrap(), url, "{input}");
        }
        for input in [
            "gitlab.com/group/repo",
            "/tmp/repo",
            "C:\\Users\\me\\repo",
            "git://gitlab.com/group/repo",
            "http://gitlab.com/group/repo",
            "https://user:token@gitlab.com/group/repo",
            "https://gitlab.com/repo",
            "https://gitlab.com/group/-repo",
            "https://gitlab.com/group/repo?x=1",
            "https://-bad.host/a/b",
            "ssh://git@gitlab.com:port/a/b",
            "-user@gitlab.com:a/b",
            "git@localhost:a/b",
            "https://gitlab.com/a/b c",
        ] {
            assert_eq!(
                clone_url(input).unwrap_err().code,
                "invalid_repository",
                "{input}"
            );
        }
    }

    #[test]
    fn cloning_preserves_history_origin_and_registers_the_local_workspace() {
        let fx = Fixture::new();
        let source = fx.path().join("source");
        std::fs::create_dir(&source).unwrap();
        fx.git.init(&source).unwrap();
        fx.git.initial_commit(&source).unwrap();
        let added = fx
            .projects()
            .clone_repository(&source.to_string_lossy(), "my clone", &fx.path(), None)
            .unwrap();
        let target = PathBuf::from(&added.project.root_path);
        assert_eq!(
            fx.git.run(&target, &["rev-parse", "HEAD"]).unwrap(),
            fx.git.run(&source, &["rev-parse", "HEAD"]).unwrap()
        );
        assert_eq!(
            fx.git
                .run(&target, &["remote", "get-url", "origin"])
                .unwrap(),
            source.to_string_lossy()
        );
        assert_eq!(added.project.workspaces[0].kind, WorkspaceKind::Local);
        assert_eq!(fx.projects().list().unwrap().len(), 1);
        let error = fx
            .projects()
            .clone_repository(&source.to_string_lossy(), "my clone", &fx.path(), None)
            .unwrap_err();
        assert_eq!(error.code, "already_exists");
        assert!(fx.git.has_commits(&target).unwrap());
    }

    #[test]
    fn a_fork_gets_its_parent_as_upstream_and_its_branches_fetched() {
        let fx = Fixture::new();
        let parent = fx.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        fx.git.init(&parent).unwrap();
        fx.git.initial_commit(&parent).unwrap();
        fx.git
            .run(&parent, &["branch", "only-upstream-has-this"])
            .unwrap();
        let fork = fx.path().join("fork");
        fx.git
            .run(
                &fx.path(),
                &[
                    "clone",
                    "--",
                    &parent.to_string_lossy(),
                    &fork.to_string_lossy(),
                ],
            )
            .unwrap();
        let added = fx
            .projects()
            .clone_repository(
                &fork.to_string_lossy(),
                "mine",
                &fx.path(),
                Some(&parent.to_string_lossy()),
            )
            .unwrap();
        let target = PathBuf::from(&added.project.root_path);
        assert_eq!(
            fx.git
                .run(&target, &["remote", "get-url", "origin"])
                .unwrap(),
            fork.to_string_lossy()
        );
        assert_eq!(
            fx.git
                .run(&target, &["remote", "get-url", "upstream"])
                .unwrap(),
            parent.to_string_lossy()
        );
        assert!(fx
            .git
            .run(
                &target,
                &["rev-parse", "--verify", "upstream/only-upstream-has-this"]
            )
            .is_ok());
        // Not a fork, nothing added.
        let plain = fx
            .projects()
            .clone_repository(&parent.to_string_lossy(), "plain", &fx.path(), None)
            .unwrap();
        assert_eq!(
            fx.git
                .remotes(&PathBuf::from(&plain.project.root_path))
                .unwrap(),
            ["origin"]
        );
    }

    #[test]
    fn failed_clone_does_not_register_a_project_or_touch_an_existing_folder() {
        let fx = Fixture::new();
        let source = fx.path().join("missing");
        assert_eq!(
            fx.projects()
                .clone_repository(&source.to_string_lossy(), "failed", &fx.path(), None)
                .unwrap_err()
                .code,
            "clone_failed"
        );
        assert!(fx.projects().list().unwrap().is_empty());
        assert!(!fx.path().join("failed").exists());
        std::fs::create_dir(fx.path().join("taken")).unwrap();
        std::fs::write(fx.path().join("taken/keep"), "user work").unwrap();
        assert_eq!(
            fx.projects()
                .clone_repository(&source.to_string_lossy(), "taken", &fx.path(), None)
                .unwrap_err()
                .code,
            "already_exists"
        );
        assert_eq!(
            std::fs::read_to_string(fx.path().join("taken/keep")).unwrap(),
            "user work"
        );
        assert_eq!(
            fx.projects()
                .clone_repository(&source.to_string_lossy(), "../escape", &fx.path(), None)
                .unwrap_err()
                .code,
            "invalid_name"
        );
    }

    #[test]
    fn creating_a_project_makes_a_repository_with_a_first_commit() {
        let fx = Fixture::new();
        let added = fx.projects().create("  my-app ", &fx.path()).unwrap();

        let root = fx.path().join("my-app");
        assert_eq!(added.project.name, "my-app");
        assert_eq!(PathBuf::from(&added.project.root_path), root);
        assert!(root.join(".git").exists());
        assert!(
            fx.git.has_commits(&root).unwrap(),
            "worktrees need a commit to branch from"
        );

        let local = &added.project.workspaces[0];
        assert_eq!(local.kind, WorkspaceKind::Local);
        assert_eq!(local.path, added.project.root_path);
        let head = local.head.as_ref().unwrap();
        assert!(!head.detached && !head.unborn && !head.label.is_empty());
    }

    #[test]
    fn creating_over_an_existing_folder_is_refused_and_leaves_it_alone() {
        let fx = Fixture::new();
        let existing = fx.path().join("taken");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("keep.txt"), "precious").unwrap();

        let err = fx.projects().create("taken", &fx.path()).unwrap_err();

        assert_eq!(err.code, "already_exists");
        assert_eq!(
            std::fs::read_to_string(existing.join("keep.txt")).unwrap(),
            "precious"
        );
        assert!(fx.projects().list().unwrap().is_empty());
    }

    #[test]
    fn names_that_cannot_be_folders_are_rejected() {
        let fx = Fixture::new();
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "what?",
            "trailing.",
            "co:lon",
        ] {
            let err = fx.projects().create(bad, &fx.path()).unwrap_err();
            assert_eq!(err.code, "invalid_name", "{bad:?}");
        }
        assert!(
            std::fs::read_dir(fx.path()).unwrap().next().is_none(),
            "nothing was created"
        );
    }

    #[test]
    fn opening_a_plain_folder_asks_before_initialising() {
        let fx = Fixture::new();
        let err = fx.projects().open(&fx.path(), false).unwrap_err();
        assert_eq!(err.code, "not_a_git_repo");
        assert!(
            !fx.path().join(".git").exists(),
            "nothing happens without consent"
        );

        let added = fx.projects().open(&fx.path(), true).unwrap();
        assert!(fx.path().join(".git").exists());
        assert!(fx.git.has_commits(&fx.path()).unwrap());
        assert!(!added.already_known);
    }

    #[test]
    fn opening_a_subfolder_adds_the_repository_root() {
        let fx = Fixture::new();
        fx.git.init(&fx.path()).unwrap();
        let nested = fx.path().join("packages").join("web");
        std::fs::create_dir_all(&nested).unwrap();

        let added = fx.projects().open(&nested, false).unwrap();

        assert!(added.opened_root_instead);
        assert_eq!(PathBuf::from(&added.project.root_path), fx.path());
    }

    #[test]
    fn opening_the_same_repository_twice_returns_the_known_project() {
        let fx = Fixture::new();
        fx.git.init(&fx.path()).unwrap();
        let first = fx.projects().open(&fx.path(), false).unwrap();
        let second = fx.projects().open(&fx.path(), false).unwrap();
        assert!(second.already_known);
        assert_eq!(second.project.id, first.project.id);
        assert_eq!(fx.projects().list().unwrap().len(), 1);
    }

    #[test]
    fn a_fresh_repository_reports_an_unborn_branch() {
        let fx = Fixture::new();
        fx.git.init(&fx.path()).unwrap();
        let added = fx.projects().open(&fx.path(), false).unwrap();
        assert!(added.project.workspaces[0].head.as_ref().unwrap().unborn);
    }

    #[test]
    fn a_vanished_folder_is_flagged_not_forgotten_and_removal_never_touches_disk() {
        let fx = Fixture::new();
        let added = fx.projects().create("app", &fx.path()).unwrap();
        let root = PathBuf::from(&added.project.root_path);

        let moved = fx.path().join("app-moved");
        std::fs::rename(&root, &moved).unwrap();
        let listed = fx.projects().list().unwrap();
        assert!(listed[0].missing);
        assert_eq!(listed[0].workspaces[0].head, None);

        std::fs::rename(&moved, &root).unwrap();
        assert!(
            !fx.projects().list().unwrap()[0].missing,
            "it comes back by itself"
        );

        fx.store.remove_project(&added.project.id, false).unwrap();
        assert!(fx.projects().list().unwrap().is_empty());
        assert!(
            root.join(".git").exists(),
            "removing a project only forgets it"
        );
    }

    #[test]
    fn a_project_removed_with_its_history_comes_back_with_every_workspace() {
        let fx = Fixture::new();
        let added = fx.projects().create("app", &fx.path()).unwrap();
        let root = PathBuf::from(&added.project.root_path);
        // A worktree far from Yardsort's own folder, imported by hand: adoption would never
        // bring this one back on its own.
        let elsewhere = fx.path().join("elsewhere");
        fx.git
            .worktree_add(&root, &elsewhere, "theirs", "HEAD")
            .unwrap();
        let imported = crate::workspaces::import_worktrees(
            &fx.store,
            &fx.git,
            &added.project.id,
            &[elsewhere.to_string_lossy().into_owned()],
        )
        .unwrap();
        fx.store
            .add_session(&crate::store::NewSession {
                id: "s1",
                workspace_id: &imported[0].id,
                harness_id: "claude",
                title: "their task",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();

        fx.store.remove_project(&added.project.id, true).unwrap();
        assert!(fx.projects().list().unwrap().is_empty());

        let back = fx.projects().open(&root, false).unwrap();
        assert!(back.revived && !back.already_known);
        assert_eq!(back.project.id, added.project.id, "the same project");
        let names: Vec<_> = back.project.workspaces.iter().map(|w| &w.name).collect();
        assert_eq!(names, ["local", "elsewhere"]);
        assert_eq!(
            fx.store.sessions(&imported[0].id).unwrap().len(),
            1,
            "with its history"
        );
        assert!(!fx.projects().open(&root, false).unwrap().revived, "once");
    }

    #[test]
    fn opening_something_that_is_not_a_folder_fails_clearly() {
        let fx = Fixture::new();
        let file = fx.path().join("file.txt");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            fx.projects().open(&file, true).unwrap_err().code,
            "not_a_directory"
        );
    }
}

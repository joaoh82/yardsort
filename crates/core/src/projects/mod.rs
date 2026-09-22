//! Projects and their workspaces: the policy that sits between the store, git and the disk.

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::git::{normalize, Git, Head};
use crate::store::{ProjectRow, Store, WorkspaceRow};

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

    fn add(&self, root: &Path, opened_root_instead: bool) -> IpcResult<AddedProject> {
        let root_path = root.to_string_lossy().into_owned();
        let (row, already_known) = match self.store.project_by_root(&root_path)? {
            Some(existing) => (existing, true),
            None => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root_path.clone());
                (self.store.add_project(&name, &root_path)?, false)
            }
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

        fx.store.remove_project(&added.project.id).unwrap();
        assert!(fx.projects().list().unwrap().is_empty());
        assert!(
            root.join(".git").exists(),
            "removing a project only forgets it"
        );
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

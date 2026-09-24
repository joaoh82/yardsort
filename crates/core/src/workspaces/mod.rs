//! Worktree workspaces: one branch, one folder, one line of work.
//!
//! Everything here is plain git underneath — `git worktree add -b` and `git worktree remove` —
//! so a user can always inspect or undo what Yardsort did with their own tools.

mod naming;

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::git::{normalize, Git, GitError, WorktreeEntry};
use crate::settings::WorkspaceSettings;
use crate::store::{ProjectRow, Store, WorkspaceRow};

pub struct Workspaces<'a> {
    pub store: &'a Store,
    pub git: &'a Git,
    /// Worktrees live in `<root>/<project>/<workspace>`, outside the repositories themselves.
    pub worktree_root: &'a Path,
    /// Branch prefix and friends.
    pub settings: &'a WorkspaceSettings,
}

impl Workspaces<'_> {
    /// Create a branch from `base` and a worktree for it, named after `prompt`.
    ///
    /// Either everything exists afterwards — branch, folder, database row — or nothing does.
    pub fn create(
        &self,
        project_id: &str,
        base: Option<&str>,
        prompt: &str,
    ) -> IpcResult<WorkspaceRow> {
        let (project, root) = self.usable_project(project_id)?;
        let base = match base {
            Some(base) => base.to_owned(),
            None => self.git.default_branch(&root)?.ok_or_else(|| {
                IpcError::new(
                    "no_base_branch",
                    "Choose a branch to start from: the repository is not on one.",
                )
            })?,
        };

        let project_dir = self.project_dir(&project);
        let seed = self.store.workspaces()?.len();
        // A name is free only if neither its branch nor its folder exists — including leftovers
        // Yardsort does not know about.
        let mut probe_error = None;
        let name = naming::unique(&naming::base_name(prompt, seed), |candidate| {
            let branch_taken = self
                .git
                .branch_exists(&root, &self.settings.branch_for(candidate))
                .unwrap_or_else(|e| {
                    probe_error = Some(e);
                    false
                });
            branch_taken || project_dir.join(candidate).exists()
        });
        if let Some(error) = probe_error {
            return Err(error.into());
        }

        let branch = self.settings.branch_for(&name);
        let path = project_dir.join(&name);
        self.ensure_dir(&project_dir)?;
        self.git.worktree_add(&root, &path, &branch, &base)?;
        self.record(&project, &root, &name, &path, &branch, Some(&base))
    }

    /// Open an *existing* branch as a workspace: a worktree for it, no new branch. This is how a
    /// branch kept by an earlier delete comes back. Git refuses a branch that is already checked
    /// out somewhere, which is exactly the rule we want.
    pub fn open_branch(&self, project_id: &str, branch: &str) -> IpcResult<WorkspaceRow> {
        let (project, root) = self.usable_project(project_id)?;
        if !self.git.branch_exists(&root, branch)? {
            return Err(IpcError::new(
                "unknown_branch",
                format!("There is no branch \"{branch}\"."),
            ));
        }
        let project_dir = self.project_dir(&project);
        // `ys/fix-login` comes back as `fix-login`; other branches are named after themselves.
        let own = self.settings.name_from_branch(branch);
        let base = naming::slugify_name(own).unwrap_or_else(|| naming::base_name("", 0));
        let name = naming::unique(&base, |candidate| project_dir.join(candidate).exists());

        let path = project_dir.join(&name);
        self.ensure_dir(&project_dir)?;
        self.git.worktree_add_existing(&root, &path, branch)?;
        self.record(&project, &root, &name, &path, branch, None)
    }

    fn usable_project(&self, project_id: &str) -> IpcResult<(ProjectRow, PathBuf)> {
        let project = self
            .store
            .project(project_id)?
            .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
        let root = PathBuf::from(&project.root_path);
        if !root.is_dir() {
            return Err(IpcError::new(
                "project_missing",
                format!("{} does not exist any more.", project.root_path),
            ));
        }
        if !self.git.has_commits(&root)? {
            return Err(IpcError::new(
                "no_commits",
                "This repository has no commits yet, so there is nothing to branch from. Make a first commit, then try again.",
            ));
        }
        Ok((project, root))
    }

    fn project_dir(&self, project: &ProjectRow) -> PathBuf {
        self.worktree_root
            .join(naming::slugify_name(&project.name).unwrap_or_else(|| project.id.clone()))
    }

    fn ensure_dir(&self, dir: &Path) -> IpcResult<()> {
        std::fs::create_dir_all(dir)
            .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", dir.display())))
    }

    /// Store a freshly added worktree. `base` is `Some` only when we created the branch.
    fn record(
        &self,
        project: &ProjectRow,
        root: &Path,
        name: &str,
        path: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> IpcResult<WorkspaceRow> {
        // Store the path the way git reports it (symlinks resolved — `/tmp` is `/private/tmp` on
        // macOS), or adoption would later mistake this worktree for an unknown one.
        let stored = normalize(path);
        match self.store.add_worktree(
            &project.id,
            name,
            &stored.to_string_lossy(),
            Some(branch),
            base,
        ) {
            Ok(row) => Ok(row),
            Err(error) => {
                self.undo(root, path, base.is_some().then_some(branch));
                Err(error.into())
            }
        }
    }

    /// Undo a workspace that was created a moment ago and never used, e.g. because its harness
    /// failed to start. A branch we created goes too — it has no commits of its own yet. A
    /// branch that existed before is never touched.
    pub fn discard(&self, workspace: &WorkspaceRow) -> IpcResult<()> {
        let root = self.project_root(workspace)?;
        self.store.remove_worktree(&workspace.id)?;
        let created_branch = workspace
            .base_branch
            .as_ref()
            .and(workspace.branch.as_deref());
        self.undo(&root, Path::new(&workspace.path), created_branch);
        Ok(())
    }

    /// Remove a workspace's worktree and forget it. The **branch is kept**: commits are never
    /// thrown away here. Without `force`, uncommitted work makes this fail with `worktree_dirty`.
    pub fn delete(&self, workspace_id: &str, force: bool) -> IpcResult<()> {
        let workspace = self.deletable(workspace_id)?;
        if !workspace.archived {
            self.remove_worktree(&workspace, force)?;
        }
        self.store.remove_worktree(&workspace.id)?;
        Ok(())
    }

    /// Put a workspace away: its worktree leaves the disk, while its row, its branch and its
    /// session history stay, so [`Self::restore`] can bring it back. Same `worktree_dirty` rule
    /// as deleting — uncommitted work lives only in the folder we are about to remove.
    pub fn archive(&self, workspace_id: &str, force: bool) -> IpcResult<()> {
        let workspace = self.deletable(workspace_id)?;
        if workspace.archived {
            return Ok(());
        }
        if workspace.branch.is_none() {
            return Err(IpcError::new(
                "not_archivable",
                "This workspace is not on a branch, so there would be nothing to restore it from. Delete it instead.",
            ));
        }
        self.remove_worktree(&workspace, force)?;
        self.store.set_workspace_archived(&workspace.id, true)?;
        Ok(())
    }

    /// Check a workspace's branch out again at its old path. Works for archived workspaces and
    /// for ones whose folder disappeared behind our back. The path matters: harnesses file
    /// their conversations by folder, so the same folder is what makes old sessions resumable.
    pub fn restore(&self, workspace_id: &str) -> IpcResult<WorkspaceRow> {
        let workspace = self.deletable(workspace_id)?;
        let root = self.project_root(&workspace)?;
        let path = PathBuf::from(&workspace.path);
        if path.exists() {
            if workspace.archived {
                return Err(IpcError::new(
                    "already_exists",
                    format!("{} is in the way.", path.display()),
                ));
            }
            return Ok(workspace);
        }
        let branch = workspace.branch.as_deref().ok_or_else(|| {
            IpcError::new("no_branch", "This workspace has no branch to restore from.")
        })?;
        if !self.git.branch_exists(&root, branch)? {
            return Err(IpcError::new(
                "unknown_branch",
                format!("The branch \"{branch}\" no longer exists, so this workspace cannot be restored."),
            ));
        }
        // Git may still list the vanished folder as a worktree holding the branch.
        let _ = self.git.worktree_prune(&root);
        if let Some(parent) = path.parent() {
            self.ensure_dir(parent)?;
        }
        self.git.worktree_add_existing(&root, &path, branch)?;
        self.store.set_workspace_archived(&workspace.id, false)?;
        Ok(WorkspaceRow {
            archived: false,
            ..workspace
        })
    }

    /// Change a workspace's display name. Folder and branch keep theirs: renaming those would
    /// orphan the harness conversations filed under the folder.
    pub fn rename(&self, workspace_id: &str, name: &str) -> IpcResult<WorkspaceRow> {
        let workspace = self.deletable(workspace_id)?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(IpcError::new(
                "invalid_name",
                "A workspace name needs 1 to 80 characters.",
            ));
        }
        self.store.rename_workspace(&workspace.id, name)?;
        Ok(WorkspaceRow {
            name: name.to_owned(),
            ..workspace
        })
    }

    /// Stop showing a workspace without touching its folder or its branch: the worktree was made
    /// elsewhere, or is wanted elsewhere, and Yardsort is simply not the place to see it. With
    /// `keep_history` its conversations wait, hidden, for the worktree to be imported again.
    pub fn forget(&self, workspace_id: &str, keep_history: bool) -> IpcResult<()> {
        let workspace = self.deletable(workspace_id)?;
        self.store.forget_workspace(&workspace.id, keep_history)?;
        Ok(())
    }

    /// A worktree workspace; `local` can be neither deleted, archived nor renamed.
    fn deletable(&self, workspace_id: &str) -> IpcResult<WorkspaceRow> {
        let workspace = self.store.workspace(workspace_id)?.ok_or_else(|| {
            IpcError::new("unknown_workspace", "That workspace no longer exists.")
        })?;
        if workspace.kind != "worktree" {
            return Err(IpcError::new(
                "not_deletable",
                "The local workspace cannot be changed this way.",
            ));
        }
        Ok(workspace)
    }

    fn remove_worktree(&self, workspace: &WorkspaceRow, force: bool) -> IpcResult<()> {
        let root = self.project_root(workspace)?;
        let path = PathBuf::from(&workspace.path);
        if path.exists() && root.is_dir() {
            match self.git.worktree_remove(&root, &path, force) {
                Ok(()) => {}
                Err(GitError::Failed { stderr, .. })
                    if !force
                        && (stderr.contains("modified or untracked")
                            || stderr.contains("--force")) =>
                {
                    return Err(IpcError::new(
                        "worktree_dirty",
                        "This workspace has uncommitted changes or untracked files.",
                    ));
                }
                Err(other) => return Err(other.into()),
            }
        } else if root.is_dir() {
            // The folder was deleted behind our back; let git forget it as well.
            let _ = self.git.worktree_prune(&root);
        }
        Ok(())
    }

    fn project_root(&self, workspace: &WorkspaceRow) -> IpcResult<PathBuf> {
        self.store
            .project(&workspace.project_id)?
            .map(|project| PathBuf::from(project.root_path))
            .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))
    }

    /// Take back a worktree, and the branch too if (and only if) we created it.
    fn undo(&self, root: &Path, path: &Path, created_branch: Option<&str>) {
        let _ = self.git.worktree_remove(root, path, true);
        let _ = self.git.worktree_prune(root);
        if let Some(branch) = created_branch {
            let _ = self.git.branch_delete(root, branch);
        }
    }
}

/// Where worktrees go: the environment override (for tests and experiments), then the setting,
/// then `~/yardsort` — visible and short on purpose, because people look into these folders and
/// Windows paths are limited.
///
/// Every client has to agree on this, or one of them creates worktrees the other cannot find.
pub fn worktree_root(
    settings: &WorkspaceSettings,
    env: &crate::env::ShellEnv,
) -> IpcResult<PathBuf> {
    if let Some(dir) = crate::legacy::env_var_os("WORKTREE_ROOT") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(root) = &settings.worktree_root {
        return Ok(PathBuf::from(root));
    }
    default_worktree_root(env)
}

/// The fallback, when nothing has been configured.
pub fn default_worktree_root(env: &crate::env::ShellEnv) -> IpcResult<PathBuf> {
    env.home_dir()
        .map(|home| home.join("yardsort"))
        .ok_or_else(|| IpcError::new("no_home", "Cannot determine your home directory."))
}

/// A worktree git knows about that is not a workspace: made by hand, by another tool, or
/// forgotten here. What the import dialog lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UntrackedWorktree {
    pub path: String,
    /// `None` when detached.
    pub branch: Option<String>,
}

/// Adopt worktrees git knows about but Yardsort does not, **when they sit under Yardsort's own
/// worktree root** — ours, orphaned when their project was removed and added again. Worktrees
/// anywhere else were made for some other purpose and stay out until they are imported. Returns
/// how many were adopted.
pub fn adopt_unknown(
    store: &Store,
    git: &Git,
    project_id: &str,
    worktree_root: &Path,
) -> IpcResult<usize> {
    let Some(project) = store.project(project_id)? else {
        return Ok(0);
    };
    let root = PathBuf::from(&project.root_path);
    if !root.is_dir() {
        return Ok(0);
    }
    // Forgotten rows count as known: the user asked not to see those again.
    let known: Vec<PathBuf> = store
        .workspace_paths()?
        .iter()
        .map(|path| normalize(Path::new(path)))
        .collect();
    let ours = normalize(worktree_root);

    let mut adopted = 0;
    for entry in git.worktrees(&root)? {
        if !linked(&entry) || known.contains(&entry.path) || !entry.path.starts_with(&ours) {
            continue;
        }
        let recorded = store.add_worktree_if_new(
            &project.id,
            &folder_name(&entry.path),
            &entry.path.to_string_lossy(),
            entry.branch.as_deref(),
        )?;
        adopted += usize::from(recorded.is_some());
    }
    Ok(adopted)
}

/// The worktrees of a project that are not workspaces, forgotten ones included, in git's order.
pub fn untracked_worktrees(
    store: &Store,
    git: &Git,
    project_id: &str,
) -> IpcResult<Vec<UntrackedWorktree>> {
    Ok(untracked_entries(store, git, project_id)?
        .into_iter()
        .map(|entry| UntrackedWorktree {
            path: entry.path.to_string_lossy().into_owned(),
            branch: entry.branch,
        })
        .collect())
}

/// Make workspaces of untracked worktrees, chosen by path. Every path is checked first, so a
/// wrong one — not a worktree of this project, or a workspace already — imports nothing. A
/// forgotten workspace at that path comes back with its history; anything else is named after
/// its folder. Nothing on disk is touched.
pub fn import_worktrees(
    store: &Store,
    git: &Git,
    project_id: &str,
    paths: &[String],
) -> IpcResult<Vec<WorkspaceRow>> {
    let untracked = untracked_entries(store, git, project_id)?;
    let chosen = paths
        .iter()
        .map(|path| {
            let wanted = normalize(Path::new(path));
            untracked
                .iter()
                .find(|entry| entry.path == wanted)
                .ok_or_else(|| {
                    IpcError::new(
                        "not_importable",
                        format!(
                            "{path} is not a worktree of this project that Yardsort could import."
                        ),
                    )
                })
        })
        .collect::<IpcResult<Vec<_>>>()?;

    let mut imported = Vec::with_capacity(chosen.len());
    for entry in chosen {
        let path = entry.path.to_string_lossy();
        let row = match store.forgotten_workspace_at(&path)? {
            Some(forgotten) => {
                store.revive_workspace(&forgotten.id, entry.branch.as_deref())?;
                store.workspace(&forgotten.id)?.unwrap_or(forgotten)
            }
            None => match store.add_worktree_if_new(
                project_id,
                &folder_name(&entry.path),
                &path,
                entry.branch.as_deref(),
            )? {
                Some(row) => row,
                // Adopted or imported from elsewhere a moment ago: the outcome is the same.
                None => continue,
            },
        };
        imported.push(row);
    }
    Ok(imported)
}

fn untracked_entries(store: &Store, git: &Git, project_id: &str) -> IpcResult<Vec<WorktreeEntry>> {
    let Some(project) = store.project(project_id)? else {
        return Err(IpcError::new(
            "unknown_project",
            "That project no longer exists.",
        ));
    };
    let root = PathBuf::from(&project.root_path);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let shown: Vec<PathBuf> = store
        .workspaces()?
        .iter()
        .map(|w| normalize(Path::new(&w.path)))
        .collect();
    Ok(git
        .worktrees(&root)?
        .into_iter()
        .filter(|entry| linked(entry) && !shown.contains(&entry.path))
        .collect())
}

/// A worktree that could be a workspace: linked, and still on disk.
fn linked(entry: &WorktreeEntry) -> bool {
    !entry.is_main && !entry.bare && !entry.prunable
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::git;
    use crate::git::Head;
    use crate::projects::Projects;

    struct Fixture {
        store: Store,
        git: Git,
        _dirs: (tempfile::TempDir, tempfile::TempDir),
        repo: PathBuf,
        worktrees: PathBuf,
        project_id: String,
        settings: WorkspaceSettings,
    }

    impl Fixture {
        fn new() -> Self {
            let (code, worktrees) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let (store, git) = (Store::in_memory(), git());
            let added = Projects {
                store: &store,
                git: &git,
            }
            .create("My App", &normalize(code.path()))
            .unwrap();
            Self {
                repo: PathBuf::from(&added.project.root_path),
                worktrees: normalize(worktrees.path()),
                project_id: added.project.id,
                settings: WorkspaceSettings::default(),
                _dirs: (code, worktrees),
                store,
                git,
            }
        }

        fn workspaces(&self) -> Workspaces<'_> {
            Workspaces {
                store: &self.store,
                git: &self.git,
                worktree_root: &self.worktrees,
                settings: &self.settings,
            }
        }

        fn names(&self) -> Vec<String> {
            self.store
                .workspaces()
                .unwrap()
                .into_iter()
                .map(|w| w.name)
                .collect()
        }
    }

    #[test]
    fn a_workspace_is_a_branch_in_a_worktree_outside_the_repository() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "Fix the login bug")
            .unwrap();

        assert_eq!(ws.name, "fix-login-bug");
        assert_eq!(ws.branch.as_deref(), Some("ys/fix-login-bug"));
        let path = PathBuf::from(&ws.path);
        assert_eq!(path, fx.worktrees.join("my-app").join("fix-login-bug"));
        assert!(!path.starts_with(&fx.repo));
        assert_eq!(
            fx.git.head(&path).unwrap(),
            Head::Branch("ys/fix-login-bug".into())
        );
        assert_eq!(fx.names(), ["local", "fix-login-bug"]);
    }

    #[test]
    fn workspace_branch_and_folder_use_the_task_after_background_context() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(
                &fx.project_id,
                None,
                "I have been looking at authentication. Could you please fix login crashes?",
            )
            .unwrap();
        assert_eq!(ws.name, "fix-login-crashes");
        assert_eq!(ws.branch.as_deref(), Some("ys/fix-login-crashes"));
        assert_eq!(
            PathBuf::from(&ws.path),
            fx.worktrees.join("my-app/fix-login-crashes")
        );
        assert_eq!(
            fx.git.head(Path::new(&ws.path)).unwrap(),
            Head::Branch("ys/fix-login-crashes".into())
        );
    }

    #[test]
    fn the_same_prompt_twice_gets_a_numbered_name_even_around_leftovers() {
        let fx = Fixture::new();
        fx.workspaces()
            .create(&fx.project_id, None, "add tests")
            .unwrap();
        // A branch Yardsort does not know about still counts as taken.
        fx.git
            .worktree_add(
                &fx.repo,
                &fx.worktrees.join("elsewhere"),
                "ys/add-tests-2",
                &fx.git.default_branch(&fx.repo).unwrap().unwrap(),
            )
            .unwrap();

        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "Add tests!")
            .unwrap();
        assert_eq!(ws.name, "add-tests-3");
    }

    #[test]
    fn the_branch_prefix_is_a_setting_and_may_be_empty() {
        let mut fx = Fixture::new();
        fx.settings.branch_prefix = "joao/wip".into();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "tidy up")
            .unwrap();
        assert_eq!(ws.branch.as_deref(), Some("joao/wip/tidy-up"));
        assert_eq!(ws.name, "tidy-up");
        fx.workspaces().delete(&ws.id, false).unwrap();
        assert_eq!(
            fx.workspaces()
                .open_branch(&fx.project_id, "joao/wip/tidy-up")
                .unwrap()
                .name,
            "tidy-up"
        );

        fx.settings.branch_prefix = String::new();
        let bare = fx
            .workspaces()
            .create(&fx.project_id, None, "no prefix")
            .unwrap();
        assert_eq!(bare.branch.as_deref(), Some("no-prefix"));
    }

    #[test]
    fn an_empty_prompt_still_gets_a_name() {
        let fx = Fixture::new();
        let ws = fx.workspaces().create(&fx.project_id, None, "   ").unwrap();
        assert!(!ws.name.is_empty());
        assert!(PathBuf::from(&ws.path).is_dir());
    }

    #[test]
    fn it_branches_from_the_base_that_was_asked_for() {
        let fx = Fixture::new();
        fx.git
            .worktree_add(&fx.repo, &fx.worktrees.join("rel"), "release", "HEAD")
            .unwrap();
        std::fs::write(fx.worktrees.join("rel").join("notes.md"), "v1").unwrap();
        let rel = fx.worktrees.join("rel");
        fx.git.run(&rel, &["add", "."]).unwrap();
        fx.git.run(&rel, &["commit", "-qm", "notes"]).unwrap();

        let ws = fx
            .workspaces()
            .create(&fx.project_id, Some("release"), "hotfix")
            .unwrap();
        assert!(
            PathBuf::from(&ws.path).join("notes.md").exists(),
            "started from `release`"
        );

        let err = fx
            .workspaces()
            .create(&fx.project_id, Some("no-such-branch"), "x")
            .unwrap_err();
        assert_eq!(err.code, "git_failed");
        assert_eq!(
            fx.names(),
            ["local", "hotfix"],
            "a failed create leaves nothing behind"
        );
        assert!(!fx.git.branch_exists(&fx.repo, "ys/x").unwrap());
    }

    #[test]
    fn a_repository_without_commits_is_refused_with_an_explanation() {
        let fx = Fixture::new();
        let dir = tempfile::tempdir().unwrap();
        fx.git.init(dir.path()).unwrap();
        let empty = Projects {
            store: &fx.store,
            git: &fx.git,
        }
        .open(dir.path(), false)
        .unwrap();
        let err = fx
            .workspaces()
            .create(&empty.project.id, None, "anything")
            .unwrap_err();
        assert_eq!(err.code, "no_commits");
    }

    #[test]
    fn discarding_removes_folder_branch_and_row() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "doomed")
            .unwrap();
        fx.workspaces().discard(&ws).unwrap();
        assert!(!PathBuf::from(&ws.path).exists());
        assert!(!fx.git.branch_exists(&fx.repo, "ys/doomed").unwrap());
        assert_eq!(fx.names(), ["local"]);
    }

    #[test]
    fn a_kept_branch_comes_back_as_a_workspace_with_its_work() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "write the docs")
            .unwrap();
        let path = PathBuf::from(&ws.path);
        std::fs::write(path.join("DOCS.md"), "hello").unwrap();
        fx.git.run(&path, &["add", "."]).unwrap();
        fx.git.run(&path, &["commit", "-qm", "docs"]).unwrap();
        fx.workspaces().delete(&ws.id, false).unwrap();
        assert_eq!(fx.names(), ["local"]);

        let back = fx
            .workspaces()
            .open_branch(&fx.project_id, "ys/write-docs")
            .unwrap();

        assert_eq!(back.name, "write-docs");
        assert_eq!(back.branch.as_deref(), Some("ys/write-docs"));
        assert_eq!(back.base_branch, None, "we did not create this branch");
        assert!(
            PathBuf::from(&back.path).join("DOCS.md").exists(),
            "the commit is there"
        );
    }

    #[test]
    fn opening_a_branch_never_puts_the_branch_at_risk() {
        let fx = Fixture::new();
        fx.git.run(&fx.repo, &["branch", "feature/Big Thing"]).ok();
        fx.git.run(&fx.repo, &["branch", "release"]).unwrap();

        let ws = fx
            .workspaces()
            .open_branch(&fx.project_id, "release")
            .unwrap();
        assert_eq!(ws.name, "release");
        // Discarding (the harness failed to start) takes back the folder, not the branch.
        fx.workspaces().discard(&ws).unwrap();
        assert!(!PathBuf::from(&ws.path).exists());
        assert!(fx.git.branch_exists(&fx.repo, "release").unwrap());

        let err = fx
            .workspaces()
            .open_branch(&fx.project_id, "nope")
            .unwrap_err();
        assert_eq!(err.code, "unknown_branch");
    }

    #[test]
    fn a_branch_already_checked_out_is_refused_and_leaves_nothing_behind() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "busy")
            .unwrap();
        let err = fx
            .workspaces()
            .open_branch(&fx.project_id, "ys/busy")
            .unwrap_err();
        assert_eq!(err.code, "git_failed");
        assert_eq!(fx.names(), ["local", "busy"]);
        assert!(PathBuf::from(&ws.path).is_dir());
    }

    #[test]
    fn worktrees_under_our_root_are_adopted_once_and_others_are_not() {
        let fx = Fixture::new();
        let ours = fx
            .workspaces()
            .create(&fx.project_id, None, "known")
            .unwrap();
        let by_hand = fx.worktrees.join("made-by-hand");
        fx.git
            .worktree_add(&fx.repo, &by_hand, "experiment", "HEAD")
            .unwrap();
        let stale = fx.worktrees.join("stale");
        fx.git
            .worktree_add(&fx.repo, &stale, "old", "HEAD")
            .unwrap();
        std::fs::remove_dir_all(&stale).unwrap();
        // Somebody's own worktree, nowhere near ours: not Yardsort's to show uninvited.
        let elsewhere = tempfile::tempdir().unwrap();
        let theirs = normalize(elsewhere.path()).join("theirs");
        fx.git
            .worktree_add(&fx.repo, &theirs, "their-branch", "HEAD")
            .unwrap();

        assert_eq!(
            adopt_unknown(&fx.store, &fx.git, &fx.project_id, &fx.worktrees).unwrap(),
            1
        );
        assert_eq!(fx.names(), ["local", "known", "made-by-hand"]);
        assert_eq!(
            untracked_worktrees(&fx.store, &fx.git, &fx.project_id).unwrap(),
            [UntrackedWorktree {
                path: theirs.to_string_lossy().into_owned(),
                branch: Some("their-branch".into()),
            }],
            "listed for importing, prunable ones left out"
        );
        let adopted = fx.store.workspaces().unwrap().pop().unwrap();
        assert_eq!(adopted.branch.as_deref(), Some("experiment"));
        assert_eq!(PathBuf::from(&adopted.path), by_hand);
        assert_eq!(adopted.base_branch, None);

        assert_eq!(
            adopt_unknown(&fx.store, &fx.git, &fx.project_id, &fx.worktrees).unwrap(),
            0,
            "idempotent"
        );
        assert!(PathBuf::from(&ours.path).is_dir());
    }

    #[test]
    fn overlapping_adoption_runs_list_a_worktree_once() {
        let fx = Fixture::new();
        fx.git
            .worktree_add(
                &fx.repo,
                &fx.worktrees.join("by-hand"),
                "experiment",
                "HEAD",
            )
            .unwrap();

        // The project list loads from several places at once (startup, window focus), and every
        // load adopts. Run many in parallel, as the app does.
        let adopted: usize = std::thread::scope(|scope| {
            let runs: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        adopt_unknown(&fx.store, &fx.git, &fx.project_id, &fx.worktrees).unwrap()
                    })
                })
                .collect();
            runs.into_iter().map(|run| run.join().unwrap()).sum()
        });

        assert_eq!(adopted, 1);
        assert_eq!(fx.names(), ["local", "by-hand"]);
    }

    #[test]
    fn removing_and_re_adding_a_project_brings_its_workspaces_back() {
        let fx = Fixture::new();
        fx.workspaces()
            .create(&fx.project_id, None, "survivor")
            .unwrap();
        fx.store.remove_project(&fx.project_id).unwrap();

        let again = Projects {
            store: &fx.store,
            git: &fx.git,
        }
        .open(&fx.repo, false)
        .unwrap();
        assert_eq!(
            adopt_unknown(&fx.store, &fx.git, &again.project.id, &fx.worktrees).unwrap(),
            1
        );
        assert_eq!(fx.names(), ["local", "survivor"]);
    }

    #[test]
    fn importing_brings_chosen_worktrees_in_and_checks_every_path_first() {
        let fx = Fixture::new();
        let elsewhere = tempfile::tempdir().unwrap();
        let one = normalize(elsewhere.path()).join("one");
        let two = normalize(elsewhere.path()).join("two");
        fx.git.worktree_add(&fx.repo, &one, "one", "HEAD").unwrap();
        fx.git.worktree_add(&fx.repo, &two, "two", "HEAD").unwrap();
        let path = |p: &Path| p.to_string_lossy().into_owned();

        let bogus = path(&fx.repo.join("src"));
        let err =
            import_worktrees(&fx.store, &fx.git, &fx.project_id, &[path(&one), bogus]).unwrap_err();
        assert_eq!(err.code, "not_importable");
        assert_eq!(fx.names(), ["local"], "one bad path imports nothing");

        let imported = import_worktrees(&fx.store, &fx.git, &fx.project_id, &[path(&two)]).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].name, "two");
        assert_eq!(imported[0].branch.as_deref(), Some("two"));
        assert_eq!(imported[0].base_branch, None, "not our branch to delete");
        assert_eq!(fx.names(), ["local", "two"]);
        assert_eq!(
            untracked_worktrees(&fx.store, &fx.git, &fx.project_id)
                .unwrap()
                .iter()
                .map(|w| w.path.clone())
                .collect::<Vec<_>>(),
            [path(&one)]
        );
        assert!(one.is_dir() && two.is_dir(), "nothing on disk moved");
    }

    #[test]
    fn forgetting_hides_a_workspace_and_importing_it_again_finds_its_history() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "keep me")
            .unwrap();
        fx.store
            .add_session(&crate::store::NewSession {
                id: "s1",
                workspace_id: &ws.id,
                harness_id: "claude",
                title: "keep me",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        let path = PathBuf::from(&ws.path);

        fx.workspaces().forget(&ws.id, true).unwrap();
        assert_eq!(fx.names(), ["local"]);
        assert!(path.is_dir(), "the folder is not ours to remove");
        assert!(fx
            .git
            .branch_exists(&fx.repo, ws.branch.as_deref().unwrap())
            .unwrap());
        assert_eq!(
            adopt_unknown(&fx.store, &fx.git, &fx.project_id, &fx.worktrees).unwrap(),
            0,
            "forgotten means forgotten, even under our own root"
        );
        let listed = untracked_worktrees(&fx.store, &fx.git, &fx.project_id).unwrap();
        assert_eq!(listed.len(), 1, "but it can be imported again");

        let back = import_worktrees(
            &fx.store,
            &fx.git,
            &fx.project_id,
            &[listed[0].path.clone()],
        )
        .unwrap();
        assert_eq!(back[0].id, ws.id, "the same workspace, history and all");
        assert_eq!(fx.store.sessions(&ws.id).unwrap().len(), 1);
        assert_eq!(fx.names(), ["local", ws.name.as_str()]);

        fx.workspaces().forget(&ws.id, false).unwrap();
        assert!(fx.store.workspace(&ws.id).unwrap().is_none());
        assert!(path.is_dir());
        let err = fx
            .workspaces()
            .forget(&fx.store.workspaces().unwrap()[0].id, true)
            .unwrap_err();
        assert_eq!(err.code, "not_deletable");
    }

    #[test]
    fn deleting_keeps_the_branch_and_protects_uncommitted_work() {
        let fx = Fixture::new();
        let ws = fx.workspaces().create(&fx.project_id, None, "wip").unwrap();
        let path = PathBuf::from(&ws.path);
        std::fs::write(path.join("draft.txt"), "not committed").unwrap();

        let err = fx.workspaces().delete(&ws.id, false).unwrap_err();
        assert_eq!(err.code, "worktree_dirty");
        assert!(path.join("draft.txt").exists(), "nothing was lost");
        assert_eq!(fx.names(), ["local", "wip"]);

        fx.workspaces().delete(&ws.id, true).unwrap();
        assert!(!path.exists());
        assert_eq!(fx.names(), ["local"]);
        assert!(
            fx.git.branch_exists(&fx.repo, "ys/wip").unwrap(),
            "commits are never thrown away"
        );
    }

    #[test]
    fn a_workspace_whose_folder_vanished_can_still_be_deleted() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "gone")
            .unwrap();
        std::fs::remove_dir_all(&ws.path).unwrap();
        fx.workspaces().delete(&ws.id, false).unwrap();
        assert_eq!(fx.names(), ["local"]);
        // …and git has forgotten it too, so the name is free again.
        fx.git.branch_delete(&fx.repo, "ys/gone").unwrap();
        assert_eq!(
            fx.workspaces()
                .create(&fx.project_id, None, "gone")
                .unwrap()
                .name,
            "gone"
        );
    }

    #[test]
    fn archiving_removes_the_folder_and_restoring_brings_the_same_one_back() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "shelve me")
            .unwrap();
        let path = PathBuf::from(&ws.path);
        std::fs::write(path.join("work.txt"), "committed work").unwrap();
        fx.git.run(&path, &["add", "."]).unwrap();
        fx.git.run(&path, &["commit", "-m", "work"]).unwrap();

        fx.workspaces().archive(&ws.id, false).unwrap();
        assert!(!path.exists());
        let row = fx.store.workspace(&ws.id).unwrap().unwrap();
        assert!(row.archived, "the row stays, flagged");
        assert!(fx
            .git
            .branch_exists(&fx.repo, ws.branch.as_deref().unwrap())
            .unwrap());
        fx.workspaces().archive(&ws.id, false).unwrap(); // archiving twice is harmless

        let restored = fx.workspaces().restore(&ws.id).unwrap();
        assert!(!restored.archived);
        assert_eq!(
            restored.path, ws.path,
            "same folder, so harness sessions still resolve"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("work.txt")).unwrap(),
            "committed work"
        );
        assert!(!fx.store.workspace(&ws.id).unwrap().unwrap().archived);
    }

    #[test]
    fn archiving_protects_uncommitted_work_like_deleting_does() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "dirty")
            .unwrap();
        std::fs::write(PathBuf::from(&ws.path).join("draft.txt"), "x").unwrap();
        assert_eq!(
            fx.workspaces().archive(&ws.id, false).unwrap_err().code,
            "worktree_dirty"
        );
        assert!(!fx.store.workspace(&ws.id).unwrap().unwrap().archived);
        fx.workspaces().archive(&ws.id, true).unwrap();
        assert!(fx.store.workspace(&ws.id).unwrap().unwrap().archived);
    }

    #[test]
    fn a_vanished_workspace_can_be_restored_and_an_archived_one_deleted() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "vanish")
            .unwrap();
        std::fs::remove_dir_all(&ws.path).unwrap();
        // Git still believes the branch is checked out in the folder that is gone.
        fx.workspaces().restore(&ws.id).unwrap();
        assert!(PathBuf::from(&ws.path).join(".git").exists());

        fx.workspaces().archive(&ws.id, false).unwrap();
        fx.workspaces().delete(&ws.id, false).unwrap();
        assert_eq!(fx.names(), ["local"]);
    }

    #[test]
    fn restoring_needs_the_branch_to_still_exist() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "doomed")
            .unwrap();
        fx.workspaces().archive(&ws.id, false).unwrap();
        fx.git
            .branch_delete(&fx.repo, ws.branch.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            fx.workspaces().restore(&ws.id).unwrap_err().code,
            "unknown_branch"
        );
    }

    #[test]
    fn renaming_changes_the_label_only() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "fix login")
            .unwrap();
        let renamed = fx
            .workspaces()
            .rename(&ws.id, "  Login: the sequel  ")
            .unwrap();
        assert_eq!(renamed.name, "Login: the sequel");
        assert_eq!((renamed.path, renamed.branch), (ws.path, ws.branch));
        for bad in ["", "   ", &"x".repeat(81), "new\nline"] {
            assert_eq!(
                fx.workspaces().rename(&ws.id, bad).unwrap_err().code,
                "invalid_name"
            );
        }
        let local = fx.store.workspaces().unwrap().remove(0);
        assert_eq!(
            fx.workspaces().rename(&local.id, "x").unwrap_err().code,
            "not_deletable"
        );
    }

    #[test]
    fn a_worktree_and_branch_removed_by_hand_leave_nothing_to_restore_from() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "by hand")
            .unwrap();
        let branch = ws.branch.clone().unwrap();
        let described = |id: &str| {
            Projects {
                store: &fx.store,
                git: &fx.git,
            }
            .list()
            .unwrap()
            .remove(0)
            .workspaces
            .into_iter()
            .find(|w| w.id == id)
            .unwrap()
        };

        let healthy = described(&ws.id);
        assert!(!healthy.missing && !healthy.branch_gone);

        // `git worktree remove` on its own: the branch is still there to come back from.
        fx.git
            .worktree_remove(&fx.repo, Path::new(&ws.path), false)
            .unwrap();
        let orphan = described(&ws.id);
        assert!(orphan.missing && !orphan.branch_gone);

        // `git branch -D` as well, and there is no way back.
        fx.git.branch_delete(&fx.repo, &branch).unwrap();
        let orphan = described(&ws.id);
        assert!(orphan.missing && orphan.branch_gone);

        // The project's own checkout never claims a branch it does not have.
        let local = described(&fx.store.workspaces().unwrap()[0].id);
        assert!(!local.branch_gone);
    }

    #[test]
    fn an_archived_workspace_notices_its_branch_being_deleted() {
        let fx = Fixture::new();
        let ws = fx
            .workspaces()
            .create(&fx.project_id, None, "shelved")
            .unwrap();
        fx.workspaces().archive(&ws.id, false).unwrap();
        let projects = Projects {
            store: &fx.store,
            git: &fx.git,
        };
        let described =
            || projects.describe_workspace(fx.store.workspace(&ws.id).unwrap().unwrap());

        let shelved = described();
        assert!(shelved.archived && !shelved.missing && !shelved.branch_gone);

        fx.git
            .branch_delete(&fx.repo, ws.branch.as_deref().unwrap())
            .unwrap();
        let shelved = described();
        assert!(
            shelved.branch_gone,
            "restoring it would now fail; the UI has to say so"
        );
    }

    #[test]
    fn local_cannot_be_deleted() {
        let fx = Fixture::new();
        let local = fx.store.workspaces().unwrap().remove(0);
        assert_eq!(
            fx.workspaces().delete(&local.id, true).unwrap_err().code,
            "not_deletable"
        );
        assert!(fx.repo.join(".git").exists());
    }
}

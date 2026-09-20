//! Git, by way of the `git` CLI: the reference implementation, and the only one that respects
//! the user's config, hooks and credentials. Everything goes through [`Git::run`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::env::ShellEnv;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not installed, or not on PATH")]
    NotInstalled,
    #[error("`git {command}` failed: {stderr}")]
    Failed { command: String, stderr: String },
    #[error("could not run git: {0}")]
    Io(#[from] std::io::Error),
}

pub type GitResult<T> = Result<T, GitError>;

/// One entry of `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// `None` when detached (or bare).
    pub branch: Option<String>,
    /// The repository's own checkout, as opposed to a linked worktree.
    pub is_main: bool,
    pub bare: bool,
    /// Git considers it stale: its folder is gone.
    pub prunable: bool,
}

/// What `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// A branch with no commits yet (a freshly initialised repository).
    Unborn(String),
    /// Not on a branch; the abbreviated commit id.
    Detached(String),
}

pub struct Git {
    program: PathBuf,
    env: Vec<(String, String)>,
    clear_env: bool,
}

impl Git {
    /// Find git on the *user's* `PATH` and run it with the user's environment.
    pub fn new(env: &ShellEnv) -> GitResult<Self> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let program = env
            .find_program("git", &cwd)
            .ok_or(GitError::NotInstalled)?;
        Ok(Self {
            program,
            env: env
                .vars
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            clear_env: env.replaces_inherited(),
        })
    }

    pub(crate) fn run(&self, cwd: &Path, args: &[&str]) -> GitResult<String> {
        let bytes = self.run_bytes(cwd, args)?;
        Ok(String::from_utf8_lossy(&bytes).trim_end().to_owned())
    }

    /// Like [`Self::run`], with stdout exactly as git wrote it: file contents, `-z` lists.
    pub(crate) fn run_bytes(&self, cwd: &Path, args: &[&str]) -> GitResult<Vec<u8>> {
        let mut command = Command::new(&self.program);
        if self.clear_env {
            command.env_clear();
        }
        command
            .args(args)
            .current_dir(cwd)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            // Never block on a credential or passphrase prompt nobody can see.
            .env("GIT_TERMINAL_PROMPT", "0")
            // Keep messages in English: a few callers have to recognise them.
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let output = command.output()?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(GitError::Failed {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// What `git --version` prints, e.g. `git version 2.55.0`.
    pub fn version(&self) -> GitResult<String> {
        self.run(&std::env::current_dir().unwrap_or_default(), &["--version"])
    }

    /// The top level of the repository containing `path`, or `None` if there isn't one.
    pub fn repo_root(&self, path: &Path) -> GitResult<Option<PathBuf>> {
        match self.run(path, &["rev-parse", "--show-toplevel"]) {
            Ok(root) => Ok(Some(normalize(Path::new(&root)))),
            Err(GitError::Failed { stderr, .. }) if stderr.contains("not a git repository") => {
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }

    pub fn init(&self, path: &Path) -> GitResult<()> {
        self.run(path, &["init"]).map(drop)
    }

    pub fn has_commits(&self, root: &Path) -> GitResult<bool> {
        match self.run(root, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(other) => Err(other),
        }
    }

    /// Create an empty first commit. A repository without commits cannot have worktrees, so
    /// Yardsort never leaves one it created in that state.
    pub fn initial_commit(&self, root: &Path) -> GitResult<()> {
        self.run(root, &["commit", "--allow-empty", "-m", "Initial commit"])
            .map(drop)
    }

    pub fn head(&self, root: &Path) -> GitResult<Head> {
        match self.run(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
            Ok(branch) if self.has_commits(root)? => Ok(Head::Branch(branch)),
            Ok(branch) => Ok(Head::Unborn(branch)),
            // Not a symbolic ref: HEAD is detached.
            Err(GitError::Failed { .. }) => self
                .run(root, &["rev-parse", "--short", "HEAD"])
                .map(Head::Detached),
            Err(other) => Err(other),
        }
    }
}

impl Git {
    /// Local branch names, in git's own order.
    pub fn branches(&self, root: &Path) -> GitResult<Vec<String>> {
        let out = self.run(
            root,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )?;
        Ok(out.lines().map(str::to_owned).collect())
    }

    pub fn branch_exists(&self, root: &Path, branch: &str) -> GitResult<bool> {
        let reference = format!("refs/heads/{branch}");
        match self.run(root, &["show-ref", "--verify", "--quiet", &reference]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(other) => Err(other),
        }
    }

    /// The branch new work should start from: what the remote calls its default if we know it
    /// and have it locally, otherwise whatever is checked out.
    pub fn default_branch(&self, root: &Path) -> GitResult<Option<String>> {
        if let Ok(remote_head) = self.run(
            root,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
        ) {
            let name = remote_head.strip_prefix("origin/").unwrap_or(&remote_head);
            if self.branch_exists(root, name)? {
                return Ok(Some(name.to_owned()));
            }
        }
        Ok(match self.head(root)? {
            Head::Branch(name) => Some(name),
            Head::Unborn(_) | Head::Detached(_) => None,
        })
    }

    /// Create `branch` from `base` and check it out in a new worktree at `path`.
    pub fn worktree_add(
        &self,
        root: &Path,
        path: &Path,
        branch: &str,
        base: &str,
    ) -> GitResult<()> {
        let path = path.to_string_lossy();
        self.run(root, &["worktree", "add", "-b", branch, &path, base])
            .map(drop)
    }

    /// Check out the *existing* `branch` in a new worktree at `path`. Git refuses if the branch
    /// is already checked out somewhere else.
    pub fn worktree_add_existing(&self, root: &Path, path: &Path, branch: &str) -> GitResult<()> {
        let path = path.to_string_lossy();
        self.run(root, &["worktree", "add", &path, branch])
            .map(drop)
    }

    /// Every worktree git knows for this repository, the main checkout first.
    pub fn worktrees(&self, root: &Path) -> GitResult<Vec<WorktreeEntry>> {
        let out = self.run(root, &["worktree", "list", "--porcelain"])?;
        Ok(parse_worktrees(&out))
    }

    /// Remove a worktree. Without `force`, git refuses if it holds modified or untracked files.
    pub fn worktree_remove(&self, root: &Path, path: &Path, force: bool) -> GitResult<()> {
        let path = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path);
        self.run(root, &args).map(drop)
    }

    /// Forget worktrees whose folders no longer exist.
    pub fn worktree_prune(&self, root: &Path) -> GitResult<()> {
        self.run(root, &["worktree", "prune"]).map(drop)
    }

    pub fn branch_delete(&self, root: &Path, branch: &str) -> GitResult<()> {
        self.run(root, &["branch", "-D", branch]).map(drop)
    }
}

fn parse_worktrees(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    for line in porcelain.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match (key, entries.last_mut()) {
            ("worktree", _) => entries.push(WorktreeEntry {
                path: normalize(Path::new(value)),
                branch: None,
                is_main: entries.is_empty(),
                bare: false,
                prunable: false,
            }),
            ("branch", Some(entry)) => {
                entry.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                );
            }
            ("bare", Some(entry)) => entry.bare = true,
            ("prunable", Some(entry)) => entry.prunable = true,
            _ => {}
        }
    }
    entries
}

/// Canonical form of a path for storing and comparing: symlinks resolved, and on Windows no
/// `\\?\` prefix and no forward slashes (git prints `C:/Users/...`).
pub fn normalize(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A `Git` that ignores the developer's own configuration, with a fixed identity.
    pub fn git() -> Git {
        let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        for (key, value) in [
            (
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.com"),
            ("GIT_COMMITTER_NAME", "Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.com"),
        ] {
            vars.insert(key.into(), value.into());
        }
        Git::new(&ShellEnv {
            vars,
            source: crate::env::EnvSource::Process,
            warning: None,
        })
        .expect("git is required to run the tests")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_directory_is_not_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(testing::git().repo_root(dir.path()).unwrap(), None);
    }

    #[test]
    fn the_root_is_found_from_a_subdirectory() {
        let git = testing::git();
        let dir = tempfile::tempdir().unwrap();
        git.init(dir.path()).unwrap();
        let nested = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(git.repo_root(&nested).unwrap(), Some(normalize(dir.path())));
    }

    #[test]
    fn head_goes_from_unborn_to_branch_to_detached() {
        let git = testing::git();
        let dir = tempfile::tempdir().unwrap();
        git.init(dir.path()).unwrap();
        git.run(dir.path(), &["checkout", "-b", "trunk"]).unwrap();

        assert_eq!(git.head(dir.path()).unwrap(), Head::Unborn("trunk".into()));
        assert!(!git.has_commits(dir.path()).unwrap());

        git.initial_commit(dir.path()).unwrap();
        assert_eq!(git.head(dir.path()).unwrap(), Head::Branch("trunk".into()));
        assert!(git.has_commits(dir.path()).unwrap());

        git.run(dir.path(), &["checkout", "--detach"]).unwrap();
        assert!(matches!(git.head(dir.path()).unwrap(), Head::Detached(sha) if sha.len() >= 7));
    }

    fn repo_with_commit() -> (Git, tempfile::TempDir) {
        let git = testing::git();
        let dir = tempfile::tempdir().unwrap();
        git.init(dir.path()).unwrap();
        git.run(dir.path(), &["checkout", "-b", "trunk"]).unwrap();
        git.initial_commit(dir.path()).unwrap();
        (git, dir)
    }

    #[test]
    fn worktrees_are_added_on_a_new_branch_and_removed_again() {
        let (git, repo) = repo_with_commit();
        let elsewhere = tempfile::tempdir().unwrap();
        let path = elsewhere.path().join("feature");

        git.worktree_add(repo.path(), &path, "ys/feature", "trunk")
            .unwrap();
        assert_eq!(git.head(&path).unwrap(), Head::Branch("ys/feature".into()));
        assert!(git.branch_exists(repo.path(), "ys/feature").unwrap());
        assert_eq!(git.branches(repo.path()).unwrap(), ["trunk", "ys/feature"]);

        git.worktree_remove(repo.path(), &path, false).unwrap();
        assert!(!path.exists());
        assert!(
            git.branch_exists(repo.path(), "ys/feature").unwrap(),
            "the branch outlives it"
        );
        git.branch_delete(repo.path(), "ys/feature").unwrap();
        assert!(!git.branch_exists(repo.path(), "ys/feature").unwrap());
    }

    #[test]
    fn worktrees_are_listed_with_their_branches_main_first() {
        let (git, repo) = repo_with_commit();
        let elsewhere = tempfile::tempdir().unwrap();
        let (a, b) = (elsewhere.path().join("a"), elsewhere.path().join("b"));
        git.worktree_add(repo.path(), &a, "ys/a", "trunk").unwrap();
        git.worktree_add(repo.path(), &b, "ys/b", "trunk").unwrap();
        git.run(&b, &["checkout", "--detach"]).unwrap();
        std::fs::remove_dir_all(&a).unwrap();

        let list = git.worktrees(repo.path()).unwrap();
        assert_eq!(list.len(), 3);
        assert!(list[0].is_main && list[0].branch.as_deref() == Some("trunk"));
        assert_eq!(list[0].path, normalize(repo.path()));
        assert!(list[1].prunable && list[1].branch.as_deref() == Some("ys/a"));
        assert!(!list[2].prunable && !list[2].is_main && list[2].branch.is_none());
        assert_eq!(list[2].path, normalize(&b));
    }

    #[test]
    fn an_existing_branch_can_be_checked_out_in_a_worktree_but_only_once() {
        let (git, repo) = repo_with_commit();
        let elsewhere = tempfile::tempdir().unwrap();
        git.run(repo.path(), &["branch", "feature"]).unwrap();

        git.worktree_add_existing(repo.path(), &elsewhere.path().join("one"), "feature")
            .unwrap();
        assert_eq!(
            git.head(&elsewhere.path().join("one")).unwrap(),
            Head::Branch("feature".into())
        );
        assert!(git
            .worktree_add_existing(repo.path(), &elsewhere.path().join("two"), "feature")
            .is_err());
    }

    #[test]
    fn a_dirty_worktree_is_only_removed_by_force() {
        let (git, repo) = repo_with_commit();
        let elsewhere = tempfile::tempdir().unwrap();
        let path = elsewhere.path().join("wip");
        git.worktree_add(repo.path(), &path, "ys/wip", "trunk")
            .unwrap();
        std::fs::write(path.join("unsaved.txt"), "work in progress").unwrap();

        assert!(git.worktree_remove(repo.path(), &path, false).is_err());
        assert!(path.join("unsaved.txt").exists());
        git.worktree_remove(repo.path(), &path, true).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn the_default_branch_is_the_checked_out_one_without_a_remote() {
        let (git, repo) = repo_with_commit();
        assert_eq!(
            git.default_branch(repo.path()).unwrap(),
            Some("trunk".into())
        );
        git.run(repo.path(), &["checkout", "--detach"]).unwrap();
        assert_eq!(git.default_branch(repo.path()).unwrap(), None);
    }

    #[test]
    fn the_default_branch_follows_the_remotes_head() {
        let (git, origin) = repo_with_commit();
        let dir = tempfile::tempdir().unwrap();
        let clone = dir.path().join("clone");
        git.run(
            dir.path(),
            &["clone", &origin.path().to_string_lossy(), "clone"],
        )
        .unwrap();
        git.run(&clone, &["checkout", "-b", "side-quest"]).unwrap();
        assert_eq!(git.default_branch(&clone).unwrap(), Some("trunk".into()));
    }

    #[test]
    fn failures_carry_gits_own_words() {
        let dir = tempfile::tempdir().unwrap();
        match testing::git().run(dir.path(), &["log"]) {
            Err(GitError::Failed { command, stderr }) => {
                assert_eq!(command, "log");
                assert!(stderr.contains("not a git repository"), "{stderr}");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}

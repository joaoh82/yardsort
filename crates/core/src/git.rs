//! Git, by way of the `git` CLI: the reference implementation, and the only one that respects
//! the user's config, hooks and credentials. Everything goes through [`Git::run`].

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::env::ShellEnv;
use crate::program::Program;

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

/// One commit, as much of it as a pull request needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub subject: String,
    /// Everything under the subject, with the blank line between them dropped. Often empty.
    pub body: String,
}

impl Commit {
    /// Split a message the way git itself reads one: first line, then the rest.
    fn from_message(message: &str) -> Self {
        let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
        Self {
            subject: subject.trim().to_owned(),
            body: body.trim().to_owned(),
        }
    }
}

pub struct Git {
    program: Program,
}

impl Git {
    /// Find git on the *user's* `PATH` and run it with the user's environment.
    pub fn new(env: &ShellEnv) -> GitResult<Self> {
        let program = Program::find(env, "git").ok_or(GitError::NotInstalled)?;
        Ok(Self { program })
    }

    pub fn run(&self, cwd: &Path, args: &[&str]) -> GitResult<String> {
        let bytes = self.run_bytes(cwd, args)?;
        Ok(String::from_utf8_lossy(&bytes).trim_end().to_owned())
    }

    /// Like [`Self::run`], with stdout exactly as git wrote it: file contents, `-z` lists.
    pub fn run_bytes(&self, cwd: &Path, args: &[&str]) -> GitResult<Vec<u8>> {
        let output = self
            .program
            .command(cwd)
            .args(args)
            // Never block on a credential or passphrase prompt nobody can see.
            .env("GIT_TERMINAL_PROMPT", "0")
            // Keep messages in English: a few callers have to recognise them.
            .env("LC_ALL", "C")
            .output()?;
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

/// Sending work outwards: a commit, a remote, a push.
impl Git {
    /// Stage everything git is willing to track and commit it, returning the new short id.
    ///
    /// Everything, because the panel offers no way to leave a file out: a partial commit the
    /// user did not ask for is worse than one more file than they expected. The message crosses
    /// as one argument, so nothing in it is ever read as a flag or by a shell.
    pub fn commit_all(&self, root: &Path, message: &str) -> GitResult<String> {
        self.run(root, &["add", "--all"])?;
        self.run(root, &["commit", "--message", message])?;
        self.run(root, &["rev-parse", "--short", "HEAD"])
    }

    /// The identity a commit would be signed with — `Ada Lovelace <ada@example.com>` — or
    /// `None` when git cannot work one out.
    ///
    /// `git var` rather than `git config user.name`, because the environment can supply an
    /// identity that the config does not: asking the config would report "no identity" for a
    /// repository that commits perfectly well. This is the one failure worth catching before
    /// anything is staged, since it is the only one the user fixes outside Yardsort.
    pub fn identity(&self, root: &Path) -> GitResult<Option<String>> {
        match self.run(root, &["var", "GIT_COMMITTER_IDENT"]) {
            // `Name <email> 1727090000 +0000` — the timestamp is git's, not the identity.
            Ok(ident) => Ok(ident
                .rsplit_once('>')
                .map(|(who, _)| format!("{who}>"))
                .filter(|who| !who.starts_with('<'))),
            Err(GitError::Failed { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Remote names, in git's own order.
    pub fn remotes(&self, root: &Path) -> GitResult<Vec<String>> {
        Ok(self
            .run(root, &["remote"])?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// The remote to push to: `origin` when it exists, otherwise the only one, otherwise none.
    pub fn push_remote(&self, root: &Path) -> GitResult<Option<String>> {
        let remotes = self.remotes(root)?;
        Ok(remotes
            .iter()
            .find(|name| *name == "origin")
            .or_else(|| remotes.first())
            .cloned())
    }

    /// Where a remote points. `None` if there is no such remote.
    pub fn remote_url(&self, root: &Path, remote: &str) -> GitResult<Option<String>> {
        match self.run(root, &["remote", "get-url", remote]) {
            Ok(url) => Ok(Some(url)),
            Err(GitError::Failed { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// What `HEAD` tracks, e.g. `origin/main`, or `None` when it tracks nothing yet.
    pub fn upstream(&self, root: &Path) -> GitResult<Option<String>> {
        match self.run(
            root,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        ) {
            Ok(name) if !name.is_empty() => Ok(Some(name)),
            Ok(_) => Ok(None),
            Err(GitError::Failed { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// How far `HEAD` is ahead of and behind `reference`. `None` when the two have no common
    /// history to count across — a remote branch we have never fetched, most often.
    pub fn ahead_behind(&self, root: &Path, reference: &str) -> GitResult<Option<(u32, u32)>> {
        let range = format!("{reference}...HEAD");
        let out = match self.run(root, &["rev-list", "--left-right", "--count", &range]) {
            Ok(out) => out,
            Err(GitError::Failed { .. }) => return Ok(None),
            Err(other) => return Err(other),
        };
        let mut counts = out.split_whitespace().map(|n| n.parse::<u32>().ok());
        match (counts.next().flatten(), counts.next().flatten()) {
            // git counts the left side first, and the left side is the reference.
            (Some(behind), Some(ahead)) => Ok(Some((ahead, behind))),
            _ => Ok(None),
        }
    }

    /// Push `branch` to `remote` and make it the branch's upstream.
    ///
    /// Never forced: a push Yardsort makes can only ever add to what the remote has.
    pub fn push(&self, root: &Path, remote: &str, branch: &str) -> GitResult<()> {
        self.run(root, &["push", "--set-upstream", remote, branch])
            .map(drop)
    }

    /// The commits in `reference..HEAD`, newest first.
    ///
    /// `%B` is the message exactly as it was written, subject and body together, and the
    /// commits are separated by NUL — a message may contain any line a friendlier separator
    /// could have used.
    pub fn commits_since(&self, root: &Path, reference: &str) -> GitResult<Vec<Commit>> {
        let range = format!("{reference}..HEAD");
        let out = match self.run_bytes(root, &["log", "--format=%B%x00", &range]) {
            Ok(out) => out,
            Err(GitError::Failed { .. }) => return Ok(vec![]),
            Err(other) => return Err(other),
        };
        Ok(String::from_utf8_lossy(&out)
            .split('\0')
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(Commit::from_message)
            .collect())
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

/// Helpers for tests, in this crate and in the app's. Behind a feature so they never reach a
/// release build: `yardsort-core = { features = ["testing"] }` in dev-dependencies.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
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

    /// A repository with a real remote: a bare one next door, so pushes go somewhere and
    /// `@{u}` means what it means anywhere else.
    fn repo_with_remote() -> (Git, tempfile::TempDir, tempfile::TempDir) {
        let (git, repo) = repo_with_commit();
        let remote = tempfile::tempdir().unwrap();
        git.run(remote.path(), &["init", "--bare"]).unwrap();
        let url = remote.path().to_string_lossy().to_string();
        git.run(repo.path(), &["remote", "add", "origin", &url])
            .unwrap();
        (git, repo, remote)
    }

    #[test]
    fn committing_takes_everything_including_untracked_files() {
        let (git, repo) = repo_with_commit();
        std::fs::write(repo.path().join("tracked.txt"), "one").unwrap();
        git.run(repo.path(), &["add", "tracked.txt"]).unwrap();
        git.run(repo.path(), &["commit", "-m", "first"]).unwrap();
        std::fs::write(repo.path().join("tracked.txt"), "two").unwrap();
        std::fs::write(repo.path().join("brand-new.txt"), "hello").unwrap();

        let id = git.commit_all(repo.path(), "Do the thing").unwrap();
        assert!(id.len() >= 7, "a short id, got {id:?}");
        assert_eq!(
            git.run(repo.path(), &["status", "--porcelain"]).unwrap(),
            "",
            "nothing is left uncommitted"
        );
        let files = git
            .run(repo.path(), &["show", "--name-only", "--format=%s", "HEAD"])
            .unwrap();
        assert!(files.contains("Do the thing"));
        assert!(files.contains("brand-new.txt"));
    }

    #[test]
    fn a_message_is_never_read_as_a_flag() {
        let (git, repo) = repo_with_commit();
        std::fs::write(repo.path().join("a.txt"), "x").unwrap();
        // Passed as argv, so a message that looks like an option is just a message.
        git.commit_all(repo.path(), "--amend is not an option here")
            .unwrap();
        assert_eq!(
            git.run(repo.path(), &["log", "--format=%s", "-1"]).unwrap(),
            "--amend is not an option here"
        );
        assert_eq!(
            git.run(repo.path(), &["rev-list", "--count", "HEAD"])
                .unwrap(),
            "2",
            "it is a new commit, not an amended one"
        );
    }

    #[test]
    fn committing_nothing_fails_and_leaves_the_history_alone() {
        let (git, repo) = repo_with_commit();
        let before = git.run(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        assert!(git.commit_all(repo.path(), "nothing to say").is_err());
        assert_eq!(
            git.run(repo.path(), &["rev-parse", "HEAD"]).unwrap(),
            before
        );
    }

    #[test]
    fn a_commits_subject_and_body_come_back_apart() {
        let (git, repo, _remote) = repo_with_remote();
        git.push(repo.path(), "origin", "trunk").unwrap();
        std::fs::write(repo.path().join("a.txt"), "x").unwrap();
        git.commit_all(
            repo.path(),
            "Fix the login redirect\n\nThe cookie was set on the wrong domain,\nso the session never came back.\n",
        )
        .unwrap();
        std::fs::write(repo.path().join("b.txt"), "y").unwrap();
        git.commit_all(repo.path(), "Tidy up").unwrap();

        let unpushed = git.commits_since(repo.path(), "origin/trunk").unwrap();
        assert_eq!(unpushed.len(), 2);
        // Newest first, as git prints them.
        assert_eq!(unpushed[0].subject, "Tidy up");
        assert_eq!(unpushed[1].subject, "Fix the login redirect");
        assert_eq!(
            unpushed[1].body,
            "The cookie was set on the wrong domain,\nso the session never came back."
        );
    }

    #[test]
    fn a_message_containing_blank_lines_is_still_one_commit() {
        let (git, repo, _remote) = repo_with_remote();
        git.push(repo.path(), "origin", "trunk").unwrap();
        std::fs::write(repo.path().join("a.txt"), "x").unwrap();
        git.commit_all(repo.path(), "Subject\n\nOne paragraph.\n\nAnd another.")
            .unwrap();

        let unpushed = git.commits_since(repo.path(), "origin/trunk").unwrap();
        assert_eq!(unpushed.len(), 1, "blank lines do not split commits");
        assert_eq!(unpushed[0].body, "One paragraph.\n\nAnd another.");
    }

    #[test]
    fn an_identity_is_found_when_there_is_one() {
        let (git, repo) = repo_with_commit();
        let who = git.identity(repo.path()).unwrap().expect("an identity");
        assert_eq!(who, "Test <test@example.com>");
    }

    #[test]
    fn the_push_remote_is_origin_when_there_is_one() {
        let (git, repo) = repo_with_commit();
        assert_eq!(git.remotes(repo.path()).unwrap(), Vec::<String>::new());
        assert_eq!(git.push_remote(repo.path()).unwrap(), None);

        git.run(
            repo.path(),
            &["remote", "add", "upstream", "https://example.com/a/b.git"],
        )
        .unwrap();
        assert_eq!(
            git.push_remote(repo.path()).unwrap().as_deref(),
            Some("upstream"),
            "the only remote will do"
        );

        git.run(
            repo.path(),
            &["remote", "add", "origin", "https://example.com/c/d.git"],
        )
        .unwrap();
        assert_eq!(
            git.push_remote(repo.path()).unwrap().as_deref(),
            Some("origin"),
            "but origin wins"
        );
        assert_eq!(
            git.remote_url(repo.path(), "origin").unwrap().as_deref(),
            Some("https://example.com/c/d.git")
        );
        assert_eq!(git.remote_url(repo.path(), "nope").unwrap(), None);
    }

    #[test]
    fn pushing_sets_the_upstream_and_the_counts_follow_it() {
        let (git, repo, _remote) = repo_with_remote();
        assert_eq!(git.upstream(repo.path()).unwrap(), None);

        git.push(repo.path(), "origin", "trunk").unwrap();
        assert_eq!(
            git.upstream(repo.path()).unwrap().as_deref(),
            Some("origin/trunk")
        );
        assert_eq!(
            git.ahead_behind(repo.path(), "origin/trunk").unwrap(),
            Some((0, 0))
        );

        std::fs::write(repo.path().join("after.txt"), "more").unwrap();
        git.commit_all(repo.path(), "More").unwrap();
        assert_eq!(
            git.ahead_behind(repo.path(), "origin/trunk").unwrap(),
            Some((1, 0)),
            "one commit the remote has not seen"
        );
        let unpushed = git.commits_since(repo.path(), "origin/trunk").unwrap();
        assert_eq!(unpushed.len(), 1);
        assert_eq!(unpushed[0].subject, "More");
        assert_eq!(unpushed[0].body, "", "a one-line message has no body");

        git.push(repo.path(), "origin", "trunk").unwrap();
        assert_eq!(
            git.ahead_behind(repo.path(), "origin/trunk").unwrap(),
            Some((0, 0))
        );
        assert!(git
            .commits_since(repo.path(), "origin/trunk")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn counting_against_a_reference_we_do_not_have_says_so() {
        let (git, repo) = repo_with_commit();
        assert_eq!(
            git.ahead_behind(repo.path(), "origin/never-fetched")
                .unwrap(),
            None
        );
        assert!(git
            .commits_since(repo.path(), "origin/never-fetched")
            .unwrap()
            .is_empty());
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

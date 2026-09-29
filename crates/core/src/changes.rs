//! What changed in a workspace, and the file contents needed to show it. In the core so the
//! app's panel and the `ys` command line read the same answer.
//!
//! Two scopes, because they answer different questions:
//! - **uncommitted** — the working tree against `HEAD`: what the agent is doing right now;
//! - **committed** — `HEAD` against the point where this branch left its base: what the
//!   workspace has produced so far.
//!
//! Diffs are returned as the two versions of a file rather than as a patch: the viewer computes
//! the diff itself and can then show as much context as it likes.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::git::{Git, GitError, Head};

/// Files larger than this are not loaded into the viewer.
pub const MAX_VIEW_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// New and not yet known to git.
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Path relative to the workspace root, with `/` separators.
    pub path: String,
    /// Where a renamed file used to live.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    /// Lines added / removed. `None` for binary and untracked files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Uncommitted,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub uncommitted: Vec<FileChange>,
    pub committed: Vec<FileChange>,
    /// The branch `committed` is measured against; `None` when there is nothing to compare with
    /// (the workspace *is* the base branch, or HEAD is detached).
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Content {
    /// The file does not exist on this side (added, or deleted).
    Absent,
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String,
    },
    NotEditable {
        reason: String,
    },
    Binary,
    TooLarge {
        bytes: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub old: Content,
    pub new: Content,
}

pub struct Changes<'a> {
    pub git: &'a Git,
    pub root: &'a Path,
    /// The branch this workspace started from, if Yardsort knows.
    pub base_branch: Option<&'a str>,
}

impl Changes<'_> {
    pub fn list(&self) -> IpcResult<ChangeSet> {
        let (base, fork_point) = self.fork_point()?;
        let committed = match &fork_point {
            Some(fork) => {
                let range = format!("{fork}..HEAD");
                let names = self
                    .git
                    .run_bytes(self.root, &["diff", "--name-status", "-z", "-M", &range])?;
                let stats = self
                    .git
                    .run_bytes(self.root, &["diff", "--numstat", "-z", "-M", &range])?;
                with_stats(parse_name_status(&names), &parse_numstat(&stats))
            }
            None => vec![],
        };

        let status = self.git.run_bytes(
            self.root,
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        )?;
        let uncommitted = if self.git.has_commits(self.root)? {
            let stats = self
                .git
                .run_bytes(self.root, &["diff", "--numstat", "-z", "-M", "HEAD"])?;
            with_stats(parse_status(&status), &parse_numstat(&stats))
        } else {
            parse_status(&status)
        };

        Ok(ChangeSet {
            uncommitted,
            committed,
            base: fork_point.and(base),
        })
    }

    pub fn diff(&self, path: &str, old_path: Option<&str>, scope: Scope) -> IpcResult<FileDiff> {
        let path = safe_relative(path)?;
        let old_path = old_path
            .map(safe_relative)
            .transpose()?
            .unwrap_or_else(|| path.clone());
        match scope {
            Scope::Uncommitted => Ok(FileDiff {
                old: self.at_revision("HEAD", &old_path)?,
                new: read_working_file(self.root, &path)?,
            }),
            Scope::Committed => {
                let (_, fork) = self.fork_point()?;
                let fork = fork.ok_or_else(|| {
                    IpcError::new(
                        "no_base",
                        "This workspace has no base branch to compare with.",
                    )
                })?;
                Ok(FileDiff {
                    old: self.at_revision(&fork, &old_path)?,
                    new: self.at_revision("HEAD", &path)?,
                })
            }
        }
    }

    /// One file's change in `scope` as a unified diff, the way git prints it — or, for a file git
    /// does not know yet, its whole text with every line added. `None` for binary files and
    /// diffs too large to be worth reading.
    pub fn patch(&self, change: &FileChange, scope: Scope) -> IpcResult<Option<String>> {
        let path = safe_relative(&change.path)?;
        let brand_new = change.kind == ChangeKind::Untracked
            || (scope == Scope::Uncommitted && !self.git.has_commits(self.root)?);
        if brand_new {
            return Ok(match read_working_file(self.root, &path)? {
                Content::Text { text } => {
                    Some(text.lines().map(|line| format!("+{line}\n")).collect())
                }
                _ => None,
            });
        }
        let range = match scope {
            Scope::Uncommitted => "HEAD".to_owned(),
            Scope::Committed => match self.fork_point()?.1 {
                Some(fork) => format!("{fork}..HEAD"),
                None => return Ok(None),
            },
        };
        let old_path = change.old_path.as_deref().map(safe_relative).transpose()?;
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M", &range, "--"];
        args.extend(old_path.as_deref());
        args.push(&path);
        let bytes = self.git.run_bytes(self.root, &args)?;
        if bytes.len() > MAX_VIEW_BYTES {
            return Ok(None);
        }
        let patch = text(&bytes);
        let binary = !patch.contains("\n@@") && patch.contains("Binary files");
        Ok((!binary && !patch.trim().is_empty()).then_some(patch))
    }

    /// Every changed file's patch in one string, for handing a whole change to a model.
    ///
    /// Built from the per-file patches rather than one `git diff`, so that untracked files —
    /// which `git diff` cannot see and which are often the whole point of a new workspace —
    /// are in it. Each is headed by its path, because a patch with no `diff --git` line above
    /// it is a patch nobody can place.
    pub fn combined_patch(&self, scope: Scope) -> IpcResult<String> {
        let changes = self.list()?;
        let files = match scope {
            Scope::Uncommitted => changes.uncommitted,
            Scope::Committed => changes.committed,
        };
        let mut combined = String::new();
        for change in &files {
            if combined.len() >= crate::draft::MAX_DIFF_CHARS {
                combined.push_str("\n… more files changed than fit here.\n");
                break;
            }
            let Some(patch) = self.patch(change, scope)? else {
                // Binary, too large, or nothing to show: name it so the model knows it happened.
                combined.push_str(&format!(
                    "--- {} ({:?}, not shown)\n",
                    change.path, change.kind
                ));
                continue;
            };
            combined.push_str(&format!("--- {}\n", change.path));
            combined.push_str(&patch);
            if !patch.ends_with('\n') {
                combined.push('\n');
            }
        }
        Ok(combined)
    }

    /// The base branch and the commit where this branch left it.
    fn fork_point(&self) -> IpcResult<(Option<String>, Option<String>)> {
        let current = match self.git.head(self.root)? {
            Head::Branch(name) => name,
            Head::Unborn(_) | Head::Detached(_) => return Ok((None, None)),
        };
        let base = match self.base_branch {
            Some(base) => Some(base.to_owned()),
            None => self.git.default_branch(self.root)?,
        };
        let Some(base) = base.filter(|base| *base != current) else {
            return Ok((None, None));
        };
        match self.git.run(self.root, &["merge-base", "HEAD", &base]) {
            Ok(fork) => Ok((Some(base), Some(fork))),
            // The base branch is gone, or the histories are unrelated: nothing to compare with.
            Err(GitError::Failed { .. }) => Ok((None, None)),
            Err(other) => Err(other.into()),
        }
    }

    fn at_revision(&self, revision: &str, path: &str) -> IpcResult<Content> {
        let spec = format!("{revision}:{path}");
        match self.git.run_bytes(self.root, &["cat-file", "-s", &spec]) {
            Ok(size) => {
                let bytes: usize = String::from_utf8_lossy(&size).trim().parse().unwrap_or(0);
                if bytes > MAX_VIEW_BYTES {
                    return Ok(Content::TooLarge {
                        bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
                    });
                }
                Ok(classify(self.git.run_bytes(self.root, &["show", &spec])?))
            }
            Err(GitError::Failed { .. }) => Ok(Content::Absent),
            Err(other) => Err(other.into()),
        }
    }
}

/// A file as it is on disk right now.
pub fn read_working_file(root: &Path, relative: &str) -> IpcResult<Content> {
    let path = resolve_inside(root, relative)?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Content::Absent),
        Err(error) => {
            return Err(IpcError::new(
                "io",
                format!("Cannot read {relative}: {error}"),
            ))
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(Content::NotEditable {
            reason: "Symbolic links cannot be edited here. Open the target file instead.".into(),
        });
    }
    if metadata.is_dir() {
        return Ok(Content::Absent);
    }
    let bytes = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if bytes > MAX_VIEW_BYTES {
        return Ok(Content::TooLarge {
            bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
        });
    }
    std::fs::read(&path)
        .map(classify)
        .map_err(|error| IpcError::new("io", format!("Cannot read {relative}: {error}")))
}

fn classify(bytes: Vec<u8>) -> Content {
    use base64::Engine;
    // Text can start with an image signature (for example "BM25 notes").
    if !bytes.iter().take(8000).any(|&b| b == 0) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            return Content::Text {
                text: text.to_owned(),
            };
        }
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"BM")
        && bytes.len() >= 14
        && bytes[6..10] == [0; 4]
        && u32::from_le_bytes(bytes[10..14].try_into().unwrap()) >= 14
        && (u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize) < bytes.len()
    {
        Some("image/bmp")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else {
        None
    };
    if let Some(mime) = mime {
        return Content::Image {
            mime: mime.into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
    }

    Content::Binary
}

/// Save only the UTF-8 version the user actually opened. Never replace an agent's newer edits.
pub fn save_working_file(root: &Path, relative: &str, expected: &str, text: &str) -> IpcResult<()> {
    save_working_file_before_replace(root, relative, expected, text, || {})
}

fn save_working_file_before_replace(
    root: &Path,
    relative: &str,
    expected: &str,
    text: &str,
    before_replace: impl FnOnce(),
) -> IpcResult<()> {
    use std::io::Write;
    // Two app commands saving the same baseline must not both pass the comparison.
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _saving = SAVING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if text.len() > MAX_VIEW_BYTES || text.contains('\0') {
        return Err(IpcError::new(
            "not_editable",
            "Only text files up to 1 MiB can be saved.",
        ));
    }
    let path = editable_path(root, relative)?;
    let io = |error| IpcError::new("io", format!("Cannot save {relative}: {error}"));
    let metadata = std::fs::metadata(&path).map_err(io)?;
    if metadata.permissions().readonly() {
        return Err(IpcError::new(
            "not_editable",
            "This file is read-only. Make it writable before saving; your draft is kept.",
        ));
    }
    if !metadata.is_file() || metadata.len() > MAX_VIEW_BYTES as u64 {
        return Err(IpcError::new(
            "not_editable",
            "Only text files up to 1 MiB can be saved.",
        ));
    }
    let current = std::fs::read(&path).map_err(io)?;
    if current != expected.as_bytes() || !matches!(classify(current), Content::Text { .. }) {
        return Err(IpcError::new("file_changed", "This file changed on disk. Copy your edits, then discard the draft to reload before saving."));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(io)?;
    temporary.write_all(text.as_bytes()).map_err(io)?;
    temporary.as_file().sync_all().map_err(io)?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(io)?;
    before_replace();
    // Check again after preparing the replacement; failed writes leave the original untouched.
    if editable_path(root, relative)? != path {
        return Err(IpcError::new(
            "bad_path",
            "The file's location changed while saving. Your draft is kept.",
        ));
    }
    if std::fs::read(&path).map_err(io)? != expected.as_bytes() {
        return Err(IpcError::new(
            "file_changed",
            "This file changed on disk. Your draft has been kept.",
        ));
    }
    if editable_path(root, relative)? != path {
        return Err(IpcError::new(
            "bad_path",
            "The file's location changed while saving. Your draft is kept.",
        ));
    }
    temporary.persist(&path).map_err(|error| io(error.error))?;
    Ok(())
}

/// Resolve again at replacement time, rejecting symlinks and git internals each time.
fn editable_path(root: &Path, relative: &str) -> IpcResult<PathBuf> {
    let path = resolve_inside(root, relative)?;
    // Do not replace symlinks, including dangling ones, or write into git's internal files.
    if path
        .strip_prefix(root)
        .unwrap_or(&path)
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
        || std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(IpcError::new(
            "not_editable",
            "Git internals and symbolic links cannot be edited here.",
        ));
    }
    let io = |error| IpcError::new("io", format!("Cannot save {relative}: {error}"));
    let canonical = dunce::canonicalize(&path).map_err(io)?;
    let canonical_root = dunce::canonicalize(root).map_err(io)?;
    let inside = canonical
        .strip_prefix(&canonical_root)
        .map_err(|_| IpcError::new("bad_path", "The file is outside the workspace."))?;
    if inside
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
    {
        return Err(IpcError::new(
            "not_editable",
            "Git internals cannot be edited here.",
        ));
    }
    Ok(canonical)
}

/// A path from a client is untrusted: it must be relative and must not climb out.
pub fn safe_relative(path: &str) -> IpcResult<String> {
    let escapes = Path::new(path)
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir));
    if path.is_empty() || escapes {
        return Err(IpcError::new(
            "bad_path",
            format!("\"{path}\" is not a path inside the workspace."),
        ));
    }
    Ok(path.replace('\\', "/"))
}

/// `root/relative`, refusing anything that resolves outside `root` — including via symlinks.
pub fn resolve_inside(root: &Path, relative: &str) -> IpcResult<PathBuf> {
    let joined = root.join(safe_relative(relative)?);
    let outside = || {
        IpcError::new(
            "bad_path",
            format!("\"{relative}\" is outside the workspace."),
        )
    };
    // The file itself may not exist (deleted); its closest existing ancestor must be inside.
    let mut probe = joined.as_path();
    let existing = loop {
        if probe.exists() {
            break probe;
        }
        probe = probe.parent().ok_or_else(outside)?;
    };
    let real_root = dunce::canonicalize(root).map_err(|_| outside())?;
    let real = dunce::canonicalize(existing).map_err(|_| outside())?;
    if real.starts_with(&real_root) {
        Ok(joined)
    } else {
        Err(outside())
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parse `git diff --name-status -z`: `M\0path\0`, and for renames `R100\0old\0new\0`.
fn parse_name_status(output: &[u8]) -> Vec<FileChange> {
    let mut fields = output.split(|&b| b == 0).filter(|f| !f.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let letter = status.first().copied().unwrap_or(b'M');
        let Some(first) = fields.next() else { break };
        let (path, old_path) = if matches!(letter, b'R' | b'C') {
            let Some(new) = fields.next() else { break };
            (text(new), Some(text(first)))
        } else {
            (text(first), None)
        };
        changes.push(FileChange {
            path,
            old_path: old_path.filter(|_| letter == b'R'),
            kind: match letter {
                b'A' | b'C' => ChangeKind::Added,
                b'D' => ChangeKind::Deleted,
                b'R' => ChangeKind::Renamed,
                b'U' => ChangeKind::Conflicted,
                _ => ChangeKind::Modified,
            },
            additions: None,
            deletions: None,
        });
    }
    changes
}

/// Parse `git status --porcelain=v2 -z`.
fn parse_status(output: &[u8]) -> Vec<FileChange> {
    let mut records = output.split(|&b| b == 0).filter(|r| !r.is_empty());
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        let record = text(record);
        let change = |path: &str, old_path: Option<String>, kind| FileChange {
            path: path.to_owned(),
            old_path,
            kind,
            additions: None,
            deletions: None,
        };
        match record.split_once(' ') {
            // 1 XY sub mH mI mW hH hI path
            Some(("1", rest)) => {
                let mut parts = rest.splitn(8, ' ');
                let xy = parts.next().unwrap_or("..");
                if let Some(path) = parts.nth(6) {
                    changes.push(change(path, None, kind_from_xy(xy)));
                }
            }
            // 2 XY sub mH mI mW hH hI Xscore path  — and the old path is the next record
            Some(("2", rest)) => {
                let mut parts = rest.splitn(9, ' ');
                let _xy = parts.next();
                let old = records.next().map(text);
                if let Some(path) = parts.nth(7) {
                    changes.push(change(path, old, ChangeKind::Renamed));
                }
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            Some(("u", rest)) => {
                if let Some(path) = rest.splitn(10, ' ').nth(9) {
                    changes.push(change(path, None, ChangeKind::Conflicted));
                }
            }
            Some(("?", path)) => changes.push(change(path, None, ChangeKind::Untracked)),
            _ => {} // "!" ignored entries and "#" headers
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

/// `XY` is index + worktree state; whichever says more wins.
fn kind_from_xy(xy: &str) -> ChangeKind {
    if xy.contains('D') {
        ChangeKind::Deleted
    } else if xy.contains('A') {
        ChangeKind::Added
    } else {
        ChangeKind::Modified
    }
}

/// Parse `git diff --numstat -z` into `(path, additions, deletions)`; binary files have `-`.
fn parse_numstat(output: &[u8]) -> Vec<(String, Option<u32>, Option<u32>)> {
    let mut fields = output.split(|&b| b == 0);
    let mut stats = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let line = text(field);
        let mut columns = line.splitn(3, '\t');
        let (Some(added), Some(deleted)) = (columns.next(), columns.next()) else {
            continue;
        };
        let path = match columns.next() {
            Some(path) if !path.is_empty() => path.to_owned(),
            // A rename: the numbers stand alone, then the old and the new path follow.
            _ => {
                let _old = fields.next();
                match fields.next() {
                    Some(new) => text(new),
                    None => break,
                }
            }
        };
        stats.push((path, added.parse().ok(), deleted.parse().ok()));
    }
    stats
}

fn with_stats(
    mut changes: Vec<FileChange>,
    stats: &[(String, Option<u32>, Option<u32>)],
) -> Vec<FileChange> {
    for change in &mut changes {
        if let Some((_, added, deleted)) = stats.iter().find(|(path, _, _)| *path == change.path) {
            change.additions = *added;
            change.deletions = *deleted;
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::git;

    struct Repo {
        git: Git,
        dir: tempfile::TempDir,
    }

    impl Repo {
        /// A repository on `main` with one committed file.
        fn new() -> Self {
            let repo = Self {
                git: git(),
                dir: tempfile::tempdir().unwrap(),
            };
            repo.git.init(repo.path()).unwrap();
            repo.run(&["checkout", "-q", "-b", "main"]);
            repo.write("README.md", "# project\n\nline two\n");
            repo.commit("initial");
            repo
        }
        fn path(&self) -> &Path {
            self.dir.path()
        }
        fn run(&self, args: &[&str]) {
            self.git.run(self.path(), args).unwrap();
        }
        fn write(&self, file: &str, contents: &str) {
            let path = self.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        fn commit(&self, message: &str) {
            self.run(&["add", "-A"]);
            self.run(&["commit", "-q", "-m", message]);
        }
        fn changes(&self, base: Option<&'static str>) -> ChangeSet {
            Changes {
                git: &self.git,
                root: self.path(),
                base_branch: base,
            }
            .list()
            .unwrap()
        }
        fn diff(&self, path: &str, old: Option<&str>, scope: Scope) -> FileDiff {
            Changes {
                git: &self.git,
                root: self.path(),
                base_branch: Some("main"),
            }
            .diff(path, old, scope)
            .unwrap()
        }
    }

    fn summary(changes: &[FileChange]) -> Vec<(String, ChangeKind, Option<u32>, Option<u32>)> {
        changes
            .iter()
            .map(|c| (c.path.clone(), c.kind, c.additions, c.deletions))
            .collect()
    }

    fn text_of(content: &Content) -> &str {
        match content {
            Content::Text { text } => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn uncommitted_work_is_listed_with_kinds_and_line_counts() {
        let repo = Repo::new();
        repo.write("README.md", "# project\n\nline two changed\nline three\n");
        repo.write("src/new file.rs", "fn main() {}\n");
        repo.write("staged.txt", "a\nb\n");
        repo.run(&["add", "staged.txt"]);

        let set = repo.changes(None);
        assert_eq!(
            summary(&set.uncommitted),
            [
                ("README.md".into(), ChangeKind::Modified, Some(2), Some(1)),
                ("src/new file.rs".into(), ChangeKind::Untracked, None, None),
                ("staged.txt".into(), ChangeKind::Added, Some(2), Some(0)),
            ]
        );
        assert!(
            set.committed.is_empty() && set.base.is_none(),
            "main has nothing to compare with"
        );
    }

    #[test]
    fn a_branch_shows_what_it_committed_since_leaving_its_base() {
        let repo = Repo::new();
        repo.write("doomed.txt", "bye\n");
        repo.write(
            "old name.txt",
            "same content, long enough to be recognised as a rename\n",
        );
        repo.commit("more on main");
        repo.run(&["checkout", "-q", "-b", "ys/feature"]);
        repo.write("feature.rs", "one\ntwo\n");
        std::fs::remove_file(repo.path().join("doomed.txt")).unwrap();
        repo.run(&["mv", "old name.txt", "new name.txt"]);
        repo.commit("the feature");
        // Main moving on afterwards must not show up as this branch's work.
        repo.run(&["checkout", "-q", "main"]);
        repo.write("unrelated.txt", "main only\n");
        repo.commit("main moves on");
        repo.run(&["checkout", "-q", "ys/feature"]);

        let set = repo.changes(Some("main"));
        assert_eq!(set.base.as_deref(), Some("main"));
        assert_eq!(
            summary(&set.committed),
            [
                ("doomed.txt".into(), ChangeKind::Deleted, Some(0), Some(1)),
                ("feature.rs".into(), ChangeKind::Added, Some(2), Some(0)),
                ("new name.txt".into(), ChangeKind::Renamed, Some(0), Some(0)),
            ]
        );
        assert_eq!(set.committed[2].old_path.as_deref(), Some("old name.txt"));
        assert!(set.uncommitted.is_empty());

        let fallback = repo.changes(None);
        assert_eq!(
            fallback.base.as_deref(),
            None,
            "no remote and not on it: nothing to guess from"
        );
    }

    #[test]
    fn patches_are_unified_diffs_per_file_and_scope() {
        let repo = Repo::new();
        repo.run(&["checkout", "-q", "-b", "ys/work"]);
        repo.write("README.md", "# project\n\ncommitted edit\n");
        repo.write("logo.png", "\0\x01binary");
        repo.commit("edit");
        repo.write("README.md", "# project\n\nuncommitted edit\n");
        repo.write("notes/new.txt", "hello\nworld\n");
        let changes = Changes {
            git: &repo.git,
            root: repo.path(),
            base_branch: Some("main"),
        };
        let set = changes.list().unwrap();
        let find =
            |list: &[FileChange], path: &str| list.iter().find(|c| c.path == path).unwrap().clone();

        let committed = changes
            .patch(&find(&set.committed, "README.md"), Scope::Committed)
            .unwrap()
            .unwrap();
        assert!(
            committed.contains("-line two\n+committed edit\n"),
            "{committed}"
        );
        let uncommitted = changes
            .patch(&find(&set.uncommitted, "README.md"), Scope::Uncommitted)
            .unwrap()
            .unwrap();
        assert!(
            uncommitted.contains("-committed edit\n+uncommitted edit\n"),
            "{uncommitted}"
        );
        assert_eq!(
            changes
                .patch(&find(&set.uncommitted, "notes/new.txt"), Scope::Uncommitted)
                .unwrap()
                .as_deref(),
            Some("+hello\n+world\n")
        );
        assert_eq!(
            changes
                .patch(&find(&set.committed, "logo.png"), Scope::Committed)
                .unwrap(),
            None,
            "binary files have no patch"
        );
    }

    #[test]
    fn diffs_return_both_sides_of_a_file() {
        let repo = Repo::new();
        repo.run(&["checkout", "-q", "-b", "ys/work"]);
        repo.write("README.md", "# project\n\ncommitted edit\n");
        repo.commit("edit");
        repo.write("README.md", "# project\n\nuncommitted edit\n");
        repo.write("brand new.txt", "hello\n");

        let uncommitted = repo.diff("README.md", None, Scope::Uncommitted);
        assert_eq!(text_of(&uncommitted.old), "# project\n\ncommitted edit\n");
        assert_eq!(text_of(&uncommitted.new), "# project\n\nuncommitted edit\n");

        let committed = repo.diff("README.md", None, Scope::Committed);
        assert_eq!(text_of(&committed.old), "# project\n\nline two\n");
        assert_eq!(text_of(&committed.new), "# project\n\ncommitted edit\n");

        let added = repo.diff("brand new.txt", None, Scope::Uncommitted);
        assert_eq!(added.old, Content::Absent);
        assert_eq!(text_of(&added.new), "hello\n");

        std::fs::remove_file(repo.path().join("README.md")).unwrap();
        assert_eq!(
            repo.diff("README.md", None, Scope::Uncommitted).new,
            Content::Absent
        );
    }

    #[test]
    fn renames_compare_against_the_old_path() {
        let repo = Repo::new();
        repo.run(&["mv", "README.md", "docs.md"]);
        let diff = repo.diff("docs.md", Some("README.md"), Scope::Uncommitted);
        assert_eq!(text_of(&diff.old), "# project\n\nline two\n");
        assert_eq!(text_of(&diff.new), "# project\n\nline two\n");
    }

    #[test]
    fn binary_and_oversized_files_are_flagged_not_loaded() {
        let repo = Repo::new();
        std::fs::write(
            repo.path().join("image.bin"),
            [0x89, b'P', b'N', b'G', 0, 1, 2],
        )
        .unwrap();
        std::fs::write(repo.path().join("huge.log"), vec![b'x'; MAX_VIEW_BYTES + 1]).unwrap();
        assert_eq!(
            repo.diff("image.bin", None, Scope::Uncommitted).new,
            Content::Binary
        );
        assert!(matches!(
            repo.diff("huge.log", None, Scope::Uncommitted).new,
            Content::TooLarge { bytes } if bytes as usize == MAX_VIEW_BYTES + 1
        ));
        repo.commit("binaries");
        assert_eq!(
            repo.diff("image.bin", None, Scope::Uncommitted).old,
            Content::Binary
        );
        assert!(matches!(
            repo.diff("huge.log", None, Scope::Uncommitted).old,
            Content::TooLarge { .. }
        ));
    }

    #[test]
    fn image_like_text_stays_editable_and_malformed_bmp_is_binary() {
        let repo = Repo::new();
        for text in [
            "BM25 notes\n",
            "BMP files start with…",
            "GIF87a notes\n",
            "GIF89a notes\n",
            "RIFF1234WEBP notes\n",
        ] {
            repo.write("notes.txt", text);
            assert_eq!(
                read_working_file(repo.path(), "notes.txt").unwrap(),
                Content::Text { text: text.into() }
            );
            save_working_file(repo.path(), "notes.txt", text, "updated").unwrap();
        }
        assert_eq!(classify(b"BM\0".to_vec()), Content::Binary);
        let mut bmp = vec![0; 58];
        bmp[..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
        assert!(
            matches!(classify(bmp.clone()), Content::Image { mime, .. } if mime == "image/bmp")
        );
        bmp[10..14].copy_from_slice(&100_u32.to_le_bytes());
        assert_eq!(classify(bmp), Content::Binary);
    }

    #[test]
    fn read_only_files_are_rejected_without_changing_contents_or_permissions() {
        let repo = Repo::new();
        repo.write("readonly.txt", "old");
        let path = repo.path().join("readonly.txt");
        let original = std::fs::metadata(&path).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&path, readonly).unwrap();
        let result = save_working_file(repo.path(), "readonly.txt", "old", "new");
        let still_readonly = std::fs::metadata(&path).unwrap().permissions().readonly();
        // Restore the original permissions so the Windows temp directory can be removed.
        std::fs::set_permissions(&path, original).unwrap();
        assert_eq!(result.unwrap_err().code, "not_editable");
        assert!(still_readonly);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "old");
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_retargeted_during_save_cannot_replace_an_outside_file() {
        let repo = Repo::new();
        let outside = tempfile::tempdir().unwrap();
        repo.write("folder/file.txt", "old");
        std::fs::write(outside.path().join("file.txt"), "old").unwrap();
        let result =
            save_working_file_before_replace(repo.path(), "folder/file.txt", "old", "new", || {
                std::fs::rename(repo.path().join("folder"), repo.path().join("original")).unwrap();
                std::os::unix::fs::symlink(outside.path(), repo.path().join("folder")).unwrap();
            });
        assert_eq!(result.unwrap_err().code, "bad_path");
        assert_eq!(
            std::fs::read_to_string(outside.path().join("file.txt")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("original/file.txt")).unwrap(),
            "old"
        );
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlinks_are_explained_instead_of_opened_for_editing() {
        let repo = Repo::new();
        repo.write("target.txt", "old");
        std::os::unix::fs::symlink("target.txt", repo.path().join("link.txt")).unwrap();
        assert!(
            matches!(read_working_file(repo.path(), "link.txt").unwrap(), Content::NotEditable { reason } if reason.contains("Symbolic links"))
        );
        assert_eq!(
            save_working_file(repo.path(), "link.txt", "old", "new")
                .unwrap_err()
                .code,
            "not_editable"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("target.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn images_are_returned_losslessly_in_files_and_git_revisions() {
        use base64::Engine;
        let repo = Repo::new();
        let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1kAAAAASUVORK5CYII=").unwrap();
        std::fs::write(repo.path().join("pixel.png"), &png).unwrap();
        repo.commit("image");
        let image = read_working_file(repo.path(), "pixel.png").unwrap();
        let Content::Image { mime, data } = &image else {
            panic!("expected image")
        };
        assert_eq!(mime, "image/png");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap(),
            png
        );
        assert_eq!(repo.diff("pixel.png", None, Scope::Uncommitted).old, image);
        assert_eq!(classify(vec![0xff, 0xfe, b'a']), Content::Binary);
    }

    #[test]
    fn saves_text_without_changing_line_endings_and_refuses_stale_or_deleted_files() {
        let repo = Repo::new();
        repo.write("edit.rs", "old\r\n");
        save_working_file(repo.path(), "edit.rs", "old\r\n", "new\r\n").unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("edit.rs")).unwrap(),
            b"new\r\n"
        );
        assert_eq!(
            save_working_file(repo.path(), "edit.rs", "old\r\n", "oops")
                .unwrap_err()
                .code,
            "file_changed"
        );
        assert_eq!(
            std::fs::read(repo.path().join("edit.rs")).unwrap(),
            b"new\r\n"
        );
        std::fs::remove_file(repo.path().join("edit.rs")).unwrap();
        assert!(save_working_file(repo.path(), "edit.rs", "new\r\n", "oops").is_err());
        assert!(!repo.path().join("edit.rs").exists());
    }

    #[test]
    fn saves_reject_binary_large_git_internal_and_escaping_paths() {
        let repo = Repo::new();
        repo.write("edit.txt", "old");
        for text in ["a\0b".to_string(), "x".repeat(MAX_VIEW_BYTES + 1)] {
            assert_eq!(
                save_working_file(repo.path(), "edit.txt", "old", &text)
                    .unwrap_err()
                    .code,
                "not_editable"
            );
        }
        assert_eq!(std::fs::read(repo.path().join("edit.txt")).unwrap(), b"old");
        assert_eq!(
            save_working_file(repo.path(), "../outside", "", "x")
                .unwrap_err()
                .code,
            "bad_path"
        );
        assert_eq!(
            save_working_file(repo.path(), ".git/HEAD", "", "x")
                .unwrap_err()
                .code,
            "not_editable"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("edit.txt", repo.path().join("link.txt")).unwrap();
            assert_eq!(
                save_working_file(repo.path(), "link.txt", "old", "x")
                    .unwrap_err()
                    .code,
                "not_editable"
            );
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                repo.path().join("edit.txt"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            save_working_file(repo.path(), "edit.txt", "old", "new").unwrap();
            assert_eq!(
                std::fs::metadata(repo.path().join("edit.txt"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn paths_cannot_escape_the_workspace() {
        let repo = Repo::new();
        let changes = Changes {
            git: &repo.git,
            root: repo.path(),
            base_branch: None,
        };
        for bad in ["../outside", "/etc/passwd", "a/../../b", ""] {
            let err = changes.diff(bad, None, Scope::Uncommitted).unwrap_err();
            assert_eq!(err.code, "bad_path", "{bad:?}");
        }
        #[cfg(unix)]
        {
            let secret = tempfile::tempdir().unwrap();
            std::fs::write(secret.path().join("key"), "secret").unwrap();
            std::os::unix::fs::symlink(secret.path(), repo.path().join("link")).unwrap();
            assert_eq!(
                read_working_file(repo.path(), "link/key").unwrap_err().code,
                "bad_path"
            );
        }
        assert_eq!(
            read_working_file(repo.path(), "not/there.txt").unwrap(),
            Content::Absent
        );
    }

    #[test]
    fn a_repository_without_commits_lists_everything_as_untracked() {
        let repo = Repo {
            git: git(),
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git.init(repo.path()).unwrap();
        repo.write("first.txt", "hi\n");
        let set = repo.changes(None);
        assert_eq!(
            summary(&set.uncommitted),
            [("first.txt".into(), ChangeKind::Untracked, None, None)]
        );
    }

    #[test]
    fn a_conflict_is_reported_as_one() {
        let repo = Repo::new();
        repo.run(&["checkout", "-q", "-b", "other"]);
        repo.write("README.md", "theirs\n");
        repo.commit("theirs");
        repo.run(&["checkout", "-q", "main"]);
        repo.write("README.md", "ours\n");
        repo.commit("ours");
        let _ = repo.git.run(repo.path(), &["merge", "other"]);
        let set = repo.changes(None);
        assert_eq!(set.uncommitted[0].kind, ChangeKind::Conflicted);
    }
}

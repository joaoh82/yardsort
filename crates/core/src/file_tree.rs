//! The file tree of a workspace, one folder at a time, leaving out what git ignores.

use std::path::Path;

use ignore::WalkBuilder;
use serde::Serialize;
use specta::Type;

use crate::changes::resolve_inside;
use crate::error::{IpcError, IpcResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    /// Relative to the workspace root, with `/` separators.
    pub path: String,
    pub is_dir: bool,
    /// Excluded by the ignore rules (or `.git` itself). Only ever listed on request.
    pub ignored: bool,
}

/// The entries of `dir` (relative; empty for the root): folders first, then files, each sorted
/// by name. Hidden files are always shown — `.github`, `.env.example` matter.
///
/// By default `.git` and whatever `.gitignore` excludes are left out, which is what keeps the
/// tree about the project rather than about `node_modules`. With `show_ignored` they are listed
/// too, flagged so the UI can set them apart. Entries *inside* an ignored folder are not flagged
/// here — the rules only name the folder — so callers carry the flag down the tree.
pub fn list_dir(root: &Path, dir: &str, show_ignored: bool) -> IpcResult<Vec<FileEntry>> {
    let target = if dir.is_empty() {
        root.to_path_buf()
    } else {
        resolve_inside(root, dir)?
    };
    if !target.is_dir() {
        return Err(IpcError::new(
            "not_a_directory",
            format!("\"{dir}\" is not a folder."),
        ));
    }

    let walk = |respect_ignores: bool| -> Vec<FileEntry> {
        WalkBuilder::new(&target)
            .max_depth(Some(1))
            .hidden(false)
            .parents(true)
            .require_git(false)
            .git_ignore(respect_ignores)
            .git_exclude(respect_ignores)
            .git_global(respect_ignores)
            .ignore(respect_ignores)
            .filter_entry(move |entry| !respect_ignores || entry.file_name() != ".git")
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.depth() == 1)
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(root).ok()?;
                Some(FileEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    path: relative.to_string_lossy().replace('\\', "/"),
                    is_dir: entry.file_type().is_some_and(|kind| kind.is_dir()),
                    ignored: false,
                })
            })
            .collect()
    };

    let mut entries = walk(true);
    if show_ignored {
        let visible: std::collections::HashSet<String> =
            entries.iter().map(|entry| entry.path.clone()).collect();
        entries = walk(false);
        for entry in &mut entries {
            entry.ignored = !visible.contains(&entry.path);
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(".gitignore", "node_modules/\n*.log\n/dist\n");
        write(".git/HEAD", "ref: refs/heads/main\n");
        write(".github/workflows/ci.yml", "");
        write("node_modules/react/index.js", "");
        write("src/main.rs", "");
        write("src/nested/.gitignore", "secret.txt\n");
        write("src/nested/secret.txt", "");
        write("src/nested/kept.txt", "");
        write("dist/bundle.js", "");
        write("Zebra.md", "");
        write("apple.md", "");
        write("debug.log", "");
        dir
    }

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn the_root_lists_folders_first_without_git_or_ignored_things() {
        let dir = tree();
        let entries = list_dir(dir.path(), "", false).unwrap();
        assert_eq!(
            names(&entries),
            [".github", "src", ".gitignore", "apple.md", "Zebra.md"]
        );
        assert!(entries[0].is_dir && !entries[2].is_dir);
    }

    #[test]
    fn subfolders_are_listed_lazily_with_root_relative_paths_and_nested_ignores() {
        let dir = tree();
        let src = list_dir(dir.path(), "src", false).unwrap();
        assert_eq!(names(&src), ["nested", "main.rs"]);
        assert_eq!(src[1].path, "src/main.rs");

        let nested = list_dir(dir.path(), "src/nested", false).unwrap();
        assert_eq!(
            names(&nested),
            [".gitignore", "kept.txt"],
            "the nested .gitignore applies"
        );
    }

    #[test]
    fn ignored_entries_are_listed_on_request_and_flagged() {
        let dir = tree();
        let entries = list_dir(dir.path(), "", true).unwrap();
        assert_eq!(
            names(&entries),
            [
                ".git",
                ".github",
                "dist",
                "node_modules",
                "src",
                ".gitignore",
                "apple.md",
                "debug.log",
                "Zebra.md"
            ]
        );
        let ignored: Vec<_> = entries
            .iter()
            .filter(|e| e.ignored)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(ignored, [".git", "dist", "node_modules", "debug.log"]);

        // Inside an ignored folder everything is reachable; and a nested ignore file still flags.
        assert_eq!(
            names(&list_dir(dir.path(), "node_modules", true).unwrap()),
            ["react"]
        );
        assert_eq!(
            names(&list_dir(dir.path(), ".git", true).unwrap()),
            ["HEAD"]
        );
        let nested = list_dir(dir.path(), "src/nested", true).unwrap();
        let secret = nested.iter().find(|e| e.name == "secret.txt").unwrap();
        assert!(secret.ignored);
        assert!(
            !nested
                .iter()
                .find(|e| e.name == "kept.txt")
                .unwrap()
                .ignored
        );
    }

    #[test]
    fn only_folders_inside_the_workspace_can_be_listed() {
        let dir = tree();
        assert_eq!(
            list_dir(dir.path(), "../", false).unwrap_err().code,
            "bad_path"
        );
        assert_eq!(
            list_dir(dir.path(), "apple.md", false).unwrap_err().code,
            "not_a_directory"
        );
    }
}

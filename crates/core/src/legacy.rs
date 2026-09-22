//! Yardsort was called **Switchyard** until v0.2. This module is everything that still knows:
//! it carries a user's data over on first launch and keeps the old environment variables working.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const OLD_IDENTIFIER: &str = "dev.switchyard.app";
const OLD_DATABASE: &str = "switchyard.db";
const OLD_BRANCH_PREFIX: &str = "sy";

/// `YARDSORT_<suffix>`, falling back to the `SWITCHYARD_<suffix>` people may still have in
/// scripts and shell profiles. Empty values count as unset.
pub fn env_var_os(suffix: &str) -> Option<OsString> {
    ["YARDSORT", "SWITCHYARD"]
        .iter()
        .filter_map(|prefix| std::env::var_os(format!("{prefix}_{suffix}")))
        .find(|value| !value.is_empty())
}

/// Branches made before the rename are `sy/<name>`; they should still read as `<name>`.
pub fn strip_old_branch_prefix(branch: &str) -> Option<&str> {
    branch
        .strip_prefix(OLD_BRANCH_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// The folder the app used under its old identifier, next to the new one.
fn old_dir(new_dir: &Path) -> Option<PathBuf> {
    Some(new_dir.parent()?.join(OLD_IDENTIFIER)).filter(|dir| dir.is_dir())
}

/// Copy `from` to `to` unless `to` already exists. Returns whether anything was copied.
fn copy_if_absent(from: &Path, to: &Path) -> std::io::Result<bool> {
    if to.exists() || !from.is_file() {
        return Ok(false);
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    Ok(true)
}

/// First launch after the rename: bring the database and settings over from the Switchyard
/// folders. Everything is **copied**, never moved, and nothing that already exists is touched —
/// so the old app keeps working, and running this again is harmless. Returns what was carried
/// over, for the log.
pub fn adopt_switchyard_data(
    data_dir: &Path,
    database_name: &str,
    config_dir: &Path,
) -> std::io::Result<Vec<String>> {
    let mut adopted = Vec::new();

    if let Some(old) = old_dir(data_dir) {
        let database = data_dir.join(database_name);
        if !database.exists() {
            // SQLite in WAL mode keeps recent writes in side files; the three belong together.
            for suffix in ["", "-wal", "-shm"] {
                let from = old.join(format!("{OLD_DATABASE}{suffix}"));
                let to = data_dir.join(format!("{database_name}{suffix}"));
                if copy_if_absent(&from, &to)? && suffix.is_empty() {
                    adopted.push(format!("database from {}", from.display()));
                }
            }
        }
    }

    if let Some(old) = old_dir(config_dir) {
        let from = old.join("settings.toml");
        if copy_if_absent(&from, &config_dir.join("settings.toml"))? {
            adopted.push(format!("settings from {}", from.display()));
        }
    }
    Ok(adopted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> (PathBuf, PathBuf) {
        let old = root.join(OLD_IDENTIFIER);
        std::fs::create_dir_all(&old).unwrap();
        (old, root.join("dev.yardsort.app"))
    }

    #[test]
    fn a_real_switchyard_database_comes_over_with_its_write_ahead_log() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = dirs(root.path());
        // A genuine database with un-checkpointed writes, left behind the way a killed app
        // leaves it: side files still on disk.
        {
            let store = crate::store::Store::open(&old.join(OLD_DATABASE)).unwrap();
            store
                .add_project("from-before", "/code/from-before")
                .unwrap();
            for suffix in ["", "-wal", "-shm"] {
                let file = old.join(format!("{OLD_DATABASE}{suffix}"));
                if file.exists() {
                    std::fs::copy(&file, old.join(format!("kept{suffix}"))).unwrap();
                }
            }
        }
        for suffix in ["", "-wal", "-shm"] {
            let kept = old.join(format!("kept{suffix}"));
            if kept.exists() {
                std::fs::rename(kept, old.join(format!("{OLD_DATABASE}{suffix}"))).unwrap();
            }
        }

        let adopted = adopt_switchyard_data(&new, "yardsort.db", &new).unwrap();

        assert_eq!(adopted.len(), 1, "{adopted:?}");
        let store = crate::store::Store::open(&new.join("yardsort.db")).unwrap();
        assert_eq!(store.projects().unwrap()[0].name, "from-before");
        assert!(old.join(OLD_DATABASE).exists(), "copied, not moved");
    }

    #[test]
    fn existing_yardsort_data_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = dirs(root.path());
        std::fs::write(old.join(OLD_DATABASE), "old").unwrap();
        std::fs::write(old.join("settings.toml"), "old settings").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("yardsort.db"), "mine").unwrap();
        std::fs::write(new.join("settings.toml"), "my settings").unwrap();

        assert!(adopt_switchyard_data(&new, "yardsort.db", &new)
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(new.join("yardsort.db")).unwrap(),
            "mine"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("settings.toml")).unwrap(),
            "my settings"
        );
    }

    #[test]
    fn a_stale_wal_is_not_attached_to_a_database_that_already_exists() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = dirs(root.path());
        std::fs::write(old.join(format!("{OLD_DATABASE}-wal")), "old wal").unwrap();
        std::fs::write(old.join(OLD_DATABASE), "old").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("yardsort.db"), "mine").unwrap();

        adopt_switchyard_data(&new, "yardsort.db", &new).unwrap();
        assert!(!new.join("yardsort.db-wal").exists());
    }

    #[test]
    fn settings_come_over_and_a_fresh_install_adopts_nothing() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("nothing-here").join("dev.yardsort.app");
        assert!(adopt_switchyard_data(&fresh, "yardsort.db", &fresh)
            .unwrap()
            .is_empty());

        let (old, new) = dirs(root.path());
        std::fs::write(old.join("settings.toml"), "[workspaces]\n").unwrap();
        let adopted = adopt_switchyard_data(&new, "yardsort.db", &new).unwrap();
        assert_eq!(adopted.len(), 1);
        assert!(new.join("settings.toml").exists());
    }

    #[test]
    fn old_branch_names_still_read_as_workspace_names() {
        assert_eq!(strip_old_branch_prefix("sy/fix-login"), Some("fix-login"));
        assert_eq!(strip_old_branch_prefix("system/x"), None);
        assert_eq!(strip_old_branch_prefix("sy"), None);
    }
}

//! Exits kept for a client that is not there.
//!
//! The daemon announces an `Exited` event to every connected client, and keeps the exited
//! session in its list — but it has no database, and it stops itself once nothing is connected
//! and nothing is running. An agent that finishes while the window is closed therefore ends
//! with nobody to tell, and the next app start could only guess "interrupted". The spool is the
//! fix: one small JSON file per exit, in a directory the client drains and empties the next
//! time it connects.
//!
//! Rules that keep it harmless: a file is written to a temporary name and renamed into place,
//! so a reader never sees half of one; the daemon writes and never rewrites; the directory is
//! capped, and past the cap the daemon drops the entry and counts it instead of filling a disk;
//! and nothing here is on the path terminal output takes — an error is printed and forgotten.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use pty_host::{ExitInfo, SessionId};
use serde::{Deserialize, Serialize};

/// Bumped when an entry changes shape. A reader refuses newer versions rather than guess.
pub const SPOOL_VERSION: u32 = 1;
/// How many entries may wait. Each is a few hundred bytes; this is far more exits than a
/// person leaves unread, and a bound on what one profile can accumulate.
pub const MAX_ENTRIES: usize = 2_000;
/// Refuse to read anything larger: a real entry is well under a kilobyte.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024;

/// The file that counts entries the cap turned away.
const DROPPED_FILE: &str = "dropped";

/// One exit, as the daemon saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpoolEntry {
    pub version: u32,
    /// Milliseconds since the Unix epoch, on the daemon's clock.
    pub at_ms: u64,
    pub daemon_pid: u32,
    pub session: SessionId,
    /// The session's labels, so a reader can tell whose exit this was without the daemon.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    pub exit: ExitInfo,
}

/// A spool directory: the daemon's side writes, a client's side drains.
#[derive(Debug, Clone)]
pub struct Spool {
    dir: PathBuf,
}

impl Spool {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Keep an exit for whoever connects next. Fails, having counted a drop, when the spool is
    /// full; fails plainly when the directory cannot be written.
    pub fn record(&self, entry: &SpoolEntry) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        private_dir(&self.dir);
        if self.entries()?.len() >= MAX_ENTRIES {
            self.note_dropped();
            return Err(io::Error::other(format!(
                "the exit spool at {} is full ({MAX_ENTRIES} entries)",
                self.dir.display()
            )));
        }
        let name = format!("{}-{}", entry.at_ms, entry.session);
        let temporary = self.dir.join(format!("{name}.tmp"));
        let bytes = serde_json::to_vec(entry).map_err(io::Error::other)?;
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, self.dir.join(format!("{name}.json")))
    }

    /// The entries waiting, oldest first. A directory that does not exist has none.
    pub fn entries(&self) -> io::Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
            Err(error) => return Err(error),
        };
        for entry in dir {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                found.push(path);
            }
        }
        found.sort();
        Ok(found)
    }

    /// Read one entry. A file that is too large, unreadable, or from a newer daemon is an
    /// error; the caller decides what to do with it (delete it, and count it).
    pub fn read(path: &Path) -> io::Result<SpoolEntry> {
        let size = std::fs::metadata(path)?.len();
        if size > MAX_ENTRY_BYTES {
            return Err(io::Error::other(format!(
                "spool entry of {size} bytes is too large"
            )));
        }
        let bytes = std::fs::read(path)?;
        let entry: SpoolEntry = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if entry.version > SPOOL_VERSION {
            return Err(io::Error::other(format!(
                "spool entry is version {}, this build reads {SPOOL_VERSION}",
                entry.version
            )));
        }
        Ok(entry)
    }

    /// How many entries the cap turned away since last asked, and forget the count.
    pub fn take_dropped(&self) -> u64 {
        let path = self.dir.join(DROPPED_FILE);
        let count = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        if count > 0 {
            let _ = std::fs::remove_file(&path);
        }
        count
    }

    fn note_dropped(&self) {
        let path = self.dir.join(DROPPED_FILE);
        let count: u64 = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let temporary = self.dir.join("dropped.tmp");
        if std::fs::write(&temporary, (count + 1).to_string()).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

/// Keep the spool to its owner: an entry names a session and its workspace.
#[cfg(unix)]
fn private_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

/// A directory under the user's profile is already theirs on Windows.
#[cfg(not(unix))]
fn private_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at_ms: u64, session: &str, code: u32) -> SpoolEntry {
        SpoolEntry {
            version: SPOOL_VERSION,
            at_ms,
            daemon_pid: 42,
            session: SessionId(session.into()),
            labels: BTreeMap::from([("workspace".to_owned(), "ws-1".to_owned())]),
            exit: ExitInfo {
                code,
                success: code == 0,
                signal: None,
            },
        }
    }

    #[test]
    fn entries_come_back_oldest_first_and_whole() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path().join("spool"));
        assert!(
            spool.entries().unwrap().is_empty(),
            "no directory, no entries"
        );

        spool.record(&entry(20, "b", 1)).unwrap();
        spool.record(&entry(10, "a", 0)).unwrap();
        let paths = spool.entries().unwrap();
        assert_eq!(paths.len(), 2);
        let first = Spool::read(&paths[0]).unwrap();
        assert_eq!((first.session.0.as_str(), first.exit.code), ("a", 0));
        assert_eq!(first.labels["workspace"], "ws-1");
        assert_eq!(Spool::read(&paths[1]).unwrap(), entry(20, "b", 1));
        assert!(
            !dir.path().join("spool").join("20-b.tmp").exists(),
            "renamed into place"
        );
    }

    #[test]
    fn a_broken_or_oversized_or_future_entry_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path().to_path_buf());
        spool.record(&entry(1, "ok", 0)).unwrap();

        let junk = dir.path().join("2-junk.json");
        std::fs::write(&junk, b"{ not json").unwrap();
        assert!(Spool::read(&junk).is_err());

        let huge = dir.path().join("3-huge.json");
        std::fs::write(&huge, vec![b' '; MAX_ENTRY_BYTES as usize + 1]).unwrap();
        assert!(Spool::read(&huge)
            .unwrap_err()
            .to_string()
            .contains("too large"));

        let future = dir.path().join("4-future.json");
        let mut newer = entry(4, "f", 0);
        newer.version = SPOOL_VERSION + 1;
        std::fs::write(&future, serde_json::to_vec(&newer).unwrap()).unwrap();
        assert!(Spool::read(&future)
            .unwrap_err()
            .to_string()
            .contains("version"));

        // Listing still works around them; reading each one is the caller's problem.
        assert_eq!(spool.entries().unwrap().len(), 4);
    }

    #[test]
    fn a_full_spool_drops_and_counts_instead_of_growing() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path().to_path_buf());
        for n in 0..MAX_ENTRIES {
            spool.record(&entry(n as u64, &format!("s{n}"), 0)).unwrap();
        }
        assert!(spool.record(&entry(9_999, "one-too-many", 0)).is_err());
        assert!(spool.record(&entry(9_998, "two-too-many", 0)).is_err());
        assert_eq!(spool.entries().unwrap().len(), MAX_ENTRIES);
        assert_eq!(spool.take_dropped(), 2);
        assert_eq!(spool.take_dropped(), 0, "taken means forgotten");
    }
}

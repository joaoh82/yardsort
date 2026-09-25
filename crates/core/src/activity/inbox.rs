//! Events an agent reported, waiting to be taken into the database.
//!
//! A hook an agent runs is a short-lived process with nobody to talk to: the window may be
//! closed, `ys` has long returned, and the database may be busy behind a lock it must not wait
//! for. So it does what the daemon's exit spool does — writes one small file and exits — and
//! whichever client comes next drains the directory. The same rules keep it harmless: a
//! temporary name then a rename, so a reader never sees half an entry; a cap, past which the
//! entry is dropped and counted rather than allowed to fill a disk; and never more than a few
//! hundred bytes of work on the agent's own critical path.
//!
//! An entry is already normalized: the hook keeps the event's metadata and throws the rest of
//! the payload away before anything touches the disk.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when an entry changes shape. A reader refuses newer versions rather than guess.
pub const INBOX_VERSION: u32 = 1;
/// How many entries may wait. A tool call is two of them, so a long conversation with the
/// window closed is a few thousand; each is a few hundred bytes.
pub const MAX_ENTRIES: usize = 10_000;
/// Refuse to read anything larger: a real entry is well under a kilobyte.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024;

const DROPPED_FILE: &str = "dropped";

/// One reported event, as the hook understood it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxEntry {
    pub version: u32,
    /// Unique per entry: the event's id, and what makes a second import of it a no-op.
    pub id: String,
    /// When the hook ran, on its clock. The agent gives no time of its own.
    pub at_ms: i64,
    /// Who reported it (`claude`), how (`hook`) and how sure to be (`reported`).
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    /// From the hook process's environment: what the launcher told the agent about itself.
    pub run_id: Option<String>,
    pub workspace_id: Option<String>,
    pub session_record_id: Option<String>,
    /// The agent's own session id — the fallback key when the environment did not survive.
    pub native_session_id: Option<String>,
    pub kind: String,
    pub privacy_class: String,
    pub payload: serde_json::Value,
}

/// An inbox directory: hooks write, a client drains.
#[derive(Debug, Clone)]
pub struct Inbox {
    dir: PathBuf,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Keep an entry for whoever drains next. Fails, having counted a drop, when the inbox is
    /// full; fails plainly when the directory cannot be written.
    pub fn record(&self, entry: &InboxEntry) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        private_dir(&self.dir);
        if self.entries()?.len() >= MAX_ENTRIES {
            self.note_dropped();
            return Err(io::Error::other(format!(
                "the activity inbox at {} is full ({MAX_ENTRIES} entries)",
                self.dir.display()
            )));
        }
        let name = format!("{:013}-{}", entry.at_ms.max(0), entry.id);
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

    /// Read one entry. Too large, unreadable, or from a newer build: an error, and the caller
    /// decides (delete it, and count it).
    pub fn read(path: &Path) -> io::Result<InboxEntry> {
        let size = std::fs::metadata(path)?.len();
        if size > MAX_ENTRY_BYTES {
            return Err(io::Error::other(format!(
                "inbox entry of {size} bytes is too large"
            )));
        }
        let bytes = std::fs::read(path)?;
        let entry: InboxEntry = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if entry.version > INBOX_VERSION {
            return Err(io::Error::other(format!(
                "inbox entry is version {}, this build reads {INBOX_VERSION}",
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

/// Keep the inbox to its owner: an entry names a workspace and what ran in it.
#[cfg(unix)]
fn private_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

/// The data directory is already the user's own on Windows.
#[cfg(not(unix))]
fn private_dir(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, at_ms: i64) -> InboxEntry {
        InboxEntry {
            version: INBOX_VERSION,
            id: id.into(),
            at_ms,
            producer: "claude".into(),
            method: "hook".into(),
            fidelity: "reported".into(),
            run_id: Some("run-1".into()),
            workspace_id: Some("ws-1".into()),
            session_record_id: None,
            native_session_id: Some("s-1".into()),
            kind: "tool.completed".into(),
            privacy_class: "metadata".into(),
            payload: serde_json::json!({ "tool": "Read" }),
        }
    }

    #[test]
    fn entries_come_back_oldest_first_and_whole() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        assert!(
            inbox.entries().unwrap().is_empty(),
            "no directory, no entries"
        );
        inbox.record(&entry("b", 20)).unwrap();
        inbox.record(&entry("a", 10)).unwrap();
        let paths = inbox.entries().unwrap();
        let read: Vec<InboxEntry> = paths.iter().map(|p| Inbox::read(p).unwrap()).collect();
        assert_eq!(read, vec![entry("a", 10), entry("b", 20)]);
        assert!(
            std::fs::read_dir(inbox.dir())
                .unwrap()
                .all(|e| e.unwrap().path().extension().unwrap() == "json"),
            "no temporary files left behind"
        );
    }

    #[test]
    fn a_full_inbox_drops_and_counts_and_a_newer_entry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().to_path_buf());
        for i in 0..MAX_ENTRIES {
            std::fs::write(dir.path().join(format!("{i:013}-x.json")), b"{}").unwrap();
        }
        assert!(inbox.record(&entry("late", 1)).is_err());
        assert!(inbox.record(&entry("later", 2)).is_err());
        assert_eq!(inbox.take_dropped(), 2);
        assert_eq!(inbox.take_dropped(), 0, "taken means forgotten");

        let newer = dir.path().join("newer.json");
        let mut e = entry("n", 1);
        e.version = INBOX_VERSION + 1;
        std::fs::write(&newer, serde_json::to_vec(&e).unwrap()).unwrap();
        assert!(Inbox::read(&newer).is_err());
    }
}

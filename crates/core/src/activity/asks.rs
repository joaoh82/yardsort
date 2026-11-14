//! Permissions an agent asked for through its own hook, open for a device to answer.
//!
//! Claude Code runs its `PermissionRequest` hook as it shows its own question in the terminal,
//! and takes whichever answer comes first: a key pressed there, or the hook's. So the hook,
//! after reporting the event as every hook does, writes the question here and waits for an
//! answer beside it ([`wait`]); a device answers with [`answer`]. Nobody at the computer waits
//! on the hook: the terminal asks all the while.
//!
//! A question is closed — its file removed, which ends the wait with no answer — when the agent
//! moves on without one: its next hook for the same tool call (it ran, failed or was denied)
//! or the end of its turn ([`close_answered`], run by those hooks). Past [`ASK_TTL_MS`] it is
//! stale whatever happened, since Claude Code ends a hook at its timeout.
//!
//! One file per question, written under a temporary name and renamed, as the inbox is. It holds
//! the command the agent wants to run, which a device shows before it is answered: that is the
//! point of it. It lasts as long as the question is open and is never copied into the database.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long an open question is answerable. Claude Code's hook is given a little longer
/// ([`super::claude::ASK_TIMEOUT_SECS`]), so the wait ends on its own terms.
pub const ASK_TTL_MS: i64 = 590_000;
/// How often a waiting hook looks for its answer.
pub const POLL: Duration = Duration::from_millis(250);
/// Longest command kept, in characters.
pub const MAX_COMMAND: usize = 2_000;
/// How often a waiting hook says it is still there, by touching its question's file.
pub const HEARTBEAT: Duration = Duration::from_secs(2);
/// A question whose file has not been touched in this long has nobody waiting on it: Claude Code
/// ends its hook when the question is answered "No" or with Esc at the terminal, before the hook
/// can take it away.
pub const ABANDONED: Duration = Duration::from_secs(10);

const ASK_EXT: &str = "json";
const ANSWER_EXT: &str = "answer";

/// Where open questions are kept, beside the inbox.
pub fn asks_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("asks")
}

/// The same, from the inbox directory a hook is given.
pub fn asks_dir_beside(inbox_dir: &Path) -> PathBuf {
    inbox_dir
        .parent()
        .map_or_else(|| inbox_dir.join("asks"), |activity| activity.join("asks"))
}

/// An open question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub id: String,
    pub at_ms: i64,
    pub session_record_id: String,
    pub harness: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Relative to the workspace, or absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Which tool call it is, to close it when that call's next hook comes: a digest of the tool's
    /// name and input, not the input.
    pub call: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    Deny,
}

/// Which tool call a hook payload is about: the same for its `PermissionRequest` and the
/// `PostToolUse`, `PostToolUseFailure` or `PermissionDenied` that follows.
pub fn call_of(tool: &str, input: Option<&Value>) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(tool.as_bytes());
    hash.update([0]);
    hash.update(serde_json::to_vec(&input.cloned().unwrap_or(Value::Null)).unwrap_or_default());
    hash.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn path_of(dir: &Path, id: &str, ext: &str) -> Option<PathBuf> {
    let safe = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    safe.then(|| dir.join(format!("{id}.{ext}")))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// Open a question.
pub fn post(dir: &Path, ask: &Ask) -> io::Result<()> {
    let path = path_of(dir, &ask.id, ASK_EXT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not an ask id"))?;
    std::fs::create_dir_all(dir)?;
    write_atomically(&path, &serde_json::to_vec(ask)?)
}

/// Remove a question and any answer to it.
pub fn close(dir: &Path, id: &str) {
    for ext in [ASK_EXT, ANSWER_EXT] {
        if let Some(path) = path_of(dir, id, ext) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Whether a hook still waits on the question in `path`: it touched it lately.
fn waited_on(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_none_or(|age| age <= ABANDONED)
}

fn touch(path: &Path) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

fn read(path: &Path) -> Option<Ask> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn all(dir: &Path) -> Vec<Ask> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == ASK_EXT))
        .filter_map(|path| read(&path))
        .collect()
}

/// Answers left with no question beside them — the question closed as the answer was written —
/// once they are a minute old: nothing will read them.
fn sweep_orphans(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if !path.extension().is_some_and(|ext| ext == ANSWER_EXT)
            || path.with_extension(ASK_EXT).exists()
        {
            continue;
        }
        let old = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(60));
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The questions `session` has open, oldest first. Stale ones are removed on the way.
pub fn open(dir: &Path, session: &str, now_ms: i64) -> Vec<Ask> {
    sweep_orphans(dir);
    let mut found: Vec<Ask> = all(dir)
        .into_iter()
        .filter(|ask| {
            let abandoned = path_of(dir, &ask.id, ASK_EXT).is_some_and(|path| !waited_on(&path));
            let stale = now_ms - ask.at_ms > ASK_TTL_MS || abandoned;
            if stale {
                close(dir, &ask.id);
            }
            !stale && ask.session_record_id == session
        })
        .collect();
    found.sort_by(|a, b| (a.at_ms, &a.id).cmp(&(b.at_ms, &b.id)));
    found
}

/// Answer a question of `session`. `false` when it is not open any more: answered already, at
/// the terminal or by another device, or the agent moved on.
pub fn answer(
    dir: &Path,
    session: &str,
    id: &str,
    decision: Decision,
    now_ms: i64,
) -> io::Result<bool> {
    let (Some(ask_path), Some(answer_path)) =
        (path_of(dir, id, ASK_EXT), path_of(dir, id, ANSWER_EXT))
    else {
        return Ok(false);
    };
    match read(&ask_path) {
        Some(ask)
            if ask.session_record_id == session
                && now_ms - ask.at_ms <= ASK_TTL_MS
                && waited_on(&ask_path) => {}
        _ => return Ok(false),
    }
    // The first answer stands: a second device's is refused, not swapped in.
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&answer_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    use std::io::Write;
    file.write_all(&serde_json::to_vec(&decision)?)?;
    file.sync_all()?;
    // Closed while this was written: the waiting hook is gone, so nothing will take it.
    if !ask_path.exists() {
        let _ = std::fs::remove_file(&answer_path);
        return Ok(false);
    }
    Ok(true)
}

/// Wait for an answer to `id` until `deadline`, or until `moved_on` says the agent went past the
/// question without one (it was answered at the terminal). `None` when the question was closed,
/// nobody answered in time, or the agent moved on. Either way the question is gone when this
/// returns.
pub fn wait(
    dir: &Path,
    id: &str,
    deadline: Instant,
    poll: Duration,
    moved_on: &mut dyn FnMut() -> bool,
) -> Option<Decision> {
    let (ask_path, answer_path) = (path_of(dir, id, ASK_EXT)?, path_of(dir, id, ANSWER_EXT)?);
    // An answer file that is still being written reads as nothing yet.
    let answered = || {
        std::fs::read(&answer_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Decision>(&bytes).ok())
    };
    let mut touched = Instant::now();
    let decided = loop {
        if let Some(decision) = answered() {
            break Some(decision);
        }
        if touched.elapsed() >= HEARTBEAT {
            touch(&ask_path);
            touched = Instant::now();
        }
        if moved_on() {
            break None;
        }
        if !ask_path.exists() || Instant::now() >= deadline {
            // An answer written just before the question closed still counts.
            break answered();
        }
        std::thread::sleep(poll);
    };
    close(dir, id);
    decided
}

/// Close the questions of `session` that its agent has moved past: those about `call` when it
/// is given (that tool call ran, failed or was denied), and every one when it is not (the turn
/// ended, or a new one began).
pub fn close_answered(dir: &Path, session: &str, call: Option<&str>) {
    for ask in all(dir) {
        if ask.session_record_id == session && call.is_none_or(|call| call == ask.call) {
            close(dir, &ask.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ask(id: &str, session: &str, at_ms: i64) -> Ask {
        Ask {
            id: id.into(),
            at_ms,
            session_record_id: session.into(),
            harness: "claude".into(),
            tool: "Bash".into(),
            command: Some("npm test".into()),
            path: None,
            call: call_of("Bash", Some(&json!({"command": "npm test"}))),
        }
    }

    #[test]
    fn a_question_is_answered_once_and_its_wait_returns_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        assert_eq!(open(dir.path(), "s1", 2_000), vec![ask("a1", "s1", 1_000)]);
        assert!(open(dir.path(), "s2", 2_000).is_empty());
        // Another conversation's device cannot answer it.
        assert!(!answer(dir.path(), "s2", "a1", Decision::Allow, 2_000).unwrap());
        assert!(answer(dir.path(), "s1", "a1", Decision::Deny, 2_000).unwrap());
        // The first answer stands.
        assert!(!answer(dir.path(), "s1", "a1", Decision::Allow, 2_000).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            wait(
                dir.path(),
                "a1",
                deadline,
                Duration::from_millis(5),
                &mut || false
            ),
            Some(Decision::Deny)
        );
        assert!(open(dir.path(), "s1", 2_000).is_empty());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn a_wait_ends_without_an_answer_when_the_agent_moves_on_or_time_runs_out() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        let path = dir.path().to_path_buf();
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            // That tool call ran: its PostToolUse names the same call.
            let call = call_of("Bash", Some(&json!({"command": "npm test"})));
            close_answered(&path, "s1", Some(&call));
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            wait(
                dir.path(),
                "a1",
                deadline,
                Duration::from_millis(5),
                &mut || false
            ),
            None
        );
        closer.join().unwrap();

        post(dir.path(), &ask("a2", "s1", 1_000)).unwrap();
        let started = Instant::now();
        assert_eq!(
            wait(
                dir.path(),
                "a2",
                started,
                Duration::from_millis(5),
                &mut || false
            ),
            None
        );
        assert!(open(dir.path(), "s1", 1_000).is_empty());
    }

    #[test]
    fn only_the_same_call_or_the_end_of_the_turn_closes_a_question() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        post(dir.path(), &ask("b1", "s2", 1_000)).unwrap();
        let other = call_of("Read", Some(&json!({"file_path": "src/a.rs"})));
        close_answered(dir.path(), "s1", Some(&other));
        assert_eq!(open(dir.path(), "s1", 1_000).len(), 1);
        close_answered(dir.path(), "s1", None);
        assert!(open(dir.path(), "s1", 1_000).is_empty());
        assert_eq!(
            open(dir.path(), "s2", 1_000).len(),
            1,
            "another conversation's stays"
        );
    }

    #[test]
    fn a_stale_question_is_neither_listed_nor_answerable_and_ids_cannot_leave_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 0)).unwrap();
        assert!(!answer(dir.path(), "s1", "a1", Decision::Allow, ASK_TTL_MS + 1).unwrap());
        assert!(open(dir.path(), "s1", ASK_TTL_MS + 1).is_empty());
        assert!(!dir.path().join("a1.json").exists());
        assert!(post(dir.path(), &ask("../x", "s1", 0)).is_err());
        assert!(!answer(dir.path(), "s1", "../x", Decision::Allow, 0).unwrap());
    }

    #[test]
    fn a_wait_ends_when_the_agent_moved_on_and_an_answer_to_a_closed_question_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        let mut asked = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut moved_on = || {
            asked += 1;
            asked > 2
        };
        assert_eq!(
            wait(
                dir.path(),
                "a1",
                deadline,
                Duration::from_millis(1),
                &mut moved_on
            ),
            None
        );
        assert!(open(dir.path(), "s1", 1_000).is_empty());
        // Nothing waits for it now, so a device is told it is not open.
        assert!(!answer(dir.path(), "s1", "a1", Decision::Allow, 1_000).unwrap());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn a_question_nobody_waits_on_any_more_is_closed_and_not_answerable() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        // The hook was ended before it could take the question away.
        let path = dir.path().join("a1.json");
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(std::time::SystemTime::now() - ABANDONED - Duration::from_secs(1))
            .unwrap();
        drop(file);
        assert!(!answer(dir.path(), "s1", "a1", Decision::Allow, 1_000).unwrap());
        assert!(open(dir.path(), "s1", 1_000).is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn a_waiting_hook_keeps_its_question_alive() {
        let dir = tempfile::tempdir().unwrap();
        post(dir.path(), &ask("a1", "s1", 1_000)).unwrap();
        let path = dir.path().join("a1.json");
        let old = std::time::SystemTime::now() - ABANDONED + Duration::from_secs(1);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let waiting = {
            let dir = dir.path().to_path_buf();
            std::thread::spawn(move || {
                let deadline = Instant::now() + HEARTBEAT + Duration::from_millis(500);
                wait(&dir, "a1", deadline, Duration::from_millis(50), &mut || {
                    false
                })
            })
        };
        std::thread::sleep(HEARTBEAT + Duration::from_millis(200));
        // Touched since: still open, though it was nearly abandoned when the wait began.
        assert_eq!(open(dir.path(), "s1", 1_000).len(), 1);
        assert_eq!(waiting.join().unwrap(), None);
    }
}

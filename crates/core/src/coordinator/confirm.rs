//! A yes the app records, which `ys` waits for.
//!
//! Ask first does not trust the agent to ask. `ys` writes the command it is about to run into
//! the profile and waits. The window lists those files and, when the user presses Allow or
//! Don't allow, writes the answer beside the request. `ys` then runs the command or refuses it.
//! The dialog never runs the command itself: the process that is waiting is the one that parsed
//! it, so a request nobody is waiting on does nothing when it is answered.
//!
//! This stops an agent that simply runs `ys` and carries on, which is what auto mode does. It
//! is not a sandbox. The Floor Manager runs as the user, so it could write the answer file
//! itself, drop [`super::ENV`], or run something that is not `ys`. The guide says so.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::presence::{self, Holder};

/// How long `ys` waits for a yes before it refuses. Long enough to read the command and decide;
/// short enough that a closed laptop does not leave an agent hanging overnight.
pub const WAIT: Duration = Duration::from_secs(10 * 60);

/// How recently a request must have been touched to still be shown as someone waiting.
///
/// `ys` touches the file on each pass of its wait. A command killed by a harness (Claude Code
/// stops a shell at two minutes) or by Ctrl-C does not run its cleanup, and the file would
/// otherwise sit until [`WAIT`]. The window polls about once a second, so a few seconds is long
/// enough to survive one slow poll and short enough that a dead command leaves the dialog.
///
/// A request this old is hidden, not deleted. Deleting it would strand the `ys` that is still
/// waiting: a pause longer than this (sleep, a busy machine, a clock stepping backwards) would
/// take the dialog away and leave the command sitting until [`WAIT`]. The file is removed only
/// once it is older than [`WAIT`].
const FRESH: Duration = Duration::from_secs(5);

/// One command waiting for a yes in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorConfirmation {
    pub id: String,
    /// The command line, for the dialog. Display only: answering does not run it.
    pub command: String,
    /// When the request was written, in milliseconds, so the oldest is asked first.
    ///
    /// A float rather than an integer: the binding generator refuses a 64-bit integer, and a
    /// millisecond count is exact in a float for centuries.
    pub created_unix_ms: f64,
}

/// Where requests and answers live: `<data dir>/coordinator/confirm`.
pub fn dir(data_dir: &Path) -> PathBuf {
    super::home(data_dir).join("confirm")
}

/// Write `command` and wait until the app answers, the app is gone, or [`WAIT`] passes.
///
/// `Ok(())` is a yes. Anything else is a reason the command was not run.
pub fn ask(data_dir: &Path, command: &str) -> Result<(), String> {
    if presence::holder(data_dir) != Some(Holder::App) {
        return Err(not_open(command));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let folder = dir(data_dir);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Cannot ask Yardsort to confirm this: {error}"))?;
    let request = CoordinatorConfirmation {
        id: id.clone(),
        command: command.to_owned(),
        created_unix_ms: now_ms(),
    };
    let path = folder.join(format!("{id}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&request).expect("a confirmation is plain data"),
    )
    .map_err(|error| format!("Cannot ask Yardsort to confirm this: {error}"))?;
    let _cleanup = Cleanup {
        folder: folder.clone(),
        id: id.clone(),
    };

    let deadline = std::time::Instant::now() + wait_for();
    loop {
        match read_answer(&folder, &id) {
            Some(true) => return Ok(()),
            Some(false) => return Err(refused(command)),
            None => {}
        }
        // The window removes a request that has outlived the wait. The file is then gone and
        // nobody will answer, so stop now instead of sitting here until the deadline. A request
        // that is only quiet is hidden, and this file is still here for the next pass.
        if !path.is_file() {
            return Err(withdrawn(command));
        }
        // Touched every pass, so a kill that skips [`Cleanup`] goes stale within [`FRESH`]
        // instead of occupying the dialog until the wait runs out. Opening for write does not
        // recreate a file the window has removed.
        heartbeat(&path);
        if presence::holder(data_dir) != Some(Holder::App) {
            return Err(not_open(command));
        }
        if std::time::Instant::now() >= deadline {
            return Err(timed_out(command));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The requests still waiting, oldest first. One that already has an answer is hidden: `ys` is
/// about to see it. One that has not been touched within [`FRESH`] is hidden too, and comes
/// back when it is touched again. One older than [`WAIT`] is removed, and so is an answer left
/// behind with no request.
pub fn pending(data_dir: &Path) -> std::io::Result<Vec<CoordinatorConfirmation>> {
    let folder = dir(data_dir);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    let mut answers = Vec::new();
    for entry in std::fs::read_dir(&folder)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.ends_with(".answer") || name.ends_with(".answer.tmp") {
            answers.push(path);
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        let Some(stem) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if uuid::Uuid::parse_str(&stem).is_err() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(item) = serde_json::from_str::<CoordinatorConfirmation>(&text) else {
            continue;
        };
        if item.id != stem {
            continue;
        }
        // Past the wait, whether or not something is still touching it: the command has given
        // up, or is about to. A merely quiet file stays, so the command can show it again.
        if older_than_wait(item.created_unix_ms) {
            remove_request(&folder, &stem);
            continue;
        }
        if !fresh(&path) || answer_path(&folder, &item.id).is_file() {
            continue;
        }
        found.push(item);
    }
    for path in answers {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let stem = name.trim_end_matches(".tmp").trim_end_matches(".answer");
        if uuid::Uuid::parse_str(stem).is_err() {
            continue;
        }
        if !folder.join(format!("{stem}.json")).is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
    found.sort_by(|a, b| a.created_unix_ms.total_cmp(&b.created_unix_ms));
    Ok(found)
}

/// Record the user's answer. `Ok(false)` when there is nothing waiting under `id` (it was
/// already answered, it timed out, or `id` is not a request's id). The id is a uuid, so it
/// cannot name a path outside the confirm folder.
pub fn answer(data_dir: &Path, id: &str, allow: bool) -> std::io::Result<bool> {
    let Ok(id) = uuid::Uuid::parse_str(id) else {
        return Ok(false);
    };
    let id = id.to_string();
    let folder = dir(data_dir);
    let json = folder.join(format!("{id}.json"));
    // Gone: the command already stopped, or the wait ran out and the file was removed.
    if !json.is_file() {
        return Ok(false);
    }
    let Ok(text) = std::fs::read_to_string(&json) else {
        return Ok(false);
    };
    let Ok(item) = serde_json::from_str::<CoordinatorConfirmation>(&text) else {
        return Ok(false);
    };
    // Quiet is not the same as gone. A command paused for longer than [`FRESH`] is hidden from
    // the dialog and still waiting; recording the answer is what lets it continue. Only a
    // request older than the wait is past being answered.
    if older_than_wait(item.created_unix_ms) {
        remove_request(&folder, &id);
        return Ok(false);
    }
    let temporary = folder.join(format!("{id}.answer.tmp"));
    std::fs::write(&temporary, if allow { "yes\n" } else { "no\n" })?;
    std::fs::rename(temporary, answer_path(&folder, &id))?;
    Ok(true)
}

fn answer_path(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("{id}.answer"))
}

/// `Some(true)` is yes, `Some(false)` is no, `None` is not written yet.
fn read_answer(folder: &Path, id: &str) -> Option<bool> {
    let text = std::fs::read_to_string(answer_path(folder, id)).ok()?;
    let word = text.trim();
    if word.is_empty() {
        return None;
    }
    Some(word.eq_ignore_ascii_case("yes"))
}

fn not_open(command: &str) -> String {
    format!(
        "The Floor Manager is set to ask first, and Yardsort is not open to confirm this, \
         so it was not run: {command}"
    )
}

fn refused(command: &str) -> String {
    format!("Not run: you refused this in Yardsort: {command}")
}

fn timed_out(command: &str) -> String {
    format!(
        "Not run: nobody answered in Yardsort within ten minutes, so this was not run: {command}"
    )
}

fn withdrawn(command: &str) -> String {
    format!(
        "Not run: the confirmation was withdrawn before it was answered, so this was not run: \
         {command}"
    )
}

/// Bring the file's modification time to now. Opening for write does not truncate.
fn heartbeat(path: &Path) {
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) else {
        return;
    };
    let _ = file.set_modified(SystemTime::now());
}

/// Whether `ys` has touched this file within [`FRESH`].
///
/// A modification time in the future means the clock stepped backwards after the touch. That
/// file was just written from its own point of view, so it is still someone waiting. Treating
/// it as stale would hide a live command until the clock caught up.
fn fresh(path: &Path) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    match modified.elapsed() {
        Ok(age) => age < FRESH,
        Err(_) => true,
    }
}

/// Whether `created_unix_ms` is further in the past than [`wait_for`].
///
/// A clock that steps backwards makes this negative, which is not older: the request stays
/// until the wait has actually passed. The command itself times its wait with a monotonic
/// clock, so a backwards step cannot make it give up early either.
fn older_than_wait(created_unix_ms: f64) -> bool {
    now_ms() - created_unix_ms > wait_for().as_secs_f64() * 1000.0
}

fn remove_request(folder: &Path, id: &str) {
    let _ = std::fs::remove_file(folder.join(format!("{id}.json")));
    let _ = std::fs::remove_file(folder.join(format!("{id}.answer")));
    let _ = std::fs::remove_file(folder.join(format!("{id}.answer.tmp")));
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Tests set `YARDSORT_COORDINATOR_CONFIRM_WAIT` to a few hundred milliseconds so a missed
/// answer fails quickly. A shorter wait only refuses sooner; it cannot turn a no into a yes.
fn wait_for() -> Duration {
    std::env::var(super::CONFIRM_WAIT_ENV)
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(WAIT)
}

/// Removes the request and its answer however `ask` returns, so a refusal leaves nothing for
/// the dialog to keep showing.
struct Cleanup {
    folder: PathBuf,
    id: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.folder.join(format!("{}.json", self.id)));
        let _ = std::fs::remove_file(self.folder.join(format!("{}.answer", self.id)));
        let _ = std::fs::remove_file(self.folder.join(format!("{}.answer.tmp", self.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(dir: &Path) -> presence::AppLock {
        presence::AppLock::take(dir)
            .unwrap()
            .expect("the lock is free")
    }

    #[test]
    fn without_the_app_a_start_is_refused_and_nothing_is_left_waiting() {
        let data = tempfile::tempdir().unwrap();
        let error = ask(data.path(), "ys workspace new demo \"fix\"").unwrap_err();
        assert!(error.contains("not open"), "{error}");
        assert!(
            pending(data.path()).unwrap().is_empty(),
            "a refusal must not leave a dialog up"
        );
    }

    #[test]
    fn a_yes_allows_and_a_no_refuses_and_a_stranger_cannot_name_the_file() {
        let data = tempfile::tempdir().unwrap();
        let _held = hold(data.path());
        let path = data.path().to_path_buf();
        let yes = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let waiting = pending(&path).unwrap();
                if let Some(request) = waiting.first() {
                    assert!(request.command.contains("workspace new"));
                    assert!(answer(&path, &request.id, true).unwrap());
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "no request");
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        ask(data.path(), "ys workspace new demo \"fix\"").expect("the app said yes");
        yes.join().unwrap();

        let path = data.path().to_path_buf();
        let no = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let waiting = pending(&path).unwrap();
                if let Some(request) = waiting.first() {
                    assert!(!answer(&path, "../outside", false).unwrap());
                    assert!(answer(&path, &request.id, false).unwrap());
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "no request");
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let error = ask(data.path(), "ys test run smoke").unwrap_err();
        assert!(error.contains("refused"), "{error}");
        no.join().unwrap();
        assert!(pending(data.path()).unwrap().is_empty());
    }

    /// Write access, not a read-only handle: Windows refuses `set_modified` on the latter.
    fn touch(path: &Path, when: SystemTime) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn write_request(folder: &Path, id: &str, created_unix_ms: f64) -> PathBuf {
        let request = CoordinatorConfirmation {
            id: id.to_owned(),
            command: "ys workspace new demo \"fix\"".into(),
            created_unix_ms,
        };
        let path = folder.join(format!("{id}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
        path
    }

    #[test]
    fn a_quiet_request_is_hidden_until_the_wait_is_over_and_orphan_answers_are_removed() {
        let data = tempfile::tempdir().unwrap();
        let folder = dir(data.path());
        std::fs::create_dir_all(&folder).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = write_request(&folder, &id, now_ms());
        touch(&path, SystemTime::now() - Duration::from_secs(30));

        let orphan = uuid::Uuid::new_v4().to_string();
        let answer_file = folder.join(format!("{orphan}.answer"));
        let temporary = folder.join(format!("{orphan}.answer.tmp"));
        std::fs::write(&answer_file, "yes\n").unwrap();
        std::fs::write(&temporary, "yes\n").unwrap();

        assert!(
            pending(data.path()).unwrap().is_empty(),
            "a quiet request must not stay in the dialog"
        );
        assert!(
            path.exists(),
            "hiding it must leave the command able to resume"
        );
        assert!(!answer_file.exists() && !temporary.exists());
        // Still inside the wait, so the yes is recorded for the command that is paused.
        assert!(answer(data.path(), &id, true).unwrap());
        assert!(answer_path(&folder, &id).is_file());
        assert!(
            path.exists(),
            "answering a quiet request must not delete it"
        );

        let expired = uuid::Uuid::new_v4().to_string();
        let expired_path = write_request(
            &folder,
            &expired,
            now_ms() - wait_for().as_secs_f64() * 1000.0 - 1000.0,
        );
        touch(&expired_path, SystemTime::now());
        assert!(pending(data.path()).unwrap().is_empty());
        assert!(!expired_path.exists(), "past the wait, the file is removed");
        assert!(!answer(data.path(), &expired, true).unwrap());

        // A timestamp in the future is a clock that stepped backwards, not a dead command.
        let rewound = uuid::Uuid::new_v4().to_string();
        let rewound_path = write_request(&folder, &rewound, now_ms() + 60_000.0);
        touch(&rewound_path, SystemTime::now() + Duration::from_secs(3600));
        let waiting = pending(data.path()).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].id, rewound);
        assert!(rewound_path.exists());
    }

    #[test]
    fn a_request_removed_while_waiting_is_refused_without_sitting_out_the_wait() {
        let data = tempfile::tempdir().unwrap();
        let _held = hold(data.path());
        let path = data.path().to_path_buf();
        let remover = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let waiting = pending(&path).unwrap();
                if let Some(request) = waiting.first() {
                    std::fs::remove_file(dir(&path).join(format!("{}.json", request.id))).unwrap();
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "no request");
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let started = std::time::Instant::now();
        let error = ask(data.path(), "ys workspace new demo \"fix\"").unwrap_err();
        assert!(
            error.contains("withdrawn"),
            "the command must stop when its request is gone: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it must not wait out the full window"
        );
        remover.join().unwrap();
    }
}

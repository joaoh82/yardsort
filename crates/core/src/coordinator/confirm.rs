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
/// about to see it. One older than [`WAIT`] is removed, so a crash does not leave a dialog up.
pub fn pending(data_dir: &Path) -> std::io::Result<Vec<CoordinatorConfirmation>> {
    let folder = dir(data_dir);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&folder)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if uuid::Uuid::parse_str(stem).is_err() {
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
        if answer_path(&folder, &item.id).is_file() {
            continue;
        }
        if now_ms() - item.created_unix_ms > wait_for().as_secs_f64() * 1000.0 {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        found.push(item);
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
    if !folder.join(format!("{id}.json")).is_file() {
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

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Tests set `YARDSORT_COORDINATOR_CONFIRM_WAIT` to a few hundred milliseconds so a missed
/// answer fails quickly. A shorter wait only refuses sooner; it cannot turn a no into a yes.
fn wait_for() -> Duration {
    std::env::var("YARDSORT_COORDINATOR_CONFIRM_WAIT")
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
}

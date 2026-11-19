//! What was done through the dashboard, and when: who signed in, what was run and how it ended,
//! what was changed.
//!
//! One sentence an entry, the newest kept. Never what a program printed, and never a sign-in
//! code, a GitHub token or a remote's password, which the dashboard keeps out of a job's title
//! already. A command typed into the page is recorded as typed, so a secret typed into one is
//! in here, as it is in a shell's history.
//! Kept in the profile beside the sessions, for this user alone, so that it outlasts a restart.
//! A record for the operator to read, not a control: someone signed in could run a command that
//! deletes the file.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Entries kept, the newest.
pub const KEPT: usize = 500;
/// The longest sentence kept.
const LONGEST: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// When, in milliseconds since 1970. For one said several times in a row, the last time.
    pub at: i64,
    /// `session`, `job`, `terminal`, `tunnel`, `git`, `device` or `remote`.
    pub kind: String,
    /// What happened, in a sentence.
    pub text: String,
    /// How many times in a row.
    #[serde(default = "once")]
    pub times: u32,
}

fn once() -> u32 {
    1
}

/// How often, at most, the file is written again for the same thing said again.
const REPEATS_WRITTEN_EVERY: Duration = Duration::from_secs(5);

pub struct Activity {
    path: PathBuf,
    kept: Mutex<Kept>,
}

#[derive(Default)]
struct Kept {
    entries: VecDeque<Entry>,
    written: Option<Instant>,
}

impl Activity {
    /// The record in `dir`, with what was kept there. A file that cannot be read starts it anew.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("activity.json");
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<VecDeque<Entry>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            kept: Mutex::new(Kept {
                entries,
                written: None,
            }),
        }
    }

    /// Record that something happened. The same thing said again straight away is counted, not
    /// repeated: a wrong code tried a thousand times is one line. Nor is the file written a
    /// thousand times for it: anyone can try a wrong code, signed in or not, so a repeat is
    /// written at most every few seconds, and with the next thing recorded at the latest.
    pub fn note(&self, kind: &str, text: impl AsRef<str>) {
        let text: String = text.as_ref().chars().take(LONGEST).collect();
        let at = yardsort_core::store::now_ms();
        let mut kept = self.lock();
        let repeat = match kept.entries.back_mut() {
            Some(last) if last.kind == kind && last.text == text => {
                last.at = at;
                last.times = last.times.saturating_add(1);
                true
            }
            _ => {
                kept.entries.push_back(Entry {
                    at,
                    kind: kind.to_owned(),
                    text,
                    times: 1,
                });
                false
            }
        };
        while kept.entries.len() > KEPT {
            kept.entries.pop_front();
        }
        if repeat
            && kept
                .written
                .is_some_and(|written| written.elapsed() < REPEATS_WRITTEN_EVERY)
        {
            return;
        }
        kept.written = Some(Instant::now());
        let written = serde_json::to_vec(&kept.entries)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::auth::write_private(&self.path, &bytes));
        if let Err(error) = written {
            eprintln!(
                "yardsort: the dashboard's activity could not be kept in {}: {error}",
                self.path.display()
            );
        }
    }

    /// Everything kept, the newest first.
    pub fn list(&self) -> Vec<Entry> {
        self.lock().entries.iter().rev().cloned().collect()
    }

    fn lock(&self) -> MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_happened_is_kept_newest_first_and_outlasts_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let activity = Activity::open(dir.path());
        activity.note("session", "A browser signed in, on this computer.");
        activity.note("job", "Started: echo hello");
        let texts = |activity: &Activity| -> Vec<String> {
            activity.list().into_iter().map(|e| e.text).collect()
        };
        assert_eq!(
            texts(&activity),
            [
                "Started: echo hello",
                "A browser signed in, on this computer."
            ]
        );
        drop(activity);
        let again = Activity::open(dir.path());
        assert_eq!(texts(&again).len(), 2);
        assert!(again.list()[0].at > 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("activity.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_same_thing_in_a_row_is_counted_and_only_the_newest_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let activity = Activity::open(dir.path());
        for _ in 0..1000 {
            activity.note(
                "session",
                "A sign-in was refused: not a code this computer gave.",
            );
        }
        let [one] = activity.list().try_into().unwrap();
        assert_eq!(one.times, 1000);
        // The file was not written a thousand times for it: what is in it is the first, and the
        // count catches up with the next thing recorded.
        let on_disk = |dir: &std::path::Path| Activity::open(dir).list();
        assert_eq!(on_disk(dir.path())[0].times, 1);
        activity.note("job", "Started: something else");
        assert_eq!(on_disk(dir.path())[1].times, 1000);
        for n in 0..KEPT + 20 {
            activity.note("job", format!("Started: job {n}"));
        }
        let kept = activity.list();
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept[0].text, format!("Started: job {}", KEPT + 19));
        // The thousand refusals were the oldest, and went first.
        assert!(kept
            .iter()
            .all(|entry| entry.text.starts_with("Started: job ")));
    }

    #[test]
    fn a_sentence_is_cut_short_and_a_broken_file_starts_the_record_anew() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("activity.json"), b"not json").unwrap();
        let activity = Activity::open(dir.path());
        assert!(activity.list().is_empty());
        activity.note("job", "x".repeat(5000));
        assert_eq!(activity.list()[0].text.chars().count(), 300);
    }
}

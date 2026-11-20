//! A project's pull requests, as a device paired with this computer is told of them.
//!
//! The device asks this computer, never the forge, and is told what the window was last shown:
//! the same [`Forge`](super::Forge) answers both. A device's channel also carries its terminal,
//! so nothing here waits for `gh`. A question finds what was last read and has it at once. When
//! that is old, or there is nothing, a reading is started off the channel and the answer says
//! one is on its way; the device asks again and has it.
//!
//! The window reads a project's pull requests while it is in front. This is what keeps them
//! read while only a phone is looking.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use yardsort_core::remote::pull::{FollowedPull, ProjectPulls, PullRequests};

use super::LastRead;
use crate::state::AppState;

/// How old a project's pull requests may be before a device's question has them read again. A
/// phone asks about once a minute while it shows them, so each of its looks is of a reading no
/// older than its last look.
const OLD_AFTER: Duration = Duration::from_secs(45);

/// How soon a project is read again when the last reading left nothing to show: one whose
/// folder is gone, or a project forgotten a moment ago because something was pushed.
const AGAIN_AFTER: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Marks {
    /// Being read now.
    reading: HashSet<String>,
    /// When each project's last reading for a device began.
    began: HashMap<String, Instant>,
    /// When the last one that took in every open one began.
    began_full: HashMap<String, Instant>,
    /// When something a device did last changed each project's pull requests. A reading begun
    /// before then may not have seen it.
    changed: HashMap<String, Instant>,
}

/// Which projects are being read for a device, and when they last were: so a second question
/// does not start a second `gh`, and a reading that cannot succeed is not tried at every one.
#[derive(Default)]
pub struct Readings(Mutex<Marks>);

impl Readings {
    /// Whether to read `project_id` now, and if so whether to take in every open one. `Some`
    /// once for each reading: it is then marked as being read, until [`done`](Self::done).
    fn begin(
        &self,
        project_id: &str,
        last: Option<&LastRead>,
        every_open: bool,
        now: Instant,
    ) -> Option<bool> {
        let mut marks = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if marks.reading.contains(project_id) {
            return None;
        }
        let since = |began: &HashMap<String, Instant>, gap: Duration| {
            began
                .get(project_id)
                .is_none_or(|at| now.saturating_duration_since(*at) >= gap)
        };
        let changed = marks.changed.get(project_id).is_some_and(|changed| {
            marks
                .began
                .get(project_id)
                .is_none_or(|began| began < changed)
        });
        let recent = changed || recent_due(last) && since(&marks.began, AGAIN_AFTER);
        // Every open one cannot be read of a forge `gh` does not speak to, and what was last
        // read never says they were: so it is asked for no more often than anything is read.
        let full = open_due(last, every_open) && since(&marks.began_full, OLD_AFTER);
        if !recent && !full {
            return None;
        }
        marks.reading.insert(project_id.to_owned());
        marks.began.insert(project_id.to_owned(), now);
        if full {
            marks.began_full.insert(project_id.to_owned(), now);
        }
        Some(full)
    }

    /// Something a device did changed `project_id`'s pull requests: the next question reads them
    /// again, however fresh what was read.
    fn changed(&self, project_id: &str, now: Instant) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .changed
            .insert(project_id.to_owned(), now);
    }

    fn done(&self, project_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reading
            .remove(project_id);
    }

    fn reading(&self, project_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reading
            .contains(project_id)
    }
}

/// What was last read is not good enough to go on showing: there is nothing, it is old, or
/// nobody worked out which workspace opened what.
fn recent_due(last: Option<&LastRead>) -> bool {
    last.is_none_or(|last| last.age >= OLD_AFTER || last.workspaces.is_none())
}

/// Every open one is wanted, and those were never read or are old.
fn open_due(last: Option<&LastRead>, every_open: bool) -> bool {
    every_open && last.is_none_or(|last| last.open_age.is_none_or(|age| age >= OLD_AFTER))
}

/// Marks a reading as over when it ends, however it ends.
struct Reading {
    readings: Arc<Readings>,
    project_id: String,
}

impl Drop for Reading {
    fn drop(&mut self) {
        self.readings.done(&self.project_id);
    }
}

/// This app's pull requests, as a device's channel reads them.
pub struct AppPulls {
    app: AppHandle,
    readings: Arc<Readings>,
}

impl AppPulls {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            readings: shared(app),
        }
    }
}

/// The app's one record of readings for devices, kept in its state so that what a device's
/// channel changes elsewhere reaches it.
fn shared(app: &AppHandle) -> Arc<Readings> {
    if let Some(readings) = app.try_state::<Arc<Readings>>() {
        return Arc::clone(&readings);
    }
    let readings = Arc::<Readings>::default();
    app.manage(Arc::clone(&readings));
    Arc::clone(&app.state::<Arc<Readings>>())
}

/// Something a device did changed `project_id`'s pull requests: a merge, a push, one opened.
pub fn changed(app: &AppHandle, project_id: &str) {
    shared(app).changed(project_id, Instant::now());
}

impl PullRequests for AppPulls {
    fn changed(&self, project_id: &str) {
        self.readings.changed(project_id, Instant::now());
    }

    fn of_project(&self, project_id: &str, every_open: bool) -> ProjectPulls {
        let state = self.app.state::<AppState>();
        let last = state.forge.last_read(project_id);
        if let Some(full) =
            self.readings
                .begin(project_id, last.as_ref(), every_open, Instant::now())
        {
            let app = self.app.clone();
            let reading = Reading {
                readings: Arc::clone(&self.readings),
                project_id: project_id.to_owned(),
            };
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<AppState>();
                super::commands::read(&state, &reading.project_id, true, full, false);
                drop(reading);
            });
        }
        let reading = self.readings.reading(project_id);
        let Some(last) = last else {
            return ProjectPulls {
                reading,
                ..Default::default()
            };
        };
        let followed = state
            .store
            .tracked_pull_requests_in_project(project_id)
            .map(|rows| state.forge.tracked_as_read(&rows))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tracked| {
                Some(FollowedPull {
                    id: tracked.id,
                    workspace_id: tracked.workspace_id,
                    repo: format!("{}/{}", tracked.repo.owner, tracked.repo.name),
                    pull_request: tracked.pull_request?,
                })
            })
            .collect();
        ProjectPulls {
            pull_requests: last.found.pull_requests,
            workspaces: last.workspaces.unwrap_or_default(),
            followed,
            reading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::ProjectPullRequests;

    fn read(age: u64, open_age: Option<u64>, workspaces: bool) -> LastRead {
        LastRead {
            age: Duration::from_secs(age),
            open_age: open_age.map(Duration::from_secs),
            found: ProjectPullRequests::default(),
            workspaces: workspaces.then(Default::default),
        }
    }

    #[test]
    fn a_reading_is_due_when_there_is_nothing_or_it_is_old_or_nobody_shared_it_out() {
        assert!(recent_due(None));
        assert!(!recent_due(Some(&read(10, None, true))));
        assert!(recent_due(Some(&read(45, None, true))));
        // Read a moment ago, for the toolbar, which does not ask which workspace opened what.
        assert!(recent_due(Some(&read(1, None, false))));
    }

    #[test]
    fn every_open_one_is_read_when_the_whole_list_is_asked_for_and_those_are_old_or_were_never_read(
    ) {
        assert!(open_due(Some(&read(1, None, true)), true));
        assert!(open_due(Some(&read(1, Some(50), true)), true));
        assert!(!open_due(Some(&read(1, Some(5), true)), true));
        // A workspace list does not ask for them.
        assert!(!open_due(Some(&read(1, None, true)), false));
    }

    #[test]
    fn a_project_being_read_is_not_read_a_second_time_until_that_is_over() {
        let readings = Readings::default();
        let now = Instant::now();
        assert!(!readings.reading("p"));
        assert_eq!(readings.begin("p", None, false, now), Some(false));
        assert!(readings.reading("p"));
        // The phone asks again while `gh` is still away.
        assert_eq!(readings.begin("p", None, true, now), None);
        // Another project is another reading.
        assert_eq!(readings.begin("q", None, true, now), Some(true));
        readings.done("p");
        assert!(!readings.reading("p"));
        // Nothing is marked for one that was not due.
        assert_eq!(
            readings.begin("r", Some(&read(1, None, true)), false, now),
            None
        );
        assert!(!readings.reading("r"));
    }

    #[test]
    fn a_reading_that_left_nothing_to_show_is_tried_again_in_a_while_and_not_at_every_question() {
        let readings = Readings::default();
        let start = Instant::now();
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        // A project whose folder is gone: read, over at once, and still nothing.
        assert_eq!(readings.begin("p", None, false, start), Some(false));
        readings.done("p");
        // The phone asks again four seconds later, and again.
        assert_eq!(readings.begin("p", None, false, after(4)), None);
        assert_eq!(readings.begin("p", None, false, after(8)), None);
        assert_eq!(readings.begin("p", None, false, after(10)), Some(false));
    }

    #[test]
    fn every_open_one_that_cannot_be_read_is_asked_for_no_more_often_than_anything_is() {
        let readings = Readings::default();
        let start = Instant::now();
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        // A forge `gh` does not speak to: read a moment ago, and its open ones never.
        let fresh = read(1, None, true);
        assert_eq!(readings.begin("p", Some(&fresh), true, start), Some(true));
        readings.done("p");
        // The list is still open on the phone, which asks again.
        assert_eq!(readings.begin("p", Some(&fresh), true, after(4)), None);
        assert_eq!(readings.begin("p", Some(&fresh), true, after(30)), None);
        assert_eq!(
            readings.begin("p", Some(&fresh), true, after(45)),
            Some(true)
        );
        readings.done("p");
        // Meanwhile what was read grew old: that is read, without every open one again.
        let old = read(60, None, true);
        assert_eq!(
            readings.begin("p", Some(&old), true, after(60)),
            Some(false)
        );
    }

    #[test]
    fn a_reading_that_ends_badly_is_still_over() {
        let readings = Arc::new(Readings::default());
        assert!(readings.begin("p", None, false, Instant::now()).is_some());
        let reading = Reading {
            readings: Arc::clone(&readings),
            project_id: "p".into(),
        };
        let stopped = std::thread::spawn(move || {
            let _held = reading;
            panic!("gh went away");
        })
        .join();
        assert!(stopped.is_err());
        assert!(!readings.reading("p"));
    }

    #[test]
    fn a_change_a_device_made_has_the_next_question_read_again_however_fresh() {
        let readings = Readings::default();
        let start = Instant::now();
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        let fresh = read(1, None, true);
        assert_eq!(readings.begin("p", None, false, start), Some(false));
        // Merged while that reading was still away.
        readings.changed("p", after(1));
        readings.done("p");
        assert_eq!(
            readings.begin("p", Some(&fresh), false, after(2)),
            Some(false)
        );
        readings.done("p");
        // Read since the change: fresh again.
        assert_eq!(readings.begin("p", Some(&fresh), false, after(3)), None);
    }
}

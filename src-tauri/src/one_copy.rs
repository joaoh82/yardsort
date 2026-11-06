//! One copy of the app per profile.
//!
//! Each copy is a whole app on one database: it settles the same records, and when Remote is on it
//! keeps its own link to the server under the same sign-in. That sign-in works once and is replaced
//! each time, and the server takes one presented twice for a stolen one, so a second copy gets the
//! computer signed out. Starting Yardsort again therefore brings up the window it already has.
//!
//! By profile, not by app: each worktree runs a development build on a profile of its own, and
//! those must not stop each other. It is the lock that also tells `ys` the app is running and lets
//! this copy drive workflow runs ([`AppLock`]), and the OS lets go of it however the process ends,
//! so a crash leaves nothing to clean up. Nothing a copy starts holds it: the file is not inherited
//! by the programs it runs, the daemon among them.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use yardsort_core::presence::AppLock;

/// How long a new copy waits to be answered, or for a copy on its way out to go. A restart starts
/// the new copy first, and that one is in no hurry to give up.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// What is said when the copy that has the profile does not answer.
pub const STUCK: &str =
    "Yardsort is already open on this profile, and is not answering. Close it, \
                         or end its process if it will not close, then start Yardsort again.";

/// How often the first copy looks for a request, and a new one for its answer.
const LOOK_EVERY: Duration = Duration::from_millis(200);

/// On its way out: a request is left for whichever copy comes next, and is not answered.
static LEAVING: AtomicBool = AtomicBool::new(false);

/// Called before this copy goes, whether it is closing or restarting. Told to show its window from
/// here on, it would say yes, and the copy asking would give way to one that is about to be gone.
pub fn leaving() {
    LEAVING.store(true, Ordering::SeqCst);
}

/// Where this copy stands.
#[derive(Debug)]
pub enum Claim {
    /// The profile is this copy's.
    Ours(Hold),
    /// Another copy has it, and has shown its window.
    Shown,
    /// Another copy has it, and did not answer.
    Stuck,
    /// The lock could not be tried: an unusual file system, say. Refusing to start over that would
    /// be worse than what it guards against.
    Unguarded(String),
}

/// This copy's claim on the profile, for as long as the process lives.
#[derive(Debug)]
pub struct Hold {
    _lock: AppLock,
    request: PathBuf,
}

/// Take the profile, or ask whoever has it to show its window.
pub fn claim(data_dir: &Path, patience: Duration) -> Claim {
    let request = data_dir.join("app.show");
    let deadline = Instant::now() + patience;
    let mut asked = false;
    loop {
        match AppLock::take(data_dir) {
            Ok(Some(lock)) => {
                // Left by a copy that asked of one that was going.
                let _ = fs::remove_file(&request);
                return Claim::Ours(Hold {
                    _lock: lock,
                    request,
                });
            }
            Ok(None) => {}
            Err(error) => return Claim::Unguarded(error.to_string()),
        }
        if !asked {
            if let Err(error) = fs::write(&request, "") {
                return Claim::Unguarded(error.to_string());
            }
            asked = true;
        } else if !request.exists() {
            return Claim::Shown;
        }
        if Instant::now() >= deadline {
            let _ = fs::remove_file(&request);
            return Claim::Stuck;
        }
        std::thread::sleep(LOOK_EVERY.min(patience));
    }
}

impl Hold {
    /// Answer a new copy's request, if there is one. Unanswered, it goes on waiting for the lock.
    fn answer(&self, going: bool, show: impl FnOnce() -> bool) {
        if !going && self.request.exists() && show() {
            let _ = fs::remove_file(&self.request);
        }
    }

    /// Answer new copies with the window, until the process ends.
    pub fn watch(self, app: AppHandle) {
        // Kept by the app too: with no thread to answer, this is still the copy that has the profile.
        let hold = Arc::new(self);
        app.manage(Arc::clone(&hold));
        let spawned = std::thread::Builder::new()
            .name("one-copy".into())
            .spawn(move || loop {
                hold.answer(LEAVING.load(Ordering::SeqCst), || show_window(&app));
                std::thread::sleep(LOOK_EVERY);
            });
        if let Err(error) = spawned {
            eprintln!("could not watch for another copy starting: {error}");
        }
    }
}

/// False with no window to show, which is for the new copy to say.
fn show_window(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOON: Duration = Duration::from_millis(600);

    fn ours(claim: Claim) -> Hold {
        match claim {
            Claim::Ours(hold) => hold,
            other => panic!("not ours: {other:?}"),
        }
    }

    /// The lock `ys` looks for and workflow runs depend on. A lock of its own on the same file
    /// shut the driver out of the very copy that held it.
    #[test]
    fn having_the_profile_is_being_the_running_app() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!yardsort_core::presence::app_running(dir.path()));
        let first = ours(claim(dir.path(), SOON));
        assert!(yardsort_core::presence::app_running(dir.path()));
        drop(first);
        assert!(!yardsort_core::presence::app_running(dir.path()));
    }

    #[test]
    fn the_first_copy_has_the_profile_and_one_with_no_answer_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let _first = ours(claim(dir.path(), SOON));
        assert!(matches!(claim(dir.path(), SOON), Claim::Stuck));
        assert!(
            !dir.path().join("app.show").exists(),
            "and takes its request back"
        );
    }

    #[test]
    fn a_second_copy_gets_the_first_ones_window_and_does_not_start() {
        let dir = tempfile::tempdir().unwrap();
        let first = ours(claim(dir.path(), SOON));
        let path = dir.path().to_owned();
        let second = std::thread::spawn(move || claim(&path, Duration::from_secs(30)));
        let mut shown = 0;
        while !second.is_finished() {
            first.answer(false, || {
                shown += 1;
                true
            });
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(second.join().unwrap(), Claim::Shown));
        assert_eq!(shown, 1);
    }

    #[test]
    fn a_copy_with_no_window_to_show_does_not_turn_the_new_one_away() {
        let dir = tempfile::tempdir().unwrap();
        let first = ours(claim(dir.path(), SOON));
        let path = dir.path().to_owned();
        let second = std::thread::spawn(move || claim(&path, SOON));
        while !second.is_finished() {
            first.answer(false, || false);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(second.join().unwrap(), Claim::Stuck));
    }

    /// A restart starts the new copy while the old one is still closing. If the old one answered,
    /// the new one would give way to a window about to be gone, and nothing would be left open.
    #[test]
    fn a_copy_on_its_way_out_leaves_the_profile_to_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let first = ours(claim(dir.path(), SOON));
        let path = dir.path().to_owned();
        let next = std::thread::spawn(move || claim(&path, Duration::from_secs(30)));
        while !dir.path().join("app.show").exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
        first.answer(true, || panic!("it is going"));
        assert!(!next.is_finished());
        drop(first);
        ours(next.join().unwrap());
        assert!(
            !dir.path().join("app.show").exists(),
            "and nothing is left to answer"
        );
    }

    /// The daemon outlives the app that started it. If it kept the lock, closing Yardsort once
    /// would mean it could never start again.
    #[test]
    fn a_program_it_started_does_not_keep_the_profile_from_the_next_copy() {
        let dir = tempfile::tempdir().unwrap();
        let first = ours(claim(dir.path(), SOON));
        let mut outliving = if cfg!(windows) {
            std::process::Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
        } else {
            std::process::Command::new("sleep").arg("30").spawn()
        }
        .unwrap();
        drop(first);
        let next = claim(dir.path(), SOON);
        let _ = outliving.kill();
        let _ = outliving.wait();
        ours(next);
    }

    #[test]
    fn profiles_do_not_stop_each_other() {
        let (one, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _one = ours(claim(one.path(), SOON));
        let _other = ours(claim(other.path(), SOON));
    }

    #[test]
    fn the_profile_is_there_to_take_once_its_holder_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        drop(ours(claim(&dir.path().join("not/yet/made"), SOON)));
        let _ = ours(claim(&dir.path().join("not/yet/made"), SOON));
    }
}

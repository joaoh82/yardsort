//! Whether the app is running for a profile.
//!
//! The app holds an exclusive lock on `app.lock` in its data directory for as long as it runs.
//! `ys` asks by trying to take the same lock without waiting: if it cannot, the app has it. The
//! operating system drops the lock when the process ends, however it ends, so a crash never
//! leaves a stale "running" behind — which a file that merely exists would. The lock also means
//! only one app drives a profile's workflow runs, even if two were started on it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

const FILE: &str = "app.lock";

pub fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Held by the running app. Dropping it releases the lock.
#[derive(Debug)]
pub struct AppLock {
    _file: File,
}

impl AppLock {
    /// Take the profile's lock. `Ok(None)` when another process holds it.
    pub fn take(data_dir: &Path) -> std::io::Result<Option<Self>> {
        let file = open(data_dir)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

/// Whether some process holds the profile's lock: whether the app is running. Takes the lock
/// for a moment when it is free, and lets it go.
pub fn app_running(data_dir: &Path) -> bool {
    matches!(AppLock::take(data_dir), Ok(None))
}

fn open(data_dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(data_dir)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_seen_while_held_and_gone_once_dropped() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!app_running(dir.path()), "nobody holds it yet");
        let held = take_within_a_moment(dir.path()).expect("free, so taken");
        assert!(app_running(dir.path()));
        assert!(
            AppLock::take(dir.path()).unwrap().is_none(),
            "a second app on the same profile does not get it"
        );
        drop(held);
        assert!(
            take_within_a_moment(dir.path()).is_some(),
            "released once the holder is gone"
        );
        assert!(
            lock_path(dir.path()).exists(),
            "the file stays; only the lock matters"
        );
    }

    /// The lock goes with the file description, and a child another test forks in the same
    /// moment inherits that description until it execs, so a just-dropped lock can be seen held
    /// briefly after the parent drops it. This also applies to the short-lived lock taken by
    /// `app_running` above. Retry acquisition itself, retaining the guard when it succeeds:
    /// checking for freedom and then taking it would introduce another probe/drop race.
    fn take_within_a_moment(dir: &Path) -> Option<AppLock> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(held) = AppLock::take(dir).unwrap() {
                return Some(held);
            }
            if std::time::Instant::now() > until {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn asking_does_not_take_the_lock_away_from_the_next_app() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!app_running(dir.path()));
        assert!(take_within_a_moment(dir.path()).is_some());
    }
}

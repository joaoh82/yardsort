//! One module per noun.

pub mod activity;
pub mod attach;
pub mod doctor;
pub mod event;
pub mod logs;
pub mod memory;
pub mod project;
pub mod schedule;
pub mod session;
pub mod task;
pub mod test;
pub mod trigger;
pub mod workflow;
pub mod workspace;

use yardsort_core::presence::{self, Holder};

use crate::{Failure, Yardsort};

/// The app carries runs out. Queued with nobody to run it, a run would only sit there, and a host
/// with no screen does not run them yet. `not_running` says what to do when nothing holds the
/// profile.
pub fn someone_carries_it_out(ys: &Yardsort, not_running: &str) -> Result<(), Failure> {
    match presence::holder(&ys.data_dir) {
        Some(Holder::App) => Ok(()),
        Some(Holder::Host) => Err(Failure::new(HEADLESS)),
        None => Err(Failure::new(not_running)),
    }
}

pub const HEADLESS: &str = "This host does not run workflows, smoke tests or schedules yet: \
                            yardsort-host is running it, not the app. Nothing was queued.";

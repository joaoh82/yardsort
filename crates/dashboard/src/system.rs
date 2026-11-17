//! The computer itself: what runs it, how busy it is, and how full.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub host_name: Option<String>,
    /// Such as `Ubuntu 24.04`.
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub arch: String,
    /// Seconds since it started.
    pub uptime: u64,
    pub cpus: usize,
    /// How busy the processors were since the last look, 0 to 100. `None` at the first look,
    /// when there is nothing to measure from.
    pub cpu_percent: Option<f32>,
    /// The load over one, five and fifteen minutes. `None` on Windows, which has no such number.
    pub load: Option<[f64; 3]>,
    pub memory: Amount,
    pub swap: Amount,
    pub disks: Vec<Disk>,
}

/// Bytes used, of how many.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Amount {
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    /// What is kept on it, in words: `Home`, `Workspaces`.
    pub holds: String,
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

/// Reads the computer, keeping what the last look measured so the next can say how busy the
/// processors were in between.
pub struct Reader {
    system: Mutex<(System, bool)>,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            system: Mutex::new((System::new(), false)),
        }
    }
}

impl Reader {
    /// The computer now. `places` are the folders whose disks matter, each with what it holds.
    pub fn read(&self, places: &[(&str, PathBuf)]) -> Machine {
        let mut guard = self.system.lock().unwrap_or_else(PoisonError::into_inner);
        let (system, looked) = &mut *guard;
        system.refresh_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let cpu_percent = looked.then(|| system.global_cpu_usage());
        *looked = true;
        let load = (!cfg!(windows)).then(|| {
            let load = System::load_average();
            [load.one, load.five, load.fifteen]
        });
        Machine {
            host_name: System::host_name(),
            os: match (System::name(), System::os_version()) {
                (Some(name), Some(version)) => Some(format!("{name} {version}")),
                (name, _) => name,
            },
            kernel: System::kernel_version(),
            arch: System::cpu_arch(),
            uptime: System::uptime(),
            cpus: system.cpus().len(),
            cpu_percent,
            load,
            memory: Amount {
                used: system.used_memory(),
                total: system.total_memory(),
            },
            swap: Amount {
                used: system.used_swap(),
                total: system.total_swap(),
            },
            disks: disks(places),
        }
    }
}

/// The disk under each of `places`, once each: two folders on one disk are one line.
fn disks(places: &[(&str, PathBuf)]) -> Vec<Disk> {
    let mounted = Disks::new_with_refreshed_list();
    let mut found: Vec<Disk> = Vec::new();
    for (holds, place) in places {
        // A folder not made yet is on the disk its nearest made parent is on.
        let place = dunce::canonicalize(nearest_made(place)).unwrap_or_else(|_| place.clone());
        let Some(disk) = mounted
            .list()
            .iter()
            .filter(|disk| place.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len())
        else {
            continue;
        };
        let mount = disk.mount_point().to_string_lossy().into_owned();
        if let Some(same) = found.iter_mut().find(|d| d.mount == mount) {
            same.holds = format!("{} and {}", same.holds, holds.to_lowercase());
            continue;
        }
        found.push(Disk {
            holds: (*holds).to_owned(),
            mount,
            used: disk.total_space().saturating_sub(disk.available_space()),
            total: disk.total_space(),
        });
    }
    found
}

fn nearest_made(path: &Path) -> &Path {
    let mut at = path;
    while !at.exists() {
        match at.parent() {
            Some(parent) => at = parent,
            None => break,
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_computer_is_read_and_the_second_look_says_how_busy_it_was() {
        let reader = Reader::default();
        let dir = tempfile::tempdir().unwrap();
        let places = [
            ("Home", dir.path().to_path_buf()),
            ("Workspaces", dir.path().join("not/made/yet")),
        ];
        let first = reader.read(&places);
        assert!(first.cpu_percent.is_none(), "nothing to measure from yet");
        assert!(first.cpus > 0);
        assert!(first.memory.total > 0 && first.memory.used <= first.memory.total);
        let second = reader.read(&places);
        assert!(second.cpu_percent.is_some());
        // Both folders are on the one disk, so it is listed once, saying both.
        if let [disk] = second.disks.as_slice() {
            assert_eq!(disk.holds, "Home and workspaces");
            assert!(disk.total >= disk.used);
        } else {
            // A sandbox may hide the mount table; then there is simply nothing to list.
            assert!(second.disks.is_empty(), "{:?}", second.disks);
        }
    }
}

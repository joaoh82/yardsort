//! Where this host keeps its things: the same rules as the app and `ys`, so all three agree.

use std::path::PathBuf;

use yardsort_core::paths;

/// A directory to keep the host's credential in. Without it, `<data dir>/credentials`: a computer
/// with no desktop session has no credential store to ask.
pub const CREDENTIALS_DIR_VAR: &str = "YARDSORT_CREDENTIALS_DIR";

pub struct Profile {
    /// The database, the daemon's socket and log, and the host's control socket.
    pub data_dir: PathBuf,
    /// `settings.toml`.
    pub config_dir: PathBuf,
    pub credentials_dir: PathBuf,
}

impl Profile {
    /// `--data-dir` wins, then `YARDSORT_DATA_DIR`, then the app's own place.
    pub fn resolve(data_dir: Option<PathBuf>) -> Result<Self, String> {
        if let Some(dir) = data_dir {
            let dir = std::path::absolute(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            // Set, not only used: the daemon, agents' hooks and `ys` find the profile by it.
            // Before any thread is started, which is what makes this sound.
            std::env::set_var("YARDSORT_DATA_DIR", &dir);
        }
        let data_dir = paths::data_dir()
            .ok_or("Cannot work out where Yardsort keeps its data. Pass --data-dir.")?;
        let config_dir = paths::config_dir().unwrap_or_else(|| data_dir.clone());
        let credentials_dir = match std::env::var_os(CREDENTIALS_DIR_VAR) {
            Some(dir) => PathBuf::from(dir),
            None => data_dir.join("credentials"),
        };
        Ok(Self {
            data_dir,
            config_dir,
            credentials_dir,
        })
    }

    /// Where `serve` may listen for this computer's own commands, in the order it tries them:
    /// as the daemon's socket, in the per-user runtime directory or the data directory. Never a
    /// network port.
    pub fn control_sockets(&self) -> Vec<PathBuf> {
        pty_ipc::socket_candidates(&self.data_dir, "host")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("yardsort.db")
    }

    pub fn settings(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }
}

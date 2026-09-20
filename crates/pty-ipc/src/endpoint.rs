//! Where to find the daemon.
//!
//! One daemon per **data directory**, so `YARDSORT_DATA_DIR` isolates experiments and tests the
//! same way it isolates the database. Both sides derive the address from that directory, which
//! is also how an app that has just been updated finds the daemon started by the previous
//! version — it has to, or it could not ask about the agents still running under it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name, ToFsName, ToNsName};

/// The address of one daemon: a socket file on Unix, a named pipe on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A path in the filesystem. Its directory's permissions are what keep other users out.
    Path(PathBuf),
    /// A name in the OS namespace (Windows named pipes).
    Namespaced(String),
}

/// Unix socket paths are limited to ~104 bytes — far shorter than any other path limit, and
/// `YARDSORT_DATA_DIR` can be anywhere. Past this we fall back to the temp directory.
const MAX_SOCKET_PATH: usize = 92;

impl Endpoint {
    /// The daemon that owns the terminals for this data directory.
    ///
    /// Windows gets a named pipe, whose default security descriptor already limits it to this
    /// user's logon session. Everywhere else it is a socket *file* — deliberately, even on Linux,
    /// where an abstract-namespace socket would also work: anything that can connect here can
    /// type into an agent's terminal, and a private directory is what keeps that to its owner.
    pub fn for_data_dir(data_dir: &Path) -> Self {
        let tag = tag_for(data_dir);
        if cfg!(windows) {
            return Self::Namespaced(format!("yardsort-{tag}.sock"));
        }
        // A per-user runtime directory is the tidiest home for a socket: it is already private,
        // and the OS clears it when the session ends.
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("yardsort"));
        let candidates = [
            runtime.map(|dir| dir.join(format!("{tag}.sock"))),
            Some(data_dir.join("daemon.sock")),
            Some(std::env::temp_dir().join(format!("yardsort-{tag}.sock"))),
        ];
        let path = candidates
            .into_iter()
            .flatten()
            .find(|path| path.as_os_str().len() <= MAX_SOCKET_PATH)
            // Every candidate is too long: the last one is still the shortest we can offer, and
            // failing loudly at bind time beats guessing.
            .unwrap_or_else(|| std::env::temp_dir().join(format!("yardsort-{tag}.sock")));
        Self::Path(path)
    }

    /// Parse what the app passed the daemon on its command line.
    pub fn parse(value: &OsStr) -> Self {
        let path = Path::new(value);
        if cfg!(windows) && !path.is_absolute() {
            Self::Namespaced(value.to_string_lossy().into_owned())
        } else {
            Self::Path(path.to_path_buf())
        }
    }

    /// How to write this endpoint back onto a command line.
    pub fn as_os_str(&self) -> &OsStr {
        match self {
            Self::Path(path) => path.as_os_str(),
            Self::Namespaced(name) => OsStr::new(name),
        }
    }

    pub(crate) fn to_name(&self) -> std::io::Result<Name<'_>> {
        match self {
            Self::Path(path) => path.as_os_str().to_fs_name::<GenericFilePath>(),
            Self::Namespaced(name) => name.as_str().to_ns_name::<GenericNamespaced>(),
        }
    }

    /// The directory that must exist, and be private, before the daemon can bind.
    pub fn parent_dir(&self) -> Option<&Path> {
        match self {
            Self::Path(path) => path.parent(),
            Self::Namespaced(_) => None,
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "{}", path.display()),
            Self::Namespaced(name) => f.write_str(name),
        }
    }
}

/// FNV-1a over the directory's bytes. Hand-rolled on purpose: `DefaultHasher` is explicitly not
/// stable between Rust releases, and an app must compute the same address as a daemon that a
/// *different build* started, or an update would orphan the agents it is meant to ask about.
fn tag_for(data_dir: &Path) -> String {
    let bytes = data_dir.as_os_str().as_encoded_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_data_directory_gets_its_own_daemon() {
        let one = Endpoint::for_data_dir(Path::new("/tmp/ys"));
        let two = Endpoint::for_data_dir(Path::new("/tmp/ys-other"));
        assert_ne!(one, two);
        assert_eq!(one, Endpoint::for_data_dir(Path::new("/tmp/ys")));
    }

    /// The tag is part of the address two different builds have to agree on, so it is frozen.
    #[test]
    fn the_tag_is_a_fixed_function_of_the_path() {
        assert_eq!(tag_for(Path::new("/tmp/ys")), "40745e22fb904384".to_owned());
    }

    #[cfg(unix)]
    #[test]
    fn an_over_long_data_directory_falls_back_to_a_short_path() {
        let deep = PathBuf::from(format!("/tmp/{}", "d".repeat(200)));
        let Endpoint::Path(path) = Endpoint::for_data_dir(&deep) else {
            panic!("unix uses socket files");
        };
        assert!(
            path.as_os_str().len() <= MAX_SOCKET_PATH,
            "{} is longer than a unix socket path may be",
            path.display()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_survives_the_command_line() {
        let endpoint = Endpoint::Path("/tmp/ys/daemon.sock".into());
        assert_eq!(Endpoint::parse(endpoint.as_os_str()), endpoint);
    }
}

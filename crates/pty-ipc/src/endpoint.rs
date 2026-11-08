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
        let runtime = runtime_dir().map(|dir| dir.join("yardsort"));
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

/// Where another of a profile's own sockets, called `name`, may go, by the daemon's rule: the
/// per-user runtime directory, then the data directory, then the temporary directory, leaving out
/// any too long to bind. In that order: whoever listens takes the first, and whoever connects tries
/// each, since the two may not see the same `XDG_RUNTIME_DIR`.
pub fn socket_candidates(data_dir: &Path, name: &str) -> Vec<PathBuf> {
    let tag = tag_for(data_dir);
    let runtime = runtime_dir().map(|dir| dir.join("yardsort").join(format!("{tag}.{name}.sock")));
    [
        runtime,
        Some(data_dir.join(format!("{name}.sock"))),
        Some(std::env::temp_dir().join(format!("yardsort-{tag}.{name}.sock"))),
    ]
    .into_iter()
    .flatten()
    .filter(|path| path.as_os_str().len() <= MAX_SOCKET_PATH)
    .collect()
}

/// The per-user runtime directory: `XDG_RUNTIME_DIR`, or on Linux, where that is not set, the one
/// logind makes for this user, `/run/user/<uid>`, if it is there and this user's alone. A shell
/// from `sudo -iu` or `su` has no `XDG_RUNTIME_DIR` while the user's systemd services do, and
/// both must still reach the same daemon: one that missed it would start a second, and take the
/// first one's agents for ended.
fn runtime_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        return Some(dir);
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `geteuid` cannot fail.
        let uid = unsafe { libc::geteuid() };
        private_to(&PathBuf::from(format!("/run/user/{uid}")), uid)
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// `dir`, if it is a directory that `uid` owns and nobody else may enter.
#[cfg(target_os = "linux")]
fn private_to(dir: &Path, uid: u32) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(dir).ok()?;
    (meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0).then(|| dir.to_path_buf())
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
    fn a_profiles_other_sockets_follow_the_daemons_rule_and_fit() {
        let short = Path::new("/tmp/ys");
        let found = socket_candidates(short, "host");
        assert!(found.contains(&short.join("host.sock")), "{found:?}");
        let long = PathBuf::from(format!("/tmp/{}", "x".repeat(120)));
        let found = socket_candidates(&long, "host");
        assert!(!found.is_empty());
        assert!(found
            .iter()
            .all(|path| path.as_os_str().len() <= MAX_SOCKET_PATH && !path.starts_with(&long)));
    }

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

    #[cfg(target_os = "linux")]
    #[test]
    fn logind_s_runtime_directory_counts_only_when_it_is_this_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: `geteuid` cannot fail.
        let uid = unsafe { libc::geteuid() };
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(private_to(dir.path(), uid), Some(dir.path().to_path_buf()));
        assert_eq!(private_to(dir.path(), uid + 1), None, "another user's");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(private_to(dir.path(), uid), None, "others may enter it");
        assert_eq!(private_to(&dir.path().join("gone"), uid), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_survives_the_command_line() {
        let endpoint = Endpoint::Path("/tmp/ys/daemon.sock".into());
        assert_eq!(Endpoint::parse(endpoint.as_os_str()), endpoint);
    }
}

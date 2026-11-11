//! The control socket: how this computer's own commands (`pair`, `devices`, `revoke`, `status`,
//! `sign-out`) ask the running host, which alone holds the sign-in. One JSON message per line.
//!
//! It is `<data dir>/host.sock`, readable and writable by this user only. Nothing listens on the
//! network: not this, and not the daemon.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use crate::profile::Profile;

/// What a command asks.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "camelCase")]
pub enum Ask {
    Status,
    Devices,
    Revoke {
        device: String,
    },
    /// Make an offer, then tell of each device that asks. Refused unless a person at a terminal
    /// asked (`guard`).
    Pair,
    /// On a `Pair` connection: the operator's answer to the device last told of.
    Answer {
        grant: String,
        allow: bool,
        control: bool,
        /// Approving or answering workflows. Absent on an older control client, which does not grant it.
        #[serde(default)]
        workflows: bool,
        /// Only these projects, by id; all of them when absent.
        projects: Option<Vec<String>>,
    },
    SignOut,
}

/// What the host answers.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "camelCase")]
pub enum Reply {
    Status(Status),
    Devices {
        devices: Vec<Device>,
    },
    Revoked {
        name: String,
    },
    Offer {
        code: String,
        link: String,
        expires_in_ms: u32,
    },
    /// A device asks to be let in.
    Asked {
        grant: String,
        device: String,
        wants_control: bool,
        /// It asked to read or decide a workflow. Absent from an older host, which did not say.
        #[serde(default)]
        wants_workflows: bool,
        /// The projects to choose among, when there is more than one.
        projects: Vec<Project>,
    },
    Answered {
        allowed: bool,
    },
    /// The offer lapsed with nobody asking.
    Lapsed,
    SignedOut,
    Failed {
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub remote: Option<RemoteStatus>,
    /// Sessions running in the daemon.
    pub terminals: usize,
    /// Set when the daemon is from an earlier release, kept for the agents in it.
    #[serde(default)]
    pub earlier_daemon: Option<EarlierDaemon>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EarlierDaemon {
    /// Its release, when it said.
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteStatus {
    pub server_url: String,
    pub name: String,
    /// The link to the relay, in words.
    pub link: String,
    pub devices: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Device {
    pub grant_id: String,
    pub name: String,
    pub control: bool,
    /// The projects it reaches, by name; `None` for all of them.
    pub projects: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

/// One end of a connection, either side.
pub struct Line {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

impl Line {
    pub fn new(stream: UnixStream) -> Self {
        let (read, write) = stream.into_split();
        Self {
            lines: BufReader::new(read).lines(),
            write,
        }
    }

    pub async fn send(&mut self, message: &impl Serialize) -> Result<(), String> {
        let mut line = serde_json::to_string(message).map_err(|e| e.to_string())?;
        line.push('\n');
        self.write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("The host went away: {e}"))
    }

    /// The next message, or `None` once the other side is gone.
    pub async fn next<T: for<'de> Deserialize<'de>>(&mut self) -> Result<Option<T>, String> {
        match self.lines.next_line().await {
            Ok(Some(line)) => serde_json::from_str(&line)
                .map(Some)
                .map_err(|e| format!("Not understood: {e}")),
            Ok(None) => Ok(None),
            Err(e) => Err(format!("The connection broke: {e}")),
        }
    }
}

const NOT_SERVING: &str = "The host is not running on this profile";

/// Whether `why`, from [`connect`], says nothing is serving.
pub fn not_serving(why: &str) -> bool {
    why.starts_with(NOT_SERVING)
}

/// Reach the running host, wherever it listens.
pub async fn connect(profile: &Profile) -> Result<Line, String> {
    let candidates = profile.control_sockets();
    for path in &candidates {
        if let Ok(stream) = UnixStream::connect(path).await {
            return Ok(Line::new(stream));
        }
    }
    Err(format!(
        "{NOT_SERVING} ({}). Start it with `yardsort-host serve`.",
        profile.data_dir.display()
    ))
}

/// Ask once, and take the one answer. A `Failed` answer is the error.
pub async fn ask_once(profile: &Profile, ask: Ask) -> Result<Reply, String> {
    let mut line = connect(profile).await?;
    line.send(&ask).await?;
    match line.next::<Reply>().await? {
        Some(Reply::Failed { message }) => Err(message),
        Some(reply) => Ok(reply),
        None => Err("The host closed the connection without answering.".into()),
    }
}

/// Listen at the first of `candidates` that can be bound, made so only this user can reach it.
/// Only call while holding the profile's lock: a socket left there is from a host that is gone,
/// and is replaced. Returns where it listens.
pub fn listen(
    candidates: &[PathBuf],
) -> Result<(std::os::unix::net::UnixListener, PathBuf), String> {
    let mut why = String::from("no place short enough for a socket");
    for path in candidates {
        match listen_at(path) {
            Ok(listener) => return Ok((listener, path.clone())),
            Err(error) => why = error,
        }
    }
    Err(why)
}

fn listen_at(path: &Path) -> Result<std::os::unix::net::UnixListener, String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(path);
    // Made with no access for anyone else from the start, not narrowed after.
    // SAFETY: `umask` only swaps the process's file mode mask; it cannot fail.
    let before = unsafe { libc::umask(0o177) };
    let bound = std::os::unix::net::UnixListener::bind(path);
    unsafe { libc::umask(before) };
    let listener = bound.map_err(|e| format!("Cannot listen at {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Cannot make {} private: {e}", path.display()))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_this_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.sock");
        std::fs::write(&path, "left behind").unwrap();
        let (_listener, at) = listen(std::slice::from_ref(&path)).unwrap();
        assert_eq!(at, path);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn messages_are_one_line_each_and_tagged() {
        let ask = serde_json::to_string(&Ask::Revoke {
            device: "Phone".into(),
        })
        .unwrap();
        assert_eq!(ask, r#"{"ask":"revoke","device":"Phone"}"#);
        let reply: Reply = serde_json::from_str(r#"{"reply":"answered","allowed":true}"#).unwrap();
        assert!(matches!(reply, Reply::Answered { allowed: true }));
    }
}

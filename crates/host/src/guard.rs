//! Who may approve a device here: a person at a terminal on this computer, not an agent.
//!
//! **A guard, not a boundary.** A process running as this user can read everything the host can,
//! its kept sign-in included. What this stops is an agent approving a device by accident, or
//! because something it read told it to: approval is not in any API an agent is given, and asking
//! for it from a terminal Yardsort started is refused. The desktop's rule is the same strength.
//!
//! The caller is checked by what the operating system says about the socket's other end, never by
//! what the caller says about itself: its user is this one; it carries none of the markers a
//! launched session has; its standard input is a terminal, and not one of the daemon's; and it
//! does not descend from the daemon. Linux only, where `/proc` says all that. Elsewhere approving
//! is refused, since none of it could be checked.

use std::os::fd::AsRawFd;

use tokio::net::UnixStream;

/// Why the process at the other end of `stream` may not approve, or `None` if it may.
/// `daemon` is the daemon's process id, when there is one.
pub fn refusal(stream: &UnixStream, daemon: Option<u32>) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::refusal(stream.as_raw_fd(), daemon)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (stream.as_raw_fd(), daemon);
        Some("Pairing from a terminal needs Linux, where the host can check who asks.".into())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use yardsort_core::activity::{RECORD_ENV, RUN_ENV, WORKSPACE_ENV};

    const AGENT: &str = "Pairing is refused from a terminal Yardsort started. A device is let in \
                         only by a person, from a terminal of their own on this computer \
                         (an SSH login, say), never by an agent.";

    pub fn refusal(fd: i32, daemon: Option<u32>) -> Option<String> {
        let Some((pid, uid)) = peer(fd) else {
            return Some("Cannot tell who is asking.".into());
        };
        // SAFETY: `geteuid` cannot fail.
        if uid != unsafe { libc::geteuid() } {
            return Some("Only this host's own user may pair a device.".into());
        }
        let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        if launched(&environ) {
            return Some(AGENT.into());
        }
        let Some(tty) = terminal_of(pid) else {
            return Some(
                "Pairing needs a person at a terminal: run `yardsort-host pair` in one, not \
                 from a script."
                    .into(),
            );
        };
        if let Some(daemon) = daemon {
            if ancestors(pid).contains(&daemon) || daemons_terminals(daemon).contains(&tty) {
                return Some(AGENT.into());
            }
        }
        None
    }

    /// The process and user at the other end of a Unix socket.
    fn peer(fd: i32) -> Option<(u32, u32)> {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for writing, and sized for `SO_PEERCRED`.
        let got = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (got == 0 && cred.pid > 0).then_some((cred.pid as u32, cred.uid))
    }

    /// Whether an environment (`/proc/<pid>/environ`) carries a launched session's markers.
    pub(super) fn launched(environ: &[u8]) -> bool {
        environ.split(|b| *b == 0).any(|entry| {
            [RUN_ENV, WORKSPACE_ENV, RECORD_ENV].iter().any(|name| {
                entry
                    .strip_prefix(name.as_bytes())
                    .and_then(|rest| rest.strip_prefix(b"="))
                    .is_some_and(|value| !value.is_empty())
            })
        })
    }

    /// The terminal a process's standard input is, if it is one.
    fn terminal_of(pid: u32) -> Option<PathBuf> {
        let target = std::fs::read_link(format!("/proc/{pid}/fd/0")).ok()?;
        is_terminal(&target).then_some(target)
    }

    pub(super) fn is_terminal(path: &std::path::Path) -> bool {
        let path = path.to_string_lossy();
        path.starts_with("/dev/pts/") || path.starts_with("/dev/tty")
    }

    /// Every process `pid` descends from.
    fn ancestors(pid: u32) -> BTreeSet<u32> {
        let mut seen = BTreeSet::new();
        let mut at = pid;
        while let Some(parent) = parent_of(at) {
            if parent <= 1 || !seen.insert(parent) {
                break;
            }
            at = parent;
        }
        seen
    }

    fn parent_of(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        parent_in_stat(&stat)
    }

    /// The parent's id in a `/proc/<pid>/stat` line, whose second field, the command's name in
    /// parentheses, may itself hold spaces and parentheses.
    pub(super) fn parent_in_stat(stat: &str) -> Option<u32> {
        let after = &stat[stat.rfind(')')? + 1..];
        after.split_whitespace().nth(1)?.parse().ok()
    }

    /// The terminals the daemon's own children run on: its sessions. A process that left the
    /// daemon's tree (a double fork) still has its session's terminal.
    fn daemons_terminals(daemon: u32) -> BTreeSet<PathBuf> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return BTreeSet::new();
        };
        entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| parent_of(*pid) == Some(daemon))
            .filter_map(terminal_of)
            .collect()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::linux::*;

    #[test]
    fn a_launched_sessions_markers_are_seen_and_an_empty_one_is_not() {
        assert!(launched(
            b"HOME=/home/a\0YARDSORT_SESSION_RECORD_ID=abc\0TERM=xterm\0"
        ));
        assert!(launched(b"YARDSORT_RUN_ID=1\0"));
        assert!(!launched(b"HOME=/home/a\0YARDSORT_RUN_ID=\0"));
        assert!(!launched(b"NOT_YARDSORT_RUN_ID=1\0YARDSORT_RUN_IDS=2\0"));
    }

    #[test]
    fn the_parent_is_read_past_a_name_with_parentheses_in_it() {
        assert_eq!(
            parent_in_stat("1234 (a (weird) name) S 987 1234 1234 0 -1"),
            Some(987)
        );
        assert_eq!(parent_in_stat("garbage"), None);
    }

    #[test]
    fn only_terminals_count_as_typed_at_one() {
        assert!(is_terminal(std::path::Path::new("/dev/pts/3")));
        assert!(is_terminal(std::path::Path::new("/dev/tty1")));
        assert!(!is_terminal(std::path::Path::new("/dev/null")));
        assert!(!is_terminal(std::path::Path::new("pipe:[1234]")));
    }
}

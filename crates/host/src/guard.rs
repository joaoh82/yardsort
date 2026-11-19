//! Who may approve a device here, or do anything else only a person may: a person at a terminal on
//! this computer, not an agent.
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
//!
//! **A command the dashboard ran is not a person either, nor is a terminal it opened.** The
//! dashboard runs commands and shells for a signed-in browser, as children of this host and
//! outside the daemon; its terminal is a real one, and a command can make itself one (`script`).
//! So the caller must also not carry the mark every dashboard job and terminal is given, and must
//! not descend from this host. That is of the strength the rest of this has: a job that sheds
//! both its mark and its ancestry gets past, as an agent that sheds its own does.
//!
//! The dashboard's page lets a device in by itself, on its Remote tab, and signs this computer
//! in to a server: the owner chose that, so that a server can be set up from a browser alone.
//! That is the page asking, with its session; no program is taken for it. So what this guard
//! refuses a dashboard job is not kept from whoever holds a dashboard session, only from a
//! command they did not mean to pair with.

use std::os::fd::AsRawFd;

use tokio::net::UnixStream;

/// Why the process at the other end of a connection may not do what only a person may. Linux makes
/// every one but `Unchecked`; elsewhere only `Unchecked` is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Refusal {
    /// Not Linux, where none of it could be checked.
    Unchecked,
    /// The system would not say who is at the other end.
    Unknown,
    /// Another user.
    OtherUser,
    /// A terminal Yardsort started, or a program running in one: an agent's.
    Launched,
    /// A command the dashboard ran for a signed-in browser, a terminal it opened for one, or a
    /// program either started.
    Job,
    /// Not a terminal at all: a script, or a pipe.
    NoTerminal,
}

impl Refusal {
    /// The refusal in words, for `act` ("Pairing", "Turning sync on") asked as `command`
    /// ("pair", "sync on").
    pub fn say(self, act: &str, command: &str) -> String {
        match self {
            Refusal::Unchecked => {
                format!("{act} from a terminal needs Linux, where the host can check who asks.")
            }
            Refusal::Unknown => "Cannot tell who is asking.".into(),
            Refusal::OtherUser => format!("Only this host's own user may do that ({act})."),
            Refusal::Launched => format!(
                "{act} is refused from a terminal Yardsort started. Only a person may do it, from a \
                 terminal of their own on this computer (an SSH login, say), never an agent."
            ),
            Refusal::Job => format!(
                "{act} is refused to a command or a terminal the dashboard ran: run \
                 `yardsort-host {command}` in a terminal of your own on this computer (an SSH \
                 login, say)."
            ),
            Refusal::NoTerminal => format!(
                "{act} needs a person at a terminal: run `yardsort-host {command}` in one, not from \
                 a script."
            ),
        }
    }
}

/// Why the process at the other end of `stream` may not do what only a person may, or `None` if
/// it may. `daemon` is the daemon's process id, when there is one.
pub fn refusal(stream: &UnixStream, daemon: Option<u32>) -> Option<Refusal> {
    #[cfg(target_os = "linux")]
    {
        linux::refusal(stream.as_raw_fd(), daemon)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (stream.as_raw_fd(), daemon);
        Some(Refusal::Unchecked)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::Refusal;
    use yardsort_core::activity::{RECORD_ENV, RUN_ENV, WORKSPACE_ENV};

    pub fn refusal(fd: i32, daemon: Option<u32>) -> Option<Refusal> {
        let Some((pid, uid)) = peer(fd) else {
            return Some(Refusal::Unknown);
        };
        // SAFETY: `geteuid` cannot fail.
        if uid != unsafe { libc::geteuid() } {
            return Some(Refusal::OtherUser);
        }
        let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        if launched(&environ) {
            return Some(Refusal::Launched);
        }
        // The dashboard's jobs are this host's own children, each given a mark.
        let above = ancestors(pid);
        if carries(&environ, &[host_dashboard::jobs::JOB_ENV])
            || above.contains(&std::process::id())
        {
            return Some(Refusal::Job);
        }
        let Some(tty) = terminal_of(pid) else {
            return Some(Refusal::NoTerminal);
        };
        if let Some(daemon) = daemon {
            if above.contains(&daemon) || daemons_terminals(daemon).contains(&tty) {
                return Some(Refusal::Launched);
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
        carries(environ, &[RUN_ENV, WORKSPACE_ENV, RECORD_ENV])
    }

    /// Whether an environment sets any of `names` to something.
    pub(super) fn carries(environ: &[u8], names: &[&str]) -> bool {
        environ.split(|b| *b == 0).any(|entry| {
            names.iter().any(|name| {
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
    fn a_dashboard_jobs_mark_is_seen_and_is_not_a_launched_sessions() {
        let job = [host_dashboard::jobs::JOB_ENV];
        assert!(carries(b"HOME=/home/a\0YARDSORT_DASHBOARD_JOB=1\0", &job));
        assert!(!carries(b"HOME=/home/a\0YARDSORT_DASHBOARD_JOB=\0", &job));
        assert!(!carries(b"NOT_YARDSORT_DASHBOARD_JOB=1\0", &job));
        assert!(!launched(b"YARDSORT_DASHBOARD_JOB=1\0"));
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

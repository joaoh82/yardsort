//! Starting, finding and talking to `yardsortd`.
//!
//! The daemon is **this same executable**, re-run with a flag — the trick `--yardsort-print-env`
//! already uses in [`crate::env`]. No second binary means nothing extra to bundle, sign or
//! notarize on three platforms, and the daemon can never be a different build from the app that
//! started it.
//!
//! One daemon per data directory, so `YARDSORT_DATA_DIR` isolates a throwaway profile's agents
//! the way it isolates its database.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pty_host::{EventSink, PtyHost, TerminalHost};
use pty_ipc::{DaemonClient, Endpoint, ServeOptions, IDLE_GRACE};
use serde::Serialize;
use specta::Type;

/// Makes this executable *be* the daemon. See [`run_daemon_and_exit_if_asked`].
const DAEMON_FLAG: &str = "--yardsort-daemon";

/// How long to wait for a daemon we have just spawned to start listening.
const START_TIMEOUT: Duration = Duration::from_secs(10);
const START_POLL: Duration = Duration::from_millis(25);

/// Run as the daemon and never return, when asked to. Called before anything Tauri does, so a
/// daemon never creates a window, a webview or a tray icon — it is a plain background process.
///
/// `yardsort --yardsort-daemon <socket> [<exit spool directory>]`. Its output is wherever
/// whoever started it pointed it: `daemon.log` when the app did, the terminal when you did.
/// Without a spool directory it keeps no exits for absent clients, as daemons before the spool
/// existed did not.
pub fn run_daemon_and_exit_if_asked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(DAEMON_FLAG)) {
        return;
    }
    let Some(socket) = args.next() else {
        eprintln!("{DAEMON_FLAG} needs a socket to listen on");
        std::process::exit(2);
    };
    let options = ServeOptions {
        spool_dir: args.next().map(std::path::PathBuf::from),
    };
    let result = pty_ipc::serve_with(
        &Endpoint::parse(&socket),
        env!("CARGO_PKG_VERSION"),
        IDLE_GRACE,
        options,
    );
    if let Err(error) = result {
        eprintln!("the terminal daemon stopped: {error}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// What the app ended up talking to, as the status bar and a bug report want it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    /// False when terminals are running inside the app instead — `YARDSORT_NO_DAEMON`, or a
    /// daemon that could not be reached. They then die with the app, as they did before.
    pub running: bool,
    pub version: Option<String>,
    pub pid: Option<u32>,
    /// Where the daemon listens, and where it writes when something goes wrong.
    pub endpoint: String,
    pub log_path: String,
    /// Why there is no daemon, when there should have been one.
    pub problem: Option<String>,
    /// Set when a daemon is running that this build cannot talk to — the app was updated
    /// underneath agents that are still working. It says how many, so they can be asked about
    /// rather than stopped behind the user's back.
    pub stranded_sessions: Option<u32>,
}

/// What the app ended up talking to.
pub struct Connected {
    pub host: Arc<dyn TerminalHost>,
    /// The same object as `host` when there is a daemon, for the things only a daemon can do.
    pub client: Option<Arc<DaemonClient>>,
    pub status: DaemonStatus,
}

/// Find the daemon for `data_dir`, starting one if there is none, and connect to it.
///
/// `YARDSORT_NO_DAEMON` keeps the host in this process instead — for debugging, and so the tests
/// do not each need a second process.
pub fn connect(data_dir: &Path, events: EventSink) -> Connected {
    let endpoint = Endpoint::for_data_dir(data_dir);
    let log_path = data_dir.join("daemon.log");
    // Where a daemon started here keeps exits for a client that is gone; see `activity`.
    let spool_dir = crate::activity::spool_dir(data_dir);
    let in_process = |problem: Option<String>| Connected {
        host: Arc::new(PtyHost::new(Arc::clone(&events))),
        client: None,
        status: DaemonStatus {
            running: false,
            version: None,
            pid: None,
            endpoint: endpoint.to_string(),
            log_path: log_path.display().to_string(),
            problem,
            stranded_sessions: None,
        },
    };

    if crate::legacy::env_var_os("NO_DAEMON").is_some() {
        eprintln!("YARDSORT_NO_DAEMON is set: terminals run in this process and die with it");
        return in_process(None);
    }

    let client = match connect_or_start(&endpoint, &log_path, &spool_dir, &events) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            // Falling back keeps the app usable: terminals work, they just will not outlive it.
            // The status bar says so, and `daemon.log` says why.
            eprintln!("could not reach a terminal daemon ({error}); running one in-process");
            return in_process(Some(error.to_string()));
        }
    };

    let info = client.daemon_info().clone();
    if !client.speaks_our_protocol() {
        // An update landed under a daemon started by the previous version.
        eprintln!(
            "the running daemon speaks protocol {} and this build speaks {}",
            info.protocol,
            pty_ipc::PROTOCOL
        );
        if info.running_sessions > 0 {
            // It has agents in it. Replacing it would kill work nobody agreed to lose, and
            // using it would fail in ways nothing here can predict — so leave it alone, say so,
            // and run terminals in-process until those agents are done and the app is restarted.
            let mut fallback = in_process(Some(format!(
                "{} agent(s) are still running under Yardsort {}, which this version cannot \
                 talk to. Finish or stop them, then restart Yardsort",
                info.running_sessions, info.version
            )));
            fallback.status.stranded_sessions = Some(info.running_sessions);
            return fallback;
        }
        // Nothing at stake: replace it.
        let _ = client.shutdown(false);
        drop(client);
        return match replace_daemon(&endpoint, &log_path, &spool_dir, &events) {
            Ok(client) => connected_to(client, endpoint, log_path),
            Err(error) => in_process(Some(error.to_string())),
        };
    }
    connected_to(client, endpoint, log_path)
}

/// Where this profile's daemon listens, as text. For diagnostics — `ys doctor` and bug reports.
pub fn endpoint_for(data_dir: &Path) -> String {
    Endpoint::for_data_dir(data_dir).to_string()
}

/// Attach to a daemon that is already running for `data_dir`, and never start one.
///
/// For clients that only want to report what is there — a listing that started a daemon would be
/// answering its own question, and would leave a process behind on a machine that had none.
pub fn connect_existing(data_dir: &Path, events: EventSink) -> Option<DaemonClient> {
    let endpoint = Endpoint::for_data_dir(data_dir);
    attach(&endpoint, &events).ok()
}

fn connected_to(
    client: Arc<DaemonClient>,
    endpoint: Endpoint,
    log_path: std::path::PathBuf,
) -> Connected {
    let info = client.daemon_info().clone();
    Connected {
        host: Arc::clone(&client) as Arc<dyn TerminalHost>,
        client: Some(client),
        status: DaemonStatus {
            running: true,
            version: Some(info.version),
            pid: Some(info.pid),
            endpoint: endpoint.to_string(),
            log_path: log_path.display().to_string(),
            problem: None,
            stranded_sessions: None,
        },
    }
}

/// Start a daemon of *this* version where an older one has just stood down, and connect to it.
fn replace_daemon(
    endpoint: &Endpoint,
    log_path: &Path,
    spool_dir: &Path,
    events: &EventSink,
) -> std::io::Result<Arc<DaemonClient>> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        // The old daemon has to let go of the socket before a new one can take it.
        if attach(endpoint, events).is_err() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(
                "the previous daemon would not stand down",
            ));
        }
        std::thread::sleep(START_POLL);
    }
    let client = Arc::new(connect_or_start(endpoint, log_path, spool_dir, events)?);
    if client.speaks_our_protocol() {
        Ok(client)
    } else {
        Err(std::io::Error::other(
            "a daemon of the wrong version keeps taking the socket",
        ))
    }
}

/// Connect to a listening daemon, or start one and connect to that.
fn connect_or_start(
    endpoint: &Endpoint,
    log_path: &Path,
    spool_dir: &Path,
    events: &EventSink,
) -> std::io::Result<DaemonClient> {
    if let Ok(client) = attach(endpoint, events) {
        return Ok(client);
    }
    // Nobody answered. Two copies of Yardsort starting at once would both get here, and must not
    // both start a daemon: only whoever holds the lock spawns, and the other waits for theirs.
    let ours = Lock::take(&log_path.with_extension("lock"));
    if let Ok(client) = attach(endpoint, events) {
        return Ok(client);
    }
    if ours.is_some() {
        spawn_daemon(endpoint, log_path, spool_dir)?;
    }

    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match attach(endpoint, events) {
            Ok(client) => return Ok(client),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(START_POLL),
        }
    }
}

fn attach(endpoint: &Endpoint, events: &EventSink) -> std::io::Result<DaemonClient> {
    DaemonClient::connect(endpoint, "yardsort", Arc::clone(events))
}

/// Start this executable again, detached, as the daemon.
fn spawn_daemon(endpoint: &Endpoint, log_path: &Path, spool_dir: &Path) -> std::io::Result<()> {
    clear_stale(endpoint);
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg(DAEMON_FLAG)
        .arg(endpoint.as_os_str())
        .arg(spool_dir.as_os_str())
        .stdin(std::process::Stdio::null());
    // A process with no terminal has nowhere to complain to, so it is given a log. Handing the
    // child the file is simpler than having it reopen its own output, and works the same on
    // every platform. A daemon started by hand keeps the terminal it was started from.
    match log_file(log_path) {
        Some((out, err)) => {
            command.stdout(out).stderr(err);
        }
        None => {
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
    }
    detach(&mut command);
    command.spawn()?;
    Ok(())
}

/// A socket file left behind by a daemon that was killed cannot be connected to, and would stop
/// its replacement binding. Only ever called having already failed to connect, so it is a corpse.
fn clear_stale(endpoint: &Endpoint) {
    if let Endpoint::Path(path) = endpoint {
        let _ = std::fs::remove_file(path);
    }
}

/// Each daemon starts its log afresh: the interesting run is the current one, and nothing here
/// should grow without bound in somebody's data directory.
fn log_file(path: &Path) -> Option<(std::process::Stdio, std::process::Stdio)> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .ok()?;
    let second = file.try_clone().ok()?;
    Some((file.into(), second.into()))
}

/// Put the daemon outside this process's session and process group, so closing the terminal (or
/// the app) that started it does not take it down with a signal.
#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // Safety: `setsid` is async-signal-safe, which is the rule for anything between fork and
    // exec. It only fails when we are already a group leader, which a child never is.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

/// No console window, its own process group, and — where the job object allows it — out of any
/// job that would otherwise kill it when the app it was launched from exits.
#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
}

/// A lock file held for as long as the guard lives, so that two copies of Yardsort starting at
/// the same moment do not start two daemons for the same profile.
///
/// [`take`](Lock::take) returns `None` when somebody else holds it. On Unix that never happens —
/// `flock` waits, and by the time it returns the other app has finished and there is a daemon to
/// connect to. On Windows the file is opened without sharing, so the second app is turned away
/// and waits for the first one's daemon instead.
struct Lock(
    /// The open file *is* the lock; the OS releases it when this closes, including if the
    /// process dies holding it.
    #[allow(dead_code)]
    std::fs::File,
);

impl Lock {
    fn take(path: &Path) -> Option<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(false);
        exclusive(&mut options);
        let file = options.open(path).ok()?;
        wait_for_it(&file);
        Some(Self(file))
    }
}

#[cfg(unix)]
fn exclusive(_options: &mut OpenOptions) {}

/// Opening with no sharing at all: a second app cannot open the file while this one holds it.
#[cfg(windows)]
fn exclusive(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.share_mode(0);
}

#[cfg(unix)]
fn wait_for_it(file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    // Safety: an ordinary advisory lock on a descriptor we own. It blocks until the lock is
    // ours, and is released when the file closes — including if this process dies holding it.
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_EX);
    }
}

#[cfg(not(unix))]
fn wait_for_it(_file: &std::fs::File) {
    // Opening the file exclusively was the lock.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_flag_is_the_first_argument_or_nothing_happens() {
        // Nothing to assert beyond "it returns": the real test is that the app still starts, and
        // every other test in this crate would fail if this exited the process.
        run_daemon_and_exit_if_asked();
    }

    /// A killed daemon leaves a socket file nobody can connect to; starting a new one has to
    /// clear it first, or the replacement cannot bind.
    #[cfg(unix)]
    #[test]
    fn a_stale_socket_is_cleared_out_of_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("daemon.sock");
        std::fs::write(&socket, b"corpse").unwrap();

        clear_stale(&Endpoint::Path(socket.clone()));
        assert!(!socket.exists());

        // Nothing to clear is not a failure, and a named pipe has no file to remove.
        clear_stale(&Endpoint::Path(socket));
        clear_stale(&Endpoint::Namespaced("yardsort-test.sock".into()));
    }

    /// Two copies of Yardsort starting at once must not start two daemons for one profile. The
    /// lock is what serialises them; the loser then finds the winner's daemon on its retry.
    #[test]
    fn only_one_app_at_a_time_gets_to_start_a_daemon() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.lock");
        let inside = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let racers: Vec<_> = (0..8)
            .map(|_| {
                let (path, inside, peak) = (path.clone(), Arc::clone(&inside), Arc::clone(&peak));
                std::thread::spawn(move || {
                    let Some(_held) = Lock::take(&path) else {
                        return;
                    };
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for racer in racers {
            racer.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1, "two apps spawned at once");
    }

    /// The log is replaced, not appended to, so a data directory cannot fill up with old runs.
    #[test]
    fn each_daemon_starts_its_log_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("daemon.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "from a previous run").unwrap();

        let handles = log_file(&path).expect("a log in a writable directory");
        drop(handles);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }
}

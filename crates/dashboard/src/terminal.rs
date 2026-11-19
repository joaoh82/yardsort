//! Terminals: a shell on this computer, typed into from the page.
//!
//! What a job cannot do: answer a question. An agent's own sign-in, `sudo` with a password and
//! `rustunnel setup` each ask one. A terminal is this user's shell in a pseudo-terminal, started
//! in the home folder with the login shell's environment, as an SSH login would be.
//!
//! It is the dashboard's, like a job: it carries the same mark ([`crate::jobs::JOB_ENV`]) and is
//! this program's own child, so what only a person at a terminal of their own may ask of
//! `yardsort-host` is refused in it too. Its output is kept as bytes, the newest of it, and only
//! ever handed to the page: nothing is read out of it, and none of it is recorded.
//!
//! Terminals live in memory and end with the program that started them, and when every browser
//! is signed out: a shell nobody can reach is not left running.
//!
//! Closing one hangs up on it, as closing a terminal's window does, and a moment later ends
//! what is still there of the shell and of the program it had in the foreground: one that
//! shrugs a hang-up off does not outlive its terminal. What was put in the background to
//! outlive it (`nohup`, `setsid`, a terminal multiplexer) does, as it outlives an SSH login.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use yardsort_core::env::ShellEnv;

/// How much of a terminal's output is kept: the newest of it.
const OUTPUT_KEPT: usize = 1024 * 1024;
/// Terminals open at once.
pub const OPEN_AT_ONCE: usize = 4;
/// Terminals remembered, open or ended.
const KEPT: usize = 8;
/// The most typed in one request.
pub const LONGEST_INPUT: usize = 64 * 1024;
/// How long what runs in a closed terminal has to go by itself, hung up on, before it is ended.
const GRACE: Duration = Duration::from_millis(1500);

/// A terminal's size in characters, kept within what a screen could be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.clamp(2, 500),
            rows: rows.clamp(1, 300),
        }
    }

    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// A terminal as the page lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: u64,
    /// The shell's name: `bash`, `zsh`.
    pub title: String,
    pub started: i64,
    pub ended: Option<i64>,
}

/// A terminal's output from some point on.
#[derive(Debug, Clone)]
pub struct Reading {
    /// Where `data` starts, counted in bytes since the terminal opened. Later than what was
    /// asked for when the start of a long output is no longer kept.
    pub from: u64,
    pub next: u64,
    pub data: Vec<u8>,
    pub ended: bool,
}

struct Terminal {
    id: u64,
    /// The shell's process, which leads a session and a process group of its own. Only a Unix
    /// has a group to end.
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    title: String,
    started: i64,
    inner: Mutex<Inner>,
    /// Woken when there is more output, and when it ends.
    changed: tokio::sync::Notify,
    /// The pseudo-terminal, until the terminal is closed or its shell is gone.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Where what is typed goes. Apart from `master`: a program that is not reading holds this
    /// for as long as it likes, and closing must not wait for it.
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    leaving: Mutex<Leaving>,
    seen: Option<Seen>,
}

/// The end of a terminal's shell. Held while a signal is sent to it or to what it ran, and
/// while it is waited for: after that its id is not its own, and nothing is sent to it.
#[derive(Default)]
struct Leaving {
    /// Closed from the page: when what is still running is ended, and the process group that
    /// was in the foreground then.
    #[cfg_attr(not(unix), allow(dead_code))]
    closing: Option<(Instant, Option<i32>)>,
    /// The shell has been waited for.
    waited_for: bool,
}

struct Inner {
    output: VecDeque<u8>,
    written: u64,
    ended: Option<i64>,
    /// Closed from the page: over for whoever reads it, before the shell is seen to be gone.
    closed: bool,
}

/// Told of a terminal when it opens and when it ends.
type Seen = Arc<dyn Fn(&Summary) + Send + Sync>;

impl Terminal {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn master(&self) -> MutexGuard<'_, Option<Box<dyn MasterPty + Send>>> {
        self.master.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn leaving(&self) -> MutexGuard<'_, Leaving> {
        self.leaving.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn summary(&self) -> Summary {
        Summary {
            id: self.id,
            title: self.title.clone(),
            started: self.started,
            ended: self.lock().ended,
        }
    }

    fn write(&self, bytes: &[u8]) {
        {
            let mut inner = self.lock();
            inner.written += bytes.len() as u64;
            inner.output.extend(bytes);
            let over = inner.output.len().saturating_sub(OUTPUT_KEPT);
            inner.output.drain(..over);
        }
        self.changed.notify_waiters();
    }

    /// The shell is gone: say so once, and let go of the pseudo-terminal so its reader ends.
    fn end(&self) {
        let first = {
            let mut inner = self.lock();
            let first = inner.ended.is_none();
            if first {
                inner.ended = Some(yardsort_core::store::now_ms());
            }
            first
        };
        *self.master() = None;
        // Not waited for: see `kill`.
        if let Ok(mut writer) = self.writer.try_lock() {
            *writer = None;
        }
        self.changed.notify_waiters();
        if first {
            if let Some(seen) = &self.seen {
                seen(&self.summary());
            }
        }
    }

    /// End the shell and what runs in it: a hang-up now, and an end to what is left after
    /// [`GRACE`]. The waiter thread sees the shell go, and ends the terminal.
    fn kill(self: &Arc<Self>) {
        {
            let mut inner = self.lock();
            let again = inner.closed;
            inner.closed = true;
            if again || inner.ended.is_some() {
                return;
            }
        }
        {
            let mut master = self.master();
            let mut leaving = self.leaving();
            if !leaving.waited_for {
                // What the shell has in the foreground is in a process group of its own, which
                // only the pseudo-terminal can name, and only while it is still open.
                #[cfg(unix)]
                let foreground = master.as_ref().and_then(|m| m.process_group_leader());
                #[cfg(not(unix))]
                let foreground = None;
                leaving.closing = Some((Instant::now() + GRACE, foreground));
                // A hang-up, as closing a terminal's window is. On Windows a kill that worked
                // is reported as an error (portable-pty 0.9), so the answer is not looked at.
                let _ = self
                    .killer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .kill();
            }
            // The pseudo-terminal goes too, which hangs up on whatever still has it open.
            *master = None;
        }
        // Not waited for: a write to a program that is not reading holds it until that program
        // ends.
        if let Ok(mut writer) = self.writer.try_lock() {
            *writer = None;
        }
        // A shell that shrugs a hang-up off is not left running behind a closed terminal. One
        // that went, leaving something behind, is seen to by the waiter thread.
        #[cfg(unix)]
        {
            let terminal = self.clone();
            let _ = std::thread::Builder::new()
                .name(format!("dashboard-terminal-{}-ender", self.id))
                .spawn(move || {
                    std::thread::sleep(GRACE);
                    let leaving = terminal.leaving();
                    if !leaving.waited_for {
                        terminal.end_what_is_left(&leaving);
                    }
                });
        }
        self.changed.notify_waiters();
    }

    /// End what a closed terminal still has running: the shell's process group, and the one it
    /// had in the foreground. With `leaving` held and the shell not yet waited for, so that the
    /// shell's id, which is its session's and its group's, is still its own.
    #[cfg(unix)]
    fn end_what_is_left(&self, leaving: &Leaving) {
        let (Some(shell), Some((_, foreground))) = (self.pid, leaving.closing) else {
            return;
        };
        let shell = shell as libc::pid_t;
        // SAFETY: signals to process groups, and a question about a process; nothing is read
        // or written.
        unsafe {
            libc::killpg(shell, libc::SIGKILL);
            let Some(group) = foreground.filter(|group| *group != shell && *group > 1) else {
                return;
            };
            // The group's id is its own for as long as anything is in it. Should all of it be
            // gone, the id may be another program's by now: one that is not in this shell's
            // session, which is how it is told apart. With no such process at all, what is left
            // of the group is what its first program left behind: `a | b` with `a` gone.
            let session = libc::getsid(group);
            let gone = session == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if session == shell || gone {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    /// Wait for the shell, which has just been started as `child`. On a Unix it is first seen
    /// to be gone without being waited for, which keeps its id its own while a closed
    /// terminal's leftovers are ended.
    fn see_out(&self, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            loop {
                // SAFETY: `info` is only written to, and is big enough; `WNOWAIT` leaves the
                // child to be waited for below.
                let seen = unsafe {
                    let mut info: libc::siginfo_t = std::mem::zeroed();
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOWAIT,
                    )
                };
                let interrupted = seen == -1
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted;
                if !interrupted {
                    break;
                }
            }
            loop {
                let mut leaving = self.leaving();
                if let Some((at, _)) = leaving.closing {
                    let left = at.saturating_duration_since(Instant::now());
                    if !left.is_zero() {
                        drop(leaving);
                        std::thread::sleep(left);
                        continue;
                    }
                    self.end_what_is_left(&leaving);
                }
                let _ = child.wait();
                leaving.waited_for = true;
                return;
            }
        }
        let _ = child.wait();
        self.leaving().waited_for = true;
    }

    fn read(&self, from: u64) -> Reading {
        let inner = self.lock();
        let kept_from = inner.written - inner.output.len() as u64;
        let from = from.clamp(kept_from, inner.written);
        let skip = (from - kept_from) as usize;
        Reading {
            from,
            next: inner.written,
            data: inner.output.iter().skip(skip).copied().collect(),
            ended: inner.ended.is_some() || inner.closed,
        }
    }
}

#[derive(Default)]
pub struct Terminals {
    list: Mutex<Vec<Arc<Terminal>>>,
    next: Mutex<u64>,
    seen: Option<Seen>,
}

impl Terminals {
    /// Terminals that tell `seen` of each one as it opens and as it ends.
    pub fn watched(seen: impl Fn(&Summary) + Send + Sync + 'static) -> Self {
        Self {
            seen: Some(Arc::new(seen)),
            ..Self::default()
        }
    }

    /// Open a terminal: this user's shell, in their home. Refused while too many are open.
    /// Blocking: it starts a program.
    pub fn open(&self, env: &ShellEnv, size: Size) -> Result<Summary, String> {
        let mut list = self.lock();
        let open = list.iter().filter(|t| t.lock().ended.is_none()).count();
        if open >= OPEN_AT_ONCE {
            return Err(format!(
                "{OPEN_AT_ONCE} terminals are open already. Close one first."
            ));
        }
        let (program, args) = shell(env);
        let pair = native_pty_system()
            .openpty(size.pty())
            .map_err(|e| format!("A terminal could not be opened: {e}"))?;
        let mut command = CommandBuilder::new(&program);
        command.args(args);
        command.env_clear();
        for (name, value) in &env.vars {
            command.env(name, value);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // The dashboard's, as a job is: see the module's words, and `yardsort-host`'s guard.
        command.env(crate::jobs::JOB_ENV, "1");
        command.cwd(env.home_dir().unwrap_or_else(std::env::temp_dir));
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("{program} could not be started: {e}"))?;
        // No handle to the other side is kept, or the reader would never see its end.
        drop(pair.slave);
        // `portable-pty` reports its failures as `anyhow::Error`; only their words are used.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("A terminal could not be opened: {e}"));
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("A terminal could not be opened: {e}"));
        let (mut reader, writer) = match (reader, writer) {
            (Ok(reader), Ok(writer)) => (reader, writer),
            (Err(why), _) | (_, Err(why)) => {
                let _ = child.kill();
                return Err(why);
            }
        };
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        let terminal = Arc::new(Terminal {
            id,
            pid: child.process_id(),
            title: std::path::Path::new(&program).file_stem().map_or_else(
                || program.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
            started: yardsort_core::store::now_ms(),
            inner: Mutex::new(Inner {
                output: VecDeque::new(),
                written: 0,
                ended: None,
                closed: false,
            }),
            changed: tokio::sync::Notify::new(),
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(child.clone_killer()),
            leaving: Mutex::default(),
            seen: self.seen.clone(),
        });
        let reading = terminal.clone();
        let read = std::thread::Builder::new()
            .name(format!("dashboard-terminal-{id}-reader"))
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => reading.write(&buffer[..n]),
                    }
                }
            });
        let waiting = terminal.clone();
        let wait = std::thread::Builder::new()
            .name(format!("dashboard-terminal-{id}-waiter"))
            .spawn(move || {
                waiting.see_out(child);
                // What it printed last is still on its way through the reader.
                std::thread::sleep(Duration::from_millis(50));
                waiting.end();
            });
        if read.is_err() || wait.is_err() {
            terminal.kill();
            terminal.end();
            return Err("A terminal could not be opened: no thread for it.".into());
        }
        list.push(terminal.clone());
        // The oldest ended ones go; one still open is never forgotten.
        while list.len() > KEPT {
            let Some(at) = list.iter().position(|t| t.lock().ended.is_some()) else {
                break;
            };
            list.remove(at);
        }
        drop(list);
        let summary = terminal.summary();
        if let Some(seen) = &self.seen {
            seen(&summary);
        }
        Ok(summary)
    }

    /// Every terminal, the oldest first.
    pub fn list(&self) -> Vec<Summary> {
        self.lock().iter().map(|t| t.summary()).collect()
    }

    /// A terminal's output from byte `from` on, waiting up to `wait` for there to be any.
    /// `None` for a terminal there is not.
    pub async fn read(&self, id: u64, from: u64, wait: Duration) -> Option<Reading> {
        let terminal = self.find(id)?;
        let until = tokio::time::Instant::now() + wait;
        loop {
            // Listening before looking, so that output written in between is not slept through.
            let changed = terminal.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let reading = terminal.read(from);
            if !reading.data.is_empty() || reading.ended || reading.from != from {
                return Some(reading);
            }
            if tokio::time::timeout_at(until, changed).await.is_err() {
                return Some(terminal.read(from));
            }
        }
    }

    /// Type into a terminal. Blocking: a program that is not reading makes it wait.
    pub fn type_into(&self, id: u64, typed: &[u8]) -> Result<(), String> {
        let terminal = self.find(id).ok_or("No such terminal.")?;
        let mut writer = terminal
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let writer = writer.as_mut().ok_or("That terminal has ended.")?;
        writer
            .write_all(typed)
            .and_then(|()| writer.flush())
            .map_err(|e| format!("That terminal took nothing: {e}"))
    }

    pub fn resize(&self, id: u64, size: Size) -> Result<(), String> {
        let terminal = self.find(id).ok_or("No such terminal.")?;
        let master = terminal.master();
        let master = master.as_ref().ok_or("That terminal has ended.")?;
        master
            .resize(size.pty())
            .map_err(|e| format!("That terminal could not be resized: {e}"))
    }

    /// End a terminal's shell, and forget the terminal. Whether there was one.
    pub fn close(&self, id: u64) -> bool {
        let Some(terminal) = self.find(id) else {
            return false;
        };
        // Whoever is waiting on it is told now, not when the shell is seen to be gone.
        terminal.kill();
        self.lock().retain(|t| t.id != id);
        true
    }

    /// End every terminal.
    pub fn close_all(&self) {
        let all: Vec<u64> = self.lock().iter().map(|t| t.id).collect();
        for id in all {
            self.close(id);
        }
    }

    fn find(&self, id: u64) -> Option<Arc<Terminal>> {
        self.lock().iter().find(|t| t.id == id).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<Terminal>>> {
        self.list.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The shell to start, and what to start it with: the user's own, as a login shell.
fn shell(env: &ShellEnv) -> (String, Vec<&'static str>) {
    if cfg!(windows) {
        let shell = env
            .get("COMSPEC")
            .filter(|s| !s.is_empty())
            .unwrap_or("cmd.exe");
        return (shell.to_owned(), Vec::new());
    }
    let shell = env
        .get("SHELL")
        .filter(|s| std::path::Path::new(s).is_absolute())
        .unwrap_or("/bin/sh");
    (shell.to_owned(), vec!["-l"])
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use yardsort_core::env::EnvSource;

    fn env(home: &std::path::Path) -> ShellEnv {
        ShellEnv {
            vars: BTreeMap::from([
                ("HOME".to_owned(), home.display().to_string()),
                ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
                ("SHELL".to_owned(), "/bin/sh".to_owned()),
                // A prompt that can be waited for, whatever the machine's shell is.
                ("PS1".to_owned(), "ready> ".to_owned()),
                ("ENV".to_owned(), String::new()),
            ]),
            source: EnvSource::Process,
            warning: None,
        }
    }

    /// Everything a terminal has printed once `wanted` is in it.
    fn until(terminals: &Terminals, id: u64, wanted: &str) -> String {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let since = Instant::now();
        loop {
            let reading = runtime
                .block_on(terminals.read(id, 0, Duration::from_millis(200)))
                .expect("the terminal");
            let text = String::from_utf8_lossy(&reading.data).into_owned();
            if text.contains(wanted) {
                return text;
            }
            assert!(
                since.elapsed() < Duration::from_secs(20) && !reading.ended,
                "never printed {wanted:?}: {text:?}"
            );
        }
    }

    #[test]
    fn a_terminal_is_a_shell_in_the_home_that_answers_what_is_typed() {
        let home = tempfile::tempdir().unwrap();
        let terminals = Terminals::default();
        let opened = terminals
            .open(&env(home.path()), Size::new(80, 24))
            .unwrap();
        assert_eq!(opened.title, "sh");
        until(&terminals, opened.id, "ready> ");
        // It is a terminal to what runs in it, in the home, and marked as the dashboard's.
        terminals
            .type_into(
                opened.id,
                b"test -t 0 && echo is-a-tty; pwd; echo mark=$YARDSORT_DASHBOARD_JOB; stty size\n",
            )
            .unwrap();
        let printed = until(&terminals, opened.id, "24 80");
        assert!(printed.contains("is-a-tty"), "{printed:?}");
        assert!(printed.contains("mark=1"), "{printed:?}");
        let real = std::fs::canonicalize(home.path()).unwrap();
        assert!(
            printed.contains(&real.display().to_string())
                || printed.contains(&home.path().display().to_string()),
            "{printed:?}"
        );
        // A question is answered: what a job cannot do.
        terminals
            .type_into(opened.id, b"read answer; echo \"got <$answer>\"\n")
            .unwrap();
        terminals.type_into(opened.id, b"forty-two\n").unwrap();
        until(&terminals, opened.id, "got <forty-two>");

        terminals.resize(opened.id, Size::new(132, 40)).unwrap();
        terminals.type_into(opened.id, b"stty size\n").unwrap();
        until(&terminals, opened.id, "40 132");
    }

    #[test]
    fn a_reader_waits_for_output_and_is_told_when_the_shell_is_gone() {
        let home = tempfile::tempdir().unwrap();
        let terminals = Arc::new(Terminals::default());
        let opened = terminals
            .open(&env(home.path()), Size::new(80, 24))
            .unwrap();
        let at = until(&terminals, opened.id, "ready> ").len() as u64;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        // Nothing new: the reader waits its whole wait, and comes back empty.
        let since = Instant::now();
        let quiet = runtime
            .block_on(terminals.read(opened.id, at, Duration::from_millis(300)))
            .unwrap();
        assert!(quiet.data.is_empty() && !quiet.ended);
        assert!(since.elapsed() >= Duration::from_millis(250));
        // Typed while a reader waits: it is back long before its wait is over.
        let waiting = {
            let terminals = terminals.clone();
            runtime
                .spawn(async move { terminals.read(opened.id, at, Duration::from_secs(20)).await })
        };
        std::thread::sleep(Duration::from_millis(100));
        let since = Instant::now();
        terminals.type_into(opened.id, b"echo woke\n").unwrap();
        let woke = runtime.block_on(waiting).unwrap().unwrap();
        assert!(!woke.data.is_empty());
        assert!(since.elapsed() < Duration::from_secs(5));

        terminals.type_into(opened.id, b"exit\n").unwrap();
        let since = Instant::now();
        loop {
            let reading = runtime
                .block_on(terminals.read(opened.id, u64::MAX, Duration::from_millis(200)))
                .unwrap();
            if reading.ended {
                break;
            }
            assert!(since.elapsed() < Duration::from_secs(20), "it never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(terminals.list()[0].ended.is_some());
        assert_eq!(
            terminals.type_into(opened.id, b"x").unwrap_err(),
            "That terminal has ended."
        );
    }

    /// Wait for the loop that keeps touching `marker` to stop.
    fn stops_touching(marker: &std::path::Path) {
        let since = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(300));
            let _ = std::fs::remove_file(marker);
            std::thread::sleep(Duration::from_millis(300));
            if !marker.exists() {
                break;
            }
            assert!(since.elapsed() < Duration::from_secs(20), "still running");
        }
    }

    #[test]
    fn closing_ends_a_program_that_shrugs_a_hang_up_off_after_its_shell_is_gone() {
        let home = tempfile::tempdir().unwrap();
        let terminals = Terminals::default();
        let opened = terminals
            .open(&env(home.path()), Size::new(80, 24))
            .unwrap();
        until(&terminals, opened.id, "ready> ");
        // In the foreground, so in a process group of its own, and deaf to the hang-up that
        // ends the shell: `sleep` inherits that.
        let marker = home.path().join("stubborn");
        terminals
            .type_into(
                opened.id,
                format!(
                    "sh -c 'trap \"\" HUP; while :; do touch {}; sleep 0.05; done'\n",
                    marker.display()
                )
                .as_bytes(),
            )
            .unwrap();
        let since = Instant::now();
        while !marker.exists() {
            assert!(since.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(terminals.close(opened.id));
        stops_touching(&marker);
    }

    #[test]
    fn closing_ends_the_shell_and_what_it_ran_and_only_four_are_open_at_once() {
        let home = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let terminals = Terminals::watched({
            let seen = seen.clone();
            move |summary| seen.lock().unwrap().push(summary.ended.is_some())
        });
        let env = env(home.path());
        let first = terminals.open(&env, Size::new(80, 24)).unwrap();
        until(&terminals, first.id, "ready> ");
        let marker = home.path().join("alive");
        terminals
            .type_into(
                first.id,
                format!(
                    "sh -c 'while :; do touch {}; sleep 0.05; done'\n",
                    marker.display()
                )
                .as_bytes(),
            )
            .unwrap();
        let since = Instant::now();
        while !marker.exists() {
            assert!(since.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
        for _ in 1..OPEN_AT_ONCE {
            terminals.open(&env, Size::new(80, 24)).unwrap();
        }
        let refused = terminals.open(&env, Size::new(80, 24)).unwrap_err();
        assert!(
            refused.contains("4 terminals are open already"),
            "{refused}"
        );

        assert!(terminals.close(first.id));
        assert!(!terminals.close(first.id), "it is forgotten");
        assert_eq!(terminals.list().len(), OPEN_AT_ONCE - 1);
        // The loop it was running stops touching the file.
        stops_touching(&marker);
        terminals.open(&env, Size::new(80, 24)).unwrap();
        terminals.close_all();
        assert!(terminals.list().is_empty());
        // Each was told of once as it opened, and the first once as it ended.
        let since = Instant::now();
        loop {
            let seen = seen.lock().unwrap().clone();
            let (opened, ended) = (
                seen.iter().filter(|ended| !**ended).count(),
                seen.iter().filter(|ended| **ended).count(),
            );
            if opened == OPEN_AT_ONCE + 1 && ended == OPEN_AT_ONCE + 1 {
                break;
            }
            assert!(since.elapsed() < Duration::from_secs(20), "{seen:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_size_is_kept_within_what_a_screen_could_be() {
        assert_eq!(Size::new(0, 0), Size { cols: 2, rows: 1 });
        assert_eq!(
            Size::new(9999, 9999),
            Size {
                cols: 500,
                rows: 300
            }
        );
        assert_eq!(
            Size::new(120, 40),
            Size {
                cols: 120,
                rows: 40
            }
        );
    }
}

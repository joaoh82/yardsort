//! A child process and everything it starts, ended together.
//!
//! A dev server started by a smoke test, or a project's setup command, starts children of its
//! own — `npm run dev` is a shell that starts node that starts esbuild. Killing the one process
//! Yardsort started leaves the rest running, holding the port. So a [`Group`] puts the child in
//! a process group of its own on Unix, and in a job object on Windows, and [`Group::kill_tree`]
//! ends the lot. The job object is also set to end its processes when the handle closes, which
//! it does when the app exits however it exits; Unix has no such thing, so an orphan from a run
//! the app stopped during is found again by its pid and start time ([`kill_orphan`]).

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

/// How long a process gets to end on its own after being asked, before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// A child process in a group of its own.
#[derive(Debug)]
pub struct Group {
    child: Child,
    #[cfg(windows)]
    job: windows::Job,
}

impl Group {
    /// Start `command` in a group of its own. The command is spawned as it was built, with
    /// what this adds: a new process group (Unix), or a new job object that kills its members
    /// when the handle closes (Windows), and no console window.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Safety: `setpgid` is async-signal-safe, and this runs in the child after the fork
            // and before the exec, which is what `pre_exec` is for.
            unsafe {
                command.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
            let child = command.spawn()?;
            Ok(Self { child })
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
            let job = windows::Job::new()?;
            let child = command.spawn()?;
            // Assigned right after it starts, not before: the standard library gives no handle
            // to a suspended main thread. A process the child starts in the microseconds
            // before this is outside the job; everything it starts afterwards is in it.
            job.assign(&child)?;
            Ok(Self { child, job })
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// The child's standard input, to write to and then drop, when it was piped.
    pub fn stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.child.stdin.take()
    }

    /// End the process and everything it started. Asks first, and kills what has not ended
    /// after a moment. Reaps the child, so no zombie stays behind.
    pub fn kill_tree(&mut self) {
        #[cfg(unix)]
        {
            let pgid = self.child.id() as libc::pid_t;
            // Safety: plain signals to a process group this process created.
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
            }
            let asked = Instant::now();
            while asked.elapsed() < GRACE {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
            let _ = self.child.kill();
        }
        #[cfg(windows)]
        {
            self.job.terminate();
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    /// Something about the process that a later process with the same pid will not share:
    /// when it started. For finding an orphan again after the app that started it stopped.
    /// `None` where the platform does not need it, or cannot say.
    pub fn fingerprint(&self) -> Option<String> {
        fingerprint(self.child.id())
    }
}

/// When the process `pid` started, as a token to compare, if it is running and the platform
/// says. On Windows the job object has already ended an orphan, so there is nothing to find.
pub fn fingerprint(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        // `/proc/<pid>/stat`: the command name in parentheses may hold spaces, so the fields
        // are counted from after it. Field 22, start time in clock ticks since boot, is the
        // nineteenth after the parenthesis.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = stat.rsplit_once(')')?.1;
        let mut fields = after.split_whitespace();
        // The first field after the name is the state: a zombie has ended, whoever has yet to
        // reap it, and is not a process to find again.
        if matches!(fields.next(), Some("Z" | "X")) {
            return None;
        }
        fields.nth(18).map(str::to_owned)
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-o", "lstart=", "-p", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        // The state comes first: a zombie has ended, and is not a process to find again.
        let (state, started) = text.split_once(' ')?;
        (!state.starts_with('Z') && !started.trim().is_empty()).then(|| started.trim().to_owned())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// End the process group of `pid` if the process is still the one that was started —
/// its start time is still `expected` — and whatever it started with it. For `start`
/// processes of a run the app stopped during. Does nothing on Windows: the job object ended
/// them when the app's handle closed.
///
/// The leader going is not the group going: `npm run dev` may exit on TERM while the node it
/// started ignores the signal. So the group is given the grace period as a whole, and what is
/// left of it is killed, whether the leader is still in it or not. The group's id cannot be
/// handed to a new process while any member remains, so the kill reaches no stranger.
pub fn kill_orphan(pid: u32, expected: &str) -> bool {
    if fingerprint(pid).as_deref() != Some(expected) {
        return false;
    }
    #[cfg(unix)]
    {
        let pgid = pid as libc::pid_t;
        // Safety: signals to a process group whose leader was just checked to be the process
        // that was started. The group id is the leader's pid, as `setpgid(0, 0)` made it.
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        let asked = Instant::now();
        while asked.elapsed() < GRACE {
            if group_is_empty(pgid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
        true
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Whether no process is left in the group `pgid`: signal 0 is delivered to nobody.
#[cfg(unix)]
fn group_is_empty(pgid: libc::pid_t) -> bool {
    // Safety: signal 0 checks, and sends nothing.
    let answer = unsafe { libc::kill(-pgid, 0) };
    answer == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A job object whose processes end when it is terminated, or when its last handle
    /// closes — which happens when the process holding it ends, however it ends.
    #[derive(Debug)]
    pub struct Job(HANDLE);

    // Safety: a job object handle is a kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> io::Result<Self> {
            // Safety: plain Win32 calls with valid arguments; every handle is checked.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if set == 0 {
                    let error = io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error);
                }
                Ok(Self(job))
            }
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            use std::os::windows::io::AsRawHandle;
            // Safety: the child's handle is open for as long as `child` lives.
            let assigned =
                unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if assigned == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // Safety: a handle this struct owns.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // Safety: closing a handle this struct owns, once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// A process that starts a child of its own, both sleeping: the shape of a dev server.
    fn sleeper() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30 & wait"]);
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args([
                "/C",
                "start /B ping -n 40 127.0.0.1 >NUL & ping -n 40 127.0.0.1 >NUL",
            ]);
            command
        }
    }

    #[test]
    fn the_whole_tree_ends_with_the_group() {
        let mut group = Group::spawn(sleeper().stdin(std::process::Stdio::null())).unwrap();
        let pid = group.id();
        assert!(matches!(group.try_wait(), Ok(None)), "still running");
        let print = group.fingerprint();
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(print.is_some(), "a start time is known while it runs");
        }
        let started = Instant::now();
        group.kill_tree();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "ended without waiting it out"
        );
        assert!(matches!(group.try_wait(), Ok(Some(_))), "reaped");
        #[cfg(unix)]
        {
            // The grandchild — `sleep` — went with the group: its parent group has nobody.
            // `kill` with signal 0 asks whether any process in the group remains.
            std::thread::sleep(Duration::from_millis(100));
            let remaining = unsafe { libc::kill(-(pid as libc::pid_t), 0) };
            assert_eq!(remaining, -1, "no process is left in group {pid}");
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
        }
        if let Some(print) = print {
            assert!(
                !kill_orphan(pid, &print),
                "a pid that is gone, or reused, is not the process that was started"
            );
        }
    }

    /// A leader that exits on TERM while its child ignores the signal: the shape of a package
    /// script around a server. The orphan kill must reach the child.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn an_orphans_survivors_go_when_its_leader_went_first() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        // The child is started with TERM ignored, which it keeps; the shell then takes TERM
        // again, so it exits on the first signal and the child does not.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "trap '' TERM; sleep 60 & echo $! > '{}'; trap - TERM; wait",
                pidfile.display()
            ),
        ]);
        command.stdin(std::process::Stdio::null());
        let mut group = Group::spawn(&mut command).unwrap();
        let pid = group.id();
        let print = group.fingerprint().expect("a start time while it runs");
        let asked = Instant::now();
        let child: u32 = loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(child) = text.trim().parse() {
                    break child;
                }
            }
            assert!(
                asked.elapsed() < Duration::from_secs(5),
                "the child's pid is written"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(fingerprint(child).is_some(), "the child runs");

        assert!(kill_orphan(pid, &print), "the group was the one started");
        // The test is the leader's parent, so it reaps the leader; init reaps the child.
        let _ = group.wait();
        let asked = Instant::now();
        while fingerprint(child).is_some() && asked.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            fingerprint(child).is_none(),
            "the child that ignored TERM was killed with the rest of the group"
        );
    }
}

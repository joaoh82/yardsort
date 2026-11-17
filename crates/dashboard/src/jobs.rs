//! Jobs: what the dashboard was asked to run, each with its output as it comes.
//!
//! A job is one or more programs run in turn, or a piece of this program's own work. A program is
//! started as everything here is: an argv array, in the environment the user's login shell
//! resolved, with nothing on its input, and in a process group of its own so that stopping it
//! stops what it started. Its output is kept as text, the newest part of it when it runs long,
//! and is only ever shown: nothing is read out of it.
//!
//! Jobs live in memory, and belong to the program that started them: when it stops, what still
//! runs is stopped with it, rather than left writing to nobody.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use yardsort_core::process::Group;

/// How much of a job's output is kept: the newest of it.
const OUTPUT_KEPT: usize = 512 * 1024;
/// Jobs remembered, running or not.
const JOBS_KEPT: usize = 30;
/// Jobs running at once.
const RUNNING_AT_ONCE: usize = 4;

/// One program to run.
pub struct Step {
    pub command: Command,
    /// Shown before it starts, as a shell would echo it. Empty for none.
    pub shown: String,
    /// Written to its input, which is then closed. For a secret that must be in no command line.
    pub input: Option<String>,
}

impl Step {
    pub fn new(command: Command) -> Self {
        Self {
            command,
            shown: String::new(),
            input: None,
        }
    }

    pub fn shown(mut self, line: impl Into<String>) -> Self {
        self.shown = line.into();
        self
    }

    pub fn input(mut self, text: impl Into<String>) -> Self {
        self.input = Some(text.into());
        self
    }
}

/// This program's own work within a job: it writes what it does, and says how it ended.
pub type Task = Box<dyn FnOnce(&Output) -> Result<(), String> + Send>;

enum Part {
    Run(Box<Step>),
    Do(Task),
}

/// What a job does: its parts in turn, the first to fail ending it.
#[derive(Default)]
pub struct Work {
    parts: Vec<Part>,
    /// Run when the job ends, however it ends: what it must not leave behind.
    after: Option<Box<dyn FnOnce() + Send>>,
}

impl Work {
    /// Programs in turn.
    pub fn steps(steps: Vec<Step>) -> Self {
        Self {
            parts: steps
                .into_iter()
                .map(|step| Part::Run(Box::new(step)))
                .collect(),
            after: None,
        }
    }

    /// This program's own work alone. It cannot be stopped part way.
    pub fn task(task: impl FnOnce(&Output) -> Result<(), String> + Send + 'static) -> Self {
        Self::default().then(task)
    }

    /// And then this, if everything before it succeeded.
    pub fn then(
        mut self,
        task: impl FnOnce(&Output) -> Result<(), String> + Send + 'static,
    ) -> Self {
        self.parts.push(Part::Do(Box::new(task)));
        self
    }

    /// And then these programs in turn, if everything before them succeeded.
    pub fn and(mut self, steps: Vec<Step>) -> Self {
        self.parts
            .extend(steps.into_iter().map(|step| Part::Run(Box::new(step))));
        self
    }

    /// And when it ends, however it ends, this.
    pub fn after(mut self, tidy: impl FnOnce() + Send + 'static) -> Self {
        self.after = Some(Box::new(tidy));
        self
    }

    fn runs_a_program(&self) -> bool {
        self.parts.iter().any(|part| matches!(part, Part::Run(_)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Running,
    Succeeded,
    Failed,
    Stopped,
}

/// A job as the page lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: u64,
    /// `command`, `agent`, `github` or `clone`.
    pub kind: String,
    pub title: String,
    pub state: State,
    pub started: i64,
    pub ended: Option<i64>,
    /// Whether **Stop** does anything.
    pub stoppable: bool,
}

/// A job's output from some point on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    #[serde(flatten)]
    pub summary: Summary,
    /// The output from `from` on.
    pub output: String,
    /// Where `output` starts, counted from the job's first byte. Later than what was asked for
    /// when the start of a long output is no longer kept.
    pub from: u64,
    /// Where to ask from next.
    pub next: u64,
}

/// Where a job's output goes.
pub struct Output(Arc<Job>);

impl Output {
    pub fn line(&self, text: &str) {
        self.0.write(text.as_bytes());
        self.0.write(b"\n");
    }
}

struct Job {
    id: u64,
    kind: String,
    title: String,
    started: i64,
    stoppable: bool,
    stop: AtomicBool,
    inner: Mutex<Inner>,
}

struct Inner {
    state: State,
    ended: Option<i64>,
    output: VecDeque<u8>,
    /// Bytes written since the job began, kept or not.
    written: u64,
    /// The program running now.
    group: Option<Arc<Mutex<Group>>>,
}

impl Job {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self, bytes: &[u8]) {
        let mut inner = self.lock();
        inner.written += bytes.len() as u64;
        inner.output.extend(bytes);
        let over = inner.output.len().saturating_sub(OUTPUT_KEPT);
        inner.output.drain(..over);
    }

    fn summary(&self) -> Summary {
        let inner = self.lock();
        Summary {
            id: self.id,
            kind: self.kind.clone(),
            title: self.title.clone(),
            state: inner.state,
            started: self.started,
            ended: inner.ended,
            stoppable: self.stoppable && inner.state == State::Running,
        }
    }

    fn finish(&self, state: State) {
        let mut inner = self.lock();
        inner.state = state;
        inner.ended = Some(yardsort_core::store::now_ms());
        inner.group = None;
    }
}

#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<Vec<Arc<Job>>>,
    next: Mutex<u64>,
}

impl Jobs {
    /// Start a job. Refused while too many run.
    pub fn start(&self, kind: &str, title: &str, work: Work) -> Result<Summary, String> {
        let job = {
            let mut jobs = self.lock();
            let running = jobs
                .iter()
                .filter(|job| job.lock().state == State::Running)
                .count();
            if running >= RUNNING_AT_ONCE {
                return Err(format!(
                    "{RUNNING_AT_ONCE} jobs are running already. Wait for one, or stop one."
                ));
            }
            let id = {
                let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
                *next += 1;
                *next
            };
            let job = Arc::new(Job {
                id,
                kind: kind.to_owned(),
                title: title.to_owned(),
                started: yardsort_core::store::now_ms(),
                stoppable: work.runs_a_program(),
                stop: AtomicBool::new(false),
                inner: Mutex::new(Inner {
                    state: State::Running,
                    ended: None,
                    output: VecDeque::new(),
                    written: 0,
                    group: None,
                }),
            });
            jobs.push(job.clone());
            // The oldest finished ones go; one still running is never forgotten.
            while jobs.len() > JOBS_KEPT {
                let Some(at) = jobs
                    .iter()
                    .position(|job| job.lock().state != State::Running)
                else {
                    break;
                };
                jobs.remove(at);
            }
            job
        };
        let summary = job.summary();
        let spawned = std::thread::Builder::new()
            .name(format!("dashboard-job-{}", job.id))
            .spawn({
                let job = job.clone();
                move || run(&job, work)
            });
        if let Err(error) = spawned {
            job.write(format!("Could not start: {error}\n").as_bytes());
            job.finish(State::Failed);
        }
        Ok(summary)
    }

    /// Every job, the newest first.
    pub fn list(&self) -> Vec<Summary> {
        self.lock().iter().rev().map(|job| job.summary()).collect()
    }

    /// A job's output from `from` on.
    pub fn read(&self, id: u64, from: u64) -> Option<Reading> {
        let job = self.find(id)?;
        let (output, start, next) = {
            let inner = job.lock();
            let kept_from = inner.written - inner.output.len() as u64;
            let start = from.clamp(kept_from, inner.written);
            let skip = (start - kept_from) as usize;
            let bytes: Vec<u8> = inner.output.iter().skip(skip).copied().collect();
            (bytes, start, inner.written)
        };
        Some(Reading {
            summary: job.summary(),
            // Split anywhere, a character may be cut in two at either end: it shows as the
            // replacement mark, which is what a terminal would do with half of one.
            output: String::from_utf8_lossy(&output).into_owned(),
            from: start,
            next,
        })
    }

    /// Stop a job and what it started. Says whether there was one running to stop.
    pub fn stop(&self, id: u64) -> bool {
        let Some(job) = self.find(id) else {
            return false;
        };
        if !job.stoppable || job.lock().state != State::Running {
            return false;
        }
        job.stop.store(true, Ordering::SeqCst);
        let group = job.lock().group.clone();
        if let Some(group) = group {
            // Off the caller's thread: asking, then waiting, takes a moment.
            std::thread::spawn(move || {
                group
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .kill_tree();
            });
        }
        true
    }

    /// Stop everything running, as the program stops.
    pub fn stop_all(&self) {
        for id in self.running() {
            self.stop(id);
        }
    }

    /// Whether anything is running.
    pub fn busy(&self) -> bool {
        !self.running().is_empty()
    }

    fn running(&self) -> Vec<u64> {
        self.lock()
            .iter()
            .filter(|job| job.lock().state == State::Running)
            .map(|job| job.id)
            .collect()
    }

    fn find(&self, id: u64) -> Option<Arc<Job>> {
        self.lock().iter().find(|job| job.id == id).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<Job>>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn run(job: &Arc<Job>, work: Work) {
    let mut state = State::Succeeded;
    for part in work.parts {
        if job.stop.load(Ordering::SeqCst) {
            state = State::Stopped;
            break;
        }
        let done = match part {
            Part::Run(step) => run_step(job, *step),
            Part::Do(task) => task(&Output(job.clone())).map(|()| true),
        };
        match done {
            Ok(true) => {}
            Ok(false) => {
                state = if job.stop.load(Ordering::SeqCst) {
                    State::Stopped
                } else {
                    State::Failed
                };
                break;
            }
            Err(why) => {
                job.write(format!("{why}\n").as_bytes());
                state = State::Failed;
                break;
            }
        }
    }
    if let Some(tidy) = work.after {
        tidy();
    }
    job.finish(state);
}

/// Run one program to its end, its output and its errors into the job as they come. Whether it
/// succeeded.
fn run_step(job: &Arc<Job>, mut step: Step) -> Result<bool, String> {
    use std::io::Write;
    if !step.shown.is_empty() {
        job.write(format!("$ {}\n", step.shown).as_bytes());
    }
    let program = PathBuf::from(step.command.get_program());
    step.command
        .stdin(if step.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut group = Group::spawn(&mut step.command)
        .map_err(|error| format!("Could not run {}: {error}", program.display()))?;
    if let (Some(text), Some(mut stdin)) = (step.input.take(), group.stdin()) {
        // Short, and the program reads it at once; closed by the drop, which is its end of input.
        let _ = stdin.write_all(text.as_bytes());
    }
    let stdout = group.stdout();
    let stderr = group.stderr();
    let group = Arc::new(Mutex::new(group));
    job.lock().group = Some(group.clone());
    // A stop that came between the last look and this line found no program to end.
    if job.stop.load(Ordering::SeqCst) {
        group
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kill_tree();
    }

    let readers: Vec<_> = [
        stdout.map(|out| Box::new(out) as Box<dyn Read + Send>),
        stderr.map(|err| Box::new(err) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut pipe| {
        let job = job.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => job.write(&buffer[..read]),
                }
            }
        })
    })
    .collect();

    // Polled, not waited for: `stop` needs the lock to end the group.
    let status = loop {
        let waited = group
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_wait();
        match waited {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(error) => break Err(error.to_string()),
        }
    };
    // The pipes close when the last process holding them ends, which may be after the program
    // itself: something it left running in the background still has them. Its output is still
    // read, but the job does not wait for it.
    let ended = std::time::Instant::now();
    for reader in readers {
        while !reader.is_finished() && ended.elapsed() < std::time::Duration::from_millis(500) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if reader.is_finished() {
            let _ = reader.join();
        }
    }
    job.lock().group = None;
    let status = status?;
    if !status.success() && !job.stop.load(Ordering::SeqCst) {
        job.write(format!("Ended with {status}.\n").as_bytes());
    }
    Ok(status.success())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn sh(script: &str) -> Step {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        Step::new(command)
    }

    fn finished(jobs: &Jobs, id: u64) -> Reading {
        let since = Instant::now();
        loop {
            let reading = jobs.read(id, 0).expect("the job");
            if reading.summary.state != State::Running {
                return reading;
            }
            assert!(since.elapsed() < Duration::from_secs(20), "still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_jobs_output_and_errors_are_kept_and_its_end_is_told() {
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "command",
                "both",
                Work::steps(vec![sh("echo out; echo err >&2").shown("both")]),
            )
            .unwrap();
        let reading = finished(&jobs, job.id);
        assert_eq!(reading.summary.state, State::Succeeded);
        assert!(reading.output.starts_with("$ both\n"), "{}", reading.output);
        assert!(reading.output.contains("out\n") && reading.output.contains("err\n"));
        // From where the last reading ended there is nothing new.
        let again = jobs.read(job.id, reading.next).unwrap();
        assert_eq!(again.output, "");
        assert_eq!(again.from, reading.next);
    }

    #[test]
    fn the_first_step_to_fail_ends_the_job() {
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "command",
                "fails",
                Work::steps(vec![sh("echo first; exit 3"), sh("echo never")]),
            )
            .unwrap();
        let reading = finished(&jobs, job.id);
        assert_eq!(reading.summary.state, State::Failed);
        assert!(reading.output.contains("first\n"), "{}", reading.output);
        assert!(!reading.output.contains("never"), "{}", reading.output);
        assert!(reading.output.contains("Ended with"), "{}", reading.output);
    }

    #[test]
    fn stopping_a_job_ends_what_it_started_too() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "command",
                "sleeps",
                Work::steps(vec![sh(&format!(
                    "sleep 600 & echo $! > {}; echo started; wait",
                    pid_file.display()
                ))]),
            )
            .unwrap();
        let since = Instant::now();
        while !jobs.read(job.id, 0).unwrap().output.contains("started") {
            assert!(since.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(jobs.stop(job.id));
        let reading = finished(&jobs, job.id);
        assert_eq!(reading.summary.state, State::Stopped);
        let child: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let since = Instant::now();
        // SAFETY: signal 0 only asks whether the process is there.
        while unsafe { libc::kill(child, 0) } == 0 {
            assert!(
                since.elapsed() < Duration::from_secs(10),
                "the sleep it started outlived the job"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!jobs.stop(job.id), "nothing left to stop");
    }

    #[test]
    fn a_secret_goes_in_on_the_input_and_never_into_the_output() {
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "github",
                "token",
                Work::steps(vec![sh("read line; test \"$line\" = s3cret && echo taken")
                    .shown("gh auth login --with-token")
                    .input("s3cret\n")]),
            )
            .unwrap();
        let reading = finished(&jobs, job.id);
        assert_eq!(
            reading.summary.state,
            State::Succeeded,
            "{}",
            reading.output
        );
        assert!(reading.output.contains("taken"));
        assert!(!reading.output.contains("s3cret"));
    }

    #[test]
    fn only_the_newest_of_a_long_output_is_kept_and_a_reader_is_told_where_it_starts() {
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "command",
                "long",
                Work::steps(vec![sh("i=0; while [ $i -lt 12000 ]; do echo \
                     0123456789012345678901234567890123456789012345678901234567890123456789; \
                     i=$((i+1)); done; echo the-end")]),
            )
            .unwrap();
        let reading = finished(&jobs, job.id);
        assert!(reading.output.len() <= OUTPUT_KEPT);
        assert!(reading.from > 0, "the start is no longer kept");
        assert_eq!(reading.next, reading.from + reading.output.len() as u64);
        assert!(reading.output.ends_with("the-end\n"));
    }

    #[test]
    fn a_task_writes_what_it_does_and_its_failure_is_the_last_line() {
        let jobs = Jobs::default();
        let job = jobs
            .start(
                "clone",
                "task",
                Work::task(|output| {
                    output.line("working");
                    Err("It did not work.".into())
                }),
            )
            .unwrap();
        assert!(!job.stoppable);
        let reading = finished(&jobs, job.id);
        assert_eq!(reading.summary.state, State::Failed);
        assert_eq!(reading.output, "working\nIt did not work.\n");
    }

    #[test]
    fn no_more_than_four_run_at_once() {
        let jobs = Jobs::default();
        let started: Vec<_> = (0..RUNNING_AT_ONCE)
            .map(|_| {
                jobs.start("command", "sleep", Work::steps(vec![sh("sleep 600")]))
                    .unwrap()
            })
            .collect();
        let refused = jobs.start("command", "one more", Work::steps(vec![sh("true")]));
        assert!(refused.unwrap_err().contains("running already"));
        jobs.stop_all();
        for job in started {
            assert_eq!(finished(&jobs, job.id).summary.state, State::Stopped);
        }
    }
}

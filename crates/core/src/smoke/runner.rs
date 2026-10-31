//! Carrying runs out: a thread per run in the app, started by a driver that ticks every second
//! and finds queued rows.
//!
//! One thread per run rather than a step-mover like the workflow driver's, because a step here
//! is a child process whose output must be read as it comes, and a `manual` step is the only
//! one that waits on anything but a process. The thread moves each step as a conditional row
//! move, so a cancel from `ys` is seen between steps, and while a step runs its process is
//! looked at every quarter second for the same.
//!
//! What the window should know comes out as [`Notice`]s through the callback the app gives the
//! driver; the core emits nothing itself.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::exec::{self, Context, StepEnd, Stop, Vars, Watch};
use super::{Action, Phase, Step, Test};
use crate::env::ShellEnv;
use crate::error::IpcResult;
use crate::store::{now_ms, StepChange, Store, TestRunRow};
use crate::workflow::runs::Place;

/// Ended runs older than this are removed, with their output folders.
pub const KEEP_RUNS_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What a run's end says when the app stopped during it.
pub const INTERRUPTED: &str = "Yardsort stopped during the run.";

/// Where runs keep their output, under the data directory.
pub fn runs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("test-runs")
}

/// Something about runs changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// A run or a step changed state: whoever lists runs should look again.
    Runs,
    /// A step's output file grew: whoever tails it should read on.
    Output { run_id: String, step_id: String },
}

pub type Notify = Arc<dyn Fn(Notice) + Send + Sync>;

/// What a driver keeps between ticks. The app holds one for as long as it runs.
pub struct Driver {
    threads: Mutex<HashMap<String, JoinHandle<()>>>,
    recovered: AtomicBool,
    notify: Notify,
    /// The runs that were active at the last tick. One that is gone since was ended from
    /// outside — `ys test cancel` — which is a change too.
    active: Mutex<HashSet<String>>,
}

impl Driver {
    pub fn new(notify: Notify) -> Self {
        Self {
            threads: Mutex::new(HashMap::new()),
            recovered: AtomicBool::new(false),
            notify,
            active: Mutex::new(HashSet::new()),
        }
    }

    /// Start every queued run that is not going yet, and notice what ended. True when anything
    /// about a run changed, so whoever shows runs knows to look again.
    ///
    /// The first tick ends the runs the previous app stopped during, kills what they left
    /// running, and removes old runs.
    pub fn tick(&self, store: &Store, data_dir: &Path, env: &Arc<ShellEnv>) -> IpcResult<bool> {
        let mut touched = false;
        if !self.recovered.swap(true, Ordering::SeqCst) {
            touched |= recover(store)?;
            prune(store, data_dir);
        }
        {
            let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
            let before = threads.len();
            threads.retain(|_, thread| !thread.is_finished());
            touched |= threads.len() != before;
        }
        let runs = store.active_test_runs()?;
        let ids: HashSet<String> = runs.iter().map(|run| run.id.clone()).collect();
        if *self.active.lock().unwrap_or_else(PoisonError::into_inner) != ids {
            touched = true;
        }
        for run in runs.iter().filter(|run| run.status == "queued") {
            let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
            if threads.contains_key(&run.id) {
                continue;
            }
            let job = Job {
                database: data_dir.join("yardsort.db"),
                data_dir: data_dir.to_path_buf(),
                run_id: run.id.clone(),
                env: Arc::clone(env),
                notify: Arc::clone(&self.notify),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("smoke-{}", crate::memory::short_id(&run.id)))
                .spawn(move || carry_out(job));
            match spawned {
                Ok(thread) => {
                    threads.insert(run.id.clone(), thread);
                    touched = true;
                }
                Err(error) => {
                    eprintln!(
                        "smoke tests: cannot start a thread for run {}: {error}",
                        run.id
                    );
                }
            }
        }
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = store
            .active_test_runs()?
            .into_iter()
            .map(|run| run.id)
            .collect();
        Ok(touched)
    }

    /// How many runs this driver is carrying out now.
    pub fn going(&self) -> usize {
        self.threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|thread| !thread.is_finished())
            .count()
    }

    /// Wait for every run going now to end. For tests, and for the app on its way out.
    pub fn join(&self) {
        let threads: Vec<JoinHandle<()>> = self
            .threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, thread)| thread)
            .collect();
        for thread in threads {
            let _ = thread.join();
        }
    }
}

/// Runs found `running` were interrupted by the app stopping: they are ended, and the `start`
/// processes they left are killed if they are still the same processes. True when any was.
fn recover(store: &Store) -> IpcResult<bool> {
    let interrupted = store.end_interrupted_test_runs(INTERRUPTED)?;
    for run in &interrupted {
        for step in store.test_steps(&run.id)? {
            if step.action != "start" {
                continue;
            }
            let outputs = step.outputs();
            let (Some(pid), Some(print)) = (
                outputs.get("pid").and_then(|p| p.parse::<u32>().ok()),
                outputs.get("process"),
            ) else {
                continue;
            };
            if crate::process::kill_orphan(pid, print) {
                eprintln!(
                    "smoke tests: stopped `{}` of run {}, left running when Yardsort stopped",
                    step.step_id,
                    crate::memory::short_id(&run.id)
                );
            }
        }
    }
    Ok(!interrupted.is_empty())
}

/// Remove ended runs older than [`KEEP_RUNS_FOR`], and their output folders — and any output
/// folder whose run is gone already, however that happened.
fn prune(store: &Store, data_dir: &Path) {
    let before = now_ms() - i64::try_from(KEEP_RUNS_FOR.as_millis()).unwrap_or(i64::MAX);
    match store.prune_test_runs(before) {
        Ok(gone) => {
            for (_, output_dir) in gone {
                if let Some(dir) = output_dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
        }
        Err(error) => eprintln!("smoke tests: cannot prune old runs: {error}"),
    }
    let Ok(kept) = store.test_runs(None, usize::MAX) else {
        return;
    };
    let ids: HashSet<String> = kept.into_iter().map(|run| run.id).collect();
    let Ok(folders) = std::fs::read_dir(runs_dir(data_dir)) else {
        return;
    };
    for folder in folders.flatten() {
        let name = folder.file_name().to_string_lossy().into_owned();
        if !ids.contains(&name) {
            let _ = std::fs::remove_dir_all(folder.path());
        }
    }
}

/// What a run's thread is given.
struct Job {
    database: PathBuf,
    data_dir: PathBuf,
    run_id: String,
    env: Arc<ShellEnv>,
    notify: Notify,
}

/// A `start` step's process, the files it writes, and whether it has been seen to end.
struct Server {
    started: exec::Started,
    ended: Option<String>,
}

/// What the runner tells a step while it runs: whether to stop, and why. Checked against the
/// store — a cancel from another process — and against the clock.
struct Watching<'a> {
    store: &'a Store,
    run_id: &'a str,
    deadline: Instant,
    servers: &'a Mutex<Vec<Server>>,
    notify: &'a Notify,
    /// Teardown steps are not stopped by a cancel or the run's limit: they are what makes a
    /// stopped run leave nothing behind.
    teardown: bool,
    /// The store is asked at most this often.
    last_asked: Mutex<Instant>,
    cancelled: AtomicBool,
    /// How big each server's output files were when the window was last told, and when.
    server_sizes: Mutex<Sizes>,
}

/// How big each server's output files were when the window was last told, and when.
struct Sizes {
    by_step: HashMap<String, (u64, u64)>,
    told_at: Instant,
}

impl Watch for Watching<'_> {
    fn stop(&self) -> Option<Stop> {
        if self.teardown {
            // A server that writes past the cap is stopped in the teardown too; nothing else
            // stops a teardown.
            let _ = self.police_servers();
            return None;
        }
        if self.cancelled.load(Ordering::Relaxed) {
            return Some(Stop::Cancelled);
        }
        {
            let mut asked = self
                .last_asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if asked.elapsed() >= Duration::from_millis(250) {
                *asked = Instant::now();
                if self
                    .store
                    .test_run_cancel_requested(self.run_id)
                    .unwrap_or(false)
                {
                    self.cancelled.store(true, Ordering::Relaxed);
                    return Some(Stop::Cancelled);
                }
            }
        }
        if Instant::now() >= self.deadline {
            return Some(Stop::RunTimedOut);
        }
        self.police_servers()
    }

    fn output_grew(&self, step_id: &str) {
        (self.notify)(Notice::Output {
            run_id: self.run_id.to_owned(),
            step_id: step_id.to_owned(),
        });
    }
}

impl Watching<'_> {
    /// Look at every server still going: one that ended is the run's end, and one that wrote
    /// past the output cap is stopped and is the run's end too. The cap holds while another
    /// step runs and while a `manual` step waits, which may be a long time.
    fn police_servers(&self) -> Option<Stop> {
        let mut servers = self.servers.lock().unwrap_or_else(PoisonError::into_inner);
        self.servers_wrote(&servers);
        for server in servers.iter_mut() {
            if server.ended.is_some() {
                continue;
            }
            if let Ok(Some(status)) = server.started.group.try_wait() {
                let exit = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "a signal".to_owned());
                server.ended = Some(exit.clone());
                return Some(Stop::ServerExited {
                    step_id: server.started.step_id.clone(),
                    exit,
                });
            }
            let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if size(&server.started.stdout) > exec::MAX_OUTPUT_BYTES
                || size(&server.started.stderr) > exec::MAX_OUTPUT_BYTES
            {
                server.started.group.kill_tree();
                server.ended = Some("too much output".to_owned());
                return Some(Stop::ServerOutputCapped {
                    step_id: server.started.step_id.clone(),
                });
            }
        }
        None
    }

    /// A server's log grows while other steps run; the window is told every half second, so
    /// the tab tailing it reads on.
    fn servers_wrote(&self, servers: &[Server]) {
        let mut sizes = self
            .server_sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if sizes.told_at.elapsed() < Duration::from_millis(500) {
            return;
        }
        sizes.told_at = Instant::now();
        for server in servers {
            let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let now = (size(&server.started.stdout), size(&server.started.stderr));
            if sizes.by_step.insert(server.started.step_id.clone(), now) != Some(now) {
                self.output_grew(&server.started.step_id);
            }
        }
    }
}

/// How a phase ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PhaseEnd {
    Passed,
    Failed,
    Stopped(Stop),
    /// The run ended from outside: nothing more of it happens.
    Gone,
}

fn carry_out(job: Job) {
    let store = match Store::open(&job.database) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "smoke tests: run {}: cannot open the store: {error}",
                job.run_id
            );
            return;
        }
    };
    if let Err(error) = run(&store, &job) {
        eprintln!("smoke tests: run {}: {}", job.run_id, error.message);
    }
    (job.notify)(Notice::Runs);
}

/// What a run leaves until it is over, and what happens to it however the run ends: the
/// servers are stopped and the scratch folder removed, and a run the thread could not see to
/// the end of — a store error, a panic — is ended as errored rather than left `running`,
/// which nothing else would end before the next app start.
struct Leftovers<'a> {
    store: &'a Store,
    run_id: &'a str,
    servers: Mutex<Vec<Server>>,
    scratch: PathBuf,
    /// What to say if the run is still running at the end.
    unfinished: Option<String>,
}

impl Leftovers<'_> {
    /// Stop every server and remove the scratch folder. Done once; the guard does it if the
    /// run did not get here.
    fn clear(&mut self) {
        for server in self
            .servers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            let mut server = server;
            server.started.group.kill_tree();
        }
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

impl Drop for Leftovers<'_> {
    fn drop(&mut self) {
        self.clear();
        let why = self
            .unfinished
            .clone()
            .unwrap_or_else(|| "Yardsort could not see the run through.".to_owned());
        let running = self
            .store
            .test_run(self.run_id)
            .ok()
            .flatten()
            .is_some_and(|run| run.status == "running");
        if running {
            // The step the thread was on, if any, is failed with the reason; the rest is
            // skipped, the teardown's too, as nothing will run it.
            if let Ok(steps) = self.store.test_steps(self.run_id) {
                for step in steps {
                    if matches!(step.status.as_str(), "running" | "waiting") {
                        let _ = mark(
                            self.store,
                            self.run_id,
                            &step.step_id,
                            &["running", "waiting"],
                            "failed",
                            None,
                            None,
                            Some(&why),
                        );
                    }
                }
            }
            let _ = skip_rest(self.store, self.run_id, "skipped", &why, true);
            let _ = self
                .store
                .finish_test_run(self.run_id, "errored", Some(&why));
        }
    }
}

fn run(store: &Store, job: &Job) -> IpcResult<()> {
    let Some(row) = store.test_run(&job.run_id)? else {
        return Ok(());
    };
    let errored = |why: &str| -> IpcResult<()> {
        // A run that cannot start still starts, so its end is recorded the same way.
        if store.start_test_run(&row.id, 0, "")? {
            skip_rest(store, &row.id, "skipped", why, true)?;
            store.finish_test_run(&row.id, "errored", Some(why))?;
        }
        Ok(())
    };
    let Ok(test) = super::parse(&row.definition) else {
        return errored("The test file no longer checks out.");
    };
    let git = crate::git::Git::new(&job.env)?;
    let place = match row.workspace_id.as_deref() {
        Some(id) => Place::load(store, id, &git)?,
        None => None,
    };
    let Some(place) = place else {
        return errored("The workspace is gone.");
    };
    let worktree = PathBuf::from(&place.workspace.path);
    if !worktree.is_dir() {
        return errored("The workspace's folder is gone.");
    }
    let output_dir = runs_dir(&job.data_dir).join(&row.id);
    let scratch = output_dir.join("scratch");
    if let Err(error) = std::fs::create_dir_all(&scratch) {
        return errored(&format!("Cannot make {}: {error}", output_dir.display()));
    }
    let port = match free_port() {
        Ok(port) => port,
        Err(error) => return errored(&format!("Cannot find a free port: {error}")),
    };
    if !store.start_test_run(&row.id, port, &output_dir.to_string_lossy())? {
        return Ok(()); // Cancelled between the tick and now.
    }
    (job.notify)(Notice::Runs);
    // From here the run is running, and whatever happens it must stop being so.
    let mut left = Leftovers {
        store,
        run_id: &row.id,
        servers: Mutex::new(Vec::new()),
        scratch: scratch.clone(),
        unfinished: None,
    };
    let deadline = Instant::now() + Duration::from_secs(u64::from(test.timeout_secs));
    let carried = (|| -> IpcResult<PhaseEnd> {
        let env: BTreeMap<String, String> = match &row.project_id {
            Some(project_id) => store.project_test_env(project_id)?.into_iter().collect(),
            None => BTreeMap::new(),
        };
        let mut vars = Vars {
            place,
            run_id: row.id.clone(),
            port,
            dir: scratch.clone(),
            inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
            env,
            steps: BTreeMap::new(),
        };
        phases(
            store,
            job,
            &row,
            &test,
            &worktree,
            &output_dir,
            &mut vars,
            deadline,
            &left.servers,
        )
    })();
    let outcome = match carried {
        Ok(outcome) => outcome,
        Err(error) => {
            left.unfinished = Some(format!(
                "Yardsort could not see the run through: {}",
                error.message
            ));
            return Err(error);
        }
    };
    left.clear();

    let (status, error) = match outcome {
        PhaseEnd::Passed => ("passed", None),
        PhaseEnd::Failed => ("failed", None),
        PhaseEnd::Stopped(Stop::Cancelled) => {
            ("cancelled", store.test_run(&row.id)?.and_then(|r| r.error))
        }
        PhaseEnd::Stopped(Stop::RunTimedOut) => (
            "failed",
            Some(format!(
                "Timed out after {}.",
                exec::duration_words(u64::from(test.timeout_secs))
            )),
        ),
        PhaseEnd::Stopped(Stop::ServerExited { step_id, exit }) => (
            "failed",
            Some(format!(
                "`{step_id}` exited with {exit} before the run ended."
            )),
        ),
        PhaseEnd::Stopped(Stop::ServerOutputCapped { step_id }) => (
            "failed",
            Some(format!(
                "`{step_id}` wrote more than {} of output, so it was stopped.",
                exec::output_cap_words()
            )),
        ),
        PhaseEnd::Gone => return Ok(()),
    };
    store.finish_test_run(&row.id, status, error.as_deref())?;
    Ok(())
}

/// Setup, steps and teardown, in order: setup and steps until the first failure or stop, the
/// teardown whatever happened, as long as the run is still the runner's.
#[allow(clippy::too_many_arguments)]
fn phases(
    store: &Store,
    job: &Job,
    row: &TestRunRow,
    test: &Test,
    worktree: &Path,
    output_dir: &Path,
    vars: &mut Vars,
    deadline: Instant,
    servers: &Mutex<Vec<Server>>,
) -> IpcResult<PhaseEnd> {
    let mut outcome = phase(
        store,
        job,
        row,
        test,
        Phase::Setup,
        worktree,
        output_dir,
        vars,
        deadline,
        servers,
    )?;
    if outcome == PhaseEnd::Passed {
        outcome = phase(
            store,
            job,
            row,
            test,
            Phase::Steps,
            worktree,
            output_dir,
            vars,
            deadline,
            servers,
        )?;
    }
    if outcome != PhaseEnd::Gone {
        // What setup or steps did not reach is skipped, with the reason, before the teardown.
        let why = match &outcome {
            PhaseEnd::Passed | PhaseEnd::Gone => None,
            PhaseEnd::Failed => Some("An earlier step failed.".to_owned()),
            PhaseEnd::Stopped(stop) => Some(skip_reason(stop)),
        };
        if let Some(why) = why {
            let to = if outcome == PhaseEnd::Stopped(Stop::Cancelled) {
                "cancelled"
            } else {
                "skipped"
            };
            if !skip_rest(store, &row.id, to, &why, false)? {
                outcome = PhaseEnd::Gone;
            }
        }
    }
    if outcome != PhaseEnd::Gone {
        let teardown = phase(
            store,
            job,
            row,
            test,
            Phase::Teardown,
            worktree,
            output_dir,
            vars,
            deadline,
            servers,
        )?;
        if teardown == PhaseEnd::Gone {
            outcome = PhaseEnd::Gone;
        }
    }
    Ok(outcome)
}

/// Why the steps after a stop did not run.
fn skip_reason(stop: &Stop) -> String {
    match stop {
        Stop::Cancelled => "The run was cancelled.".to_owned(),
        Stop::RunTimedOut => "The run's time limit passed.".to_owned(),
        Stop::ServerExited { step_id, exit } => {
            format!("`{step_id}` exited with {exit} before the run ended.")
        }
        Stop::ServerOutputCapped { step_id } => format!(
            "`{step_id}` wrote more than {} of output, so it was stopped.",
            exec::output_cap_words()
        ),
    }
}

/// Run one phase's steps in order. Setup and steps stop at the first failure; teardown runs
/// every step whatever happens. Steps not reached are marked skipped with the reason.
#[allow(clippy::too_many_arguments)]
fn phase(
    store: &Store,
    job: &Job,
    row: &TestRunRow,
    test: &Test,
    which: Phase,
    worktree: &Path,
    output_dir: &Path,
    vars: &mut Vars,
    deadline: Instant,
    servers: &Mutex<Vec<Server>>,
) -> IpcResult<PhaseEnd> {
    let steps: &[Step] = match which {
        Phase::Setup => &test.setup,
        Phase::Steps => &test.steps,
        Phase::Teardown => &test.teardown,
    };
    let watching = Watching {
        store,
        run_id: &row.id,
        deadline,
        servers,
        notify: &job.notify,
        teardown: which == Phase::Teardown,
        last_asked: Mutex::new(Instant::now() - Duration::from_secs(1)),
        cancelled: AtomicBool::new(false),
        server_sizes: Mutex::new(Sizes {
            by_step: HashMap::new(),
            told_at: Instant::now() - Duration::from_secs(1),
        }),
    };
    let mut end = PhaseEnd::Passed;
    for (index, step) in steps.iter().enumerate() {
        if end != PhaseEnd::Passed && which != Phase::Teardown {
            // Everything after a failure or a stop is skipped, with the reason.
            let why = match &end {
                PhaseEnd::Failed => format!("An earlier step failed (`{}`).", steps[index - 1].id),
                PhaseEnd::Stopped(stop) => skip_reason(stop),
                PhaseEnd::Passed | PhaseEnd::Gone => unreachable!(),
            };
            let to = if end == PhaseEnd::Stopped(Stop::Cancelled) {
                "cancelled"
            } else {
                "skipped"
            };
            if !mark(
                store,
                &row.id,
                &step.id,
                &["pending"],
                to,
                None,
                None,
                Some(&why),
            )? {
                return Ok(PhaseEnd::Gone);
            }
            continue;
        }
        if let Some(stop) = watching.stop() {
            // Between steps: the stop is seen before the next one starts.
            note_server_stop(store, &row.id, &stop, servers)?;
            let to = if stop == Stop::Cancelled {
                "cancelled"
            } else {
                "skipped"
            };
            let why = skip_reason(&stop);
            end = PhaseEnd::Stopped(stop);
            if !mark(
                store,
                &row.id,
                &step.id,
                &["pending"],
                to,
                None,
                None,
                Some(&why),
            )? {
                return Ok(PhaseEnd::Gone);
            }
            continue;
        }
        if let Action::Manual { prompt } = &step.action {
            let rendered = vars.render(prompt);
            let outputs = serde_json::json!({ "prompt": rendered }).to_string();
            if !mark(
                store,
                &row.id,
                &step.id,
                &["pending"],
                "waiting",
                None,
                Some(&outputs),
                None,
            )? {
                return Ok(PhaseEnd::Gone);
            }
            (job.notify)(Notice::Runs);
            match wait_for_answer(store, &row.id, &step.id, &watching)? {
                Answer::Given { passed, outputs } => {
                    vars.steps.insert(step.id.clone(), outputs);
                    if !passed && which != Phase::Teardown {
                        end = PhaseEnd::Failed;
                    }
                }
                Answer::Stopped(stop) => {
                    note_server_stop(store, &row.id, &stop, servers)?;
                    end = PhaseEnd::Stopped(stop);
                }
                Answer::Gone => return Ok(PhaseEnd::Gone),
            }
            (job.notify)(Notice::Runs);
            continue;
        }
        if !mark(
            store,
            &row.id,
            &step.id,
            &["pending"],
            "running",
            None,
            None,
            None,
        )? {
            return Ok(PhaseEnd::Gone);
        }
        (job.notify)(Notice::Runs);
        let ctx = Context {
            env: &job.env,
            worktree,
            output_dir,
            vars,
        };
        let mut started = Vec::new();
        let done: StepEnd = exec::carry_out(&ctx, &watching, &step.id, &step.action, &mut started);
        for started in started {
            servers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Server {
                    started,
                    ended: None,
                });
        }
        let outputs = serde_json::to_string(&done.outputs).unwrap_or_else(|_| "{}".to_owned());
        if !mark(
            store,
            &row.id,
            &step.id,
            &["running"],
            done.status,
            done.exit,
            Some(&outputs),
            done.note.as_deref(),
        )? {
            return Ok(PhaseEnd::Gone);
        }
        (job.notify)(Notice::Runs);
        vars.steps.insert(step.id.clone(), done.outputs);
        if let Some(stop) = &done.stopped {
            note_server_stop(store, &row.id, stop, servers)?;
        }
        if which == Phase::Teardown {
            continue;
        }
        match done.stopped {
            Some(stop) => end = PhaseEnd::Stopped(stop),
            None if done.status != "passed" => end = PhaseEnd::Failed,
            None => {}
        }
    }
    Ok(end)
}

/// A `start` step whose process ended before the run did, or wrote past the cap and was
/// stopped, is marked failed, with the last lines it wrote. Other stops are not a server's.
fn note_server_stop(
    store: &Store,
    run_id: &str,
    stop: &Stop,
    servers: &Mutex<Vec<Server>>,
) -> IpcResult<()> {
    let (step_id, mut note) = match stop {
        Stop::ServerExited { step_id, exit } => {
            (step_id, format!("Exited with {exit} before the run ended."))
        }
        Stop::ServerOutputCapped { step_id } => (
            step_id,
            format!(
                "Wrote more than {} of output, so it was stopped.",
                exec::output_cap_words()
            ),
        ),
        Stop::Cancelled | Stop::RunTimedOut => return Ok(()),
    };
    // What a server wrote past the cap is not worth quoting; what one wrote before dying is.
    let tail = if matches!(stop, Stop::ServerOutputCapped { .. }) {
        String::new()
    } else {
        let servers = servers.lock().unwrap_or_else(PoisonError::into_inner);
        servers
            .iter()
            .find(|s| s.started.step_id == *step_id)
            .map(|s| {
                let stderr = exec::read_capped(&s.started.stderr, exec::MAX_VARIABLE_BYTES);
                let text = if stderr.trim().is_empty() {
                    exec::read_capped(&s.started.stdout, exec::MAX_VARIABLE_BYTES)
                } else {
                    stderr
                };
                exec::last_lines(&text, 8)
            })
            .unwrap_or_default()
    };
    if !tail.is_empty() {
        note.push_str("\nlast lines:\n");
        note.push_str(&exec::indent(&tail, "  "));
    }
    mark(
        store,
        run_id,
        step_id,
        &["passed"],
        "failed",
        None,
        None,
        Some(&note),
    )?;
    Ok(())
}

/// How a `manual` step's wait ended.
enum Answer {
    /// A person answered, and the step left its note.
    Given {
        passed: bool,
        outputs: BTreeMap<String, String>,
    },
    /// The run stopped while it waited: a cancel is still a cancel, not a failed answer.
    Stopped(Stop),
    /// The run ended from outside.
    Gone,
}

/// Wait for a `manual` step to be answered in the app. A cancel, the run's limit or a server
/// going ends the wait with that reason.
fn wait_for_answer(
    store: &Store,
    run_id: &str,
    step_id: &str,
    watching: &Watching<'_>,
) -> IpcResult<Answer> {
    loop {
        let Some(step) = store.test_step(run_id, step_id)? else {
            return Ok(Answer::Gone);
        };
        match step.status.as_str() {
            "passed" => {
                return Ok(Answer::Given {
                    passed: true,
                    outputs: step.outputs(),
                })
            }
            "failed" => {
                return Ok(Answer::Given {
                    passed: false,
                    outputs: step.outputs(),
                })
            }
            "waiting" => {}
            _ => return Ok(Answer::Gone),
        }
        if let Some(stop) = watching.stop() {
            let (to, note) = match &stop {
                Stop::Cancelled => (
                    "cancelled",
                    "The run was cancelled while this waited.".to_owned(),
                ),
                Stop::RunTimedOut => (
                    "failed",
                    "The run's time limit passed while this waited.".to_owned(),
                ),
                Stop::ServerExited { step_id, exit } => (
                    "failed",
                    format!("`{step_id}` exited with {exit} while this waited."),
                ),
                Stop::ServerOutputCapped { step_id } => (
                    "failed",
                    format!(
                        "`{step_id}` wrote more than {} of output while this waited, so it was stopped.",
                        exec::output_cap_words()
                    ),
                ),
            };
            if !mark(
                store,
                run_id,
                step_id,
                &["waiting"],
                to,
                None,
                None,
                Some(&note),
            )? {
                return Ok(Answer::Gone);
            }
            return Ok(Answer::Stopped(stop));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Every step still pending — the teardown's too, when `teardown` — becomes `to`, with the
/// reason. False when the run ended from outside meanwhile.
fn skip_rest(store: &Store, run_id: &str, to: &str, why: &str, teardown: bool) -> IpcResult<bool> {
    for step in store.test_steps(run_id)? {
        if step.status != "pending" || (!teardown && step.phase == "teardown") {
            continue;
        }
        if !mark(
            store,
            run_id,
            &step.step_id,
            &["pending"],
            to,
            None,
            None,
            Some(why),
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn mark(
    store: &Store,
    run_id: &str,
    step_id: &str,
    from: &[&str],
    to: &str,
    exit: Option<i64>,
    outputs: Option<&str>,
    note: Option<&str>,
) -> IpcResult<bool> {
    Ok(store.move_test_step(
        run_id,
        step_id,
        from,
        &StepChange {
            to,
            exit,
            outputs,
            note,
            screenshots: None,
        },
    )?)
}

/// A port nobody is listening on right now: bound and let go. What is started on it must be
/// quick about it, as anything on this machine may take it meanwhile.
pub fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests;

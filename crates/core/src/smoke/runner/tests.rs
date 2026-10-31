//! Runner tests: real processes, a real server on the run's port, a real cancel.
//!
//! The programs the tests run are `bun` scripts, because `bun` is on every CI runner for the
//! frontend already and speaks the same on all three platforms. Where it is not found the
//! tests that need it say so and pass, rather than fail for a reason that is not the runner's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::env::{EnvSource, ShellEnv};
use crate::smoke::report;
use crate::smoke::runs::{queue, Request};
use crate::store::{ProjectRow, Store, TestStepRow, WorkspaceRow};

/// A server on the run's port, from the environment, answering `/health` with JSON and
/// anything else with text.
const SERVER: &str = r#"const port = Number(process.env.YS_RUN_PORT);
Bun.serve({ port, fetch(req) {
  const url = new URL(req.url);
  if (url.pathname === '/health') return Response.json({ status: 'ok', db: { up: true }, port });
  if (url.pathname === '/echo') return req.text().then(t => new Response('got ' + t));
  return new Response('hello from the server', { status: 200 });
}});
console.error('listening on ' + port);"#;

struct Fx {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    store: Store,
    _project: ProjectRow,
    workspace: WorkspaceRow,
    env: Arc<ShellEnv>,
    notices: Arc<Mutex<Vec<Notice>>>,
    driver: Driver,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(crate::smoke::dir(&worktree)).unwrap();
        let store = Store::open(&data_dir.join("yardsort.db")).unwrap();
        let project = store
            .add_project("app", &dir.path().join("code").to_string_lossy())
            .unwrap();
        let workspace = store
            .add_worktree(&project.id, "fix", &worktree.to_string_lossy(), None, None)
            .unwrap();
        let env = Arc::new(ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        });
        let notices: Arc<Mutex<Vec<Notice>>> = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&notices);
        let driver = Driver::new(Arc::new(move |notice| heard.lock().unwrap().push(notice)));
        Self {
            _dir: dir,
            data_dir,
            store,
            _project: project,
            workspace,
            env,
            notices,
            driver,
        }
    }

    fn worktree(&self) -> PathBuf {
        PathBuf::from(&self.workspace.path)
    }

    fn write(&self, id: &str, yaml: &str) {
        std::fs::write(
            crate::smoke::dir(&self.worktree()).join(format!("{id}.yaml")),
            yaml,
        )
        .unwrap();
    }

    /// Queue a run of `id` and approve it, as the user would in the app.
    fn queue(&self, id: &str, inputs: &[(&str, &str)]) -> String {
        let inputs: BTreeMap<String, String> = inputs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let queued = queue(
            &self.store,
            &Request {
                test_id: id,
                workspace_id: &self.workspace.id,
                inputs: &inputs,
                requested_by: "cli",
                session_record_id: None,
            },
        )
        .unwrap();
        assert!(queued.awaiting_approval, "a file nobody has seen waits");
        assert!(self.store.approve_test_run(&queued.id).unwrap());
        queued.id
    }

    fn tick(&self) -> bool {
        self.driver
            .tick(&self.store, &self.data_dir, &self.env)
            .unwrap()
    }

    /// Carry the queued runs out, and wait for them.
    fn drive(&self) {
        assert!(self.tick(), "a queued run is a change");
        self.driver.join();
        assert!(self.tick(), "a finished run is a change");
    }

    fn run(&self, id: &str) -> TestRunRow {
        self.store.test_run(id).unwrap().unwrap()
    }

    fn steps(&self, id: &str) -> Vec<TestStepRow> {
        self.store.test_steps(id).unwrap()
    }

    fn step<'a>(&self, steps: &'a [TestStepRow], id: &str) -> &'a TestStepRow {
        steps.iter().find(|s| s.step_id == id).unwrap()
    }

    fn summary(&self, id: &str) -> String {
        report::summary(&self.run(id), &self.steps(id))
    }

    /// Wait until `found` says so about the run's steps, for at most ten seconds.
    fn until(&self, id: &str, found: impl Fn(&[TestStepRow]) -> bool) {
        let started = Instant::now();
        while !found(&self.steps(id)) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "still waiting after ten seconds: {:?}",
                self.steps(id)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn has_bun(&self) -> bool {
        if self.env.find_program("bun", &self.worktree()).is_some() {
            return true;
        }
        eprintln!("skipped: `bun` is not on the PATH, and the test's programs are bun scripts");
        false
    }
}

/// A `bun -e` step, as YAML list items.
fn bun(script: &str) -> String {
    format!("program: bun\n    args: [-e, {}]", yaml_string(script))
}

fn yaml_string(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// Whether the process `pid` is gone, or at least not the one that was started.
fn gone(outputs: &BTreeMap<String, String>) -> bool {
    let pid: u32 = outputs["pid"].parse().unwrap();
    match outputs.get("process") {
        Some(print) => crate::process::fingerprint(pid).as_deref() != Some(print),
        None => true,
    }
}

#[test]
fn a_server_on_the_runs_port_is_waited_for_asked_and_stopped() {
    let fx = Fx::new();
    if !fx.has_bun() {
        return;
    }
    fx.write(
        "health",
        &format!(
            r#"id: health
name: The server answers
version: 1
inputs:
  - id: greeting
    kind: text
    default: hello
setup:
  - id: server
    action: start
    {server}
  - id: ready
    action: wait_for
    port: "{{{{ run.port }}}}"
    timeout: 20s
steps:
  - id: health
    action: http
    get: "http://127.0.0.1:{{{{ run.port }}}}/health"
    expect:
      status: 200
      json: {{ status: ok, db: {{ up: true }} }}
      header: {{ name: Content-Type, contains: json }}
  - id: echo
    action: http
    post: "http://127.0.0.1:{{{{ run.port }}}}/echo"
    body: "{{{{ inputs.greeting }}}} from {{{{ workspace.name }}}}"
    expect:
      body_contains: "got hello from fix"
  - id: url
    action: wait_for
    url: "http://127.0.0.1:{{{{ run.port }}}}/"
  - id: env
    action: run
    {env}
    expect:
      stdout_contains: "port={{{{ run.port }}}} input=hello"
  - id: earlier
    action: run
    {earlier}
    env:
      STATUS: "{{{{ steps.health.status }}}}"
      BODY: "{{{{ steps.health.body }}}}"
    expect:
      stdout_json: {{ path: status, equals: 200 }}
      stdout_contains: '"ok":true'
teardown:
  - id: bye
    action: run
    {bye}
"#,
            server = bun(SERVER),
            env = bun("console.log(`port=${process.env.YS_RUN_PORT} input=${process.env.YS_INPUT_GREETING}`)"),
            // `earlier` reads what the http step left, through variables in its environment.
            earlier = bun("console.log(JSON.stringify({status: Number(process.env.STATUS), ok: process.env.BODY.includes('\"status\":\"ok\"')}))"),
            bye = bun("console.log('bye')"),
        ),
    );
    let id = fx.queue("health", &[]);
    fx.drive();

    let run = fx.run(&id);
    let steps = fx.steps(&id);
    assert_eq!(run.status, "passed", "{}", fx.summary(&id));
    assert!(run.port.is_some() && run.started_at.is_some() && run.ended_at.is_some());
    for step in &steps {
        assert_eq!(step.status, "passed", "{}: {:?}", step.step_id, step.note);
    }
    let server = fx.step(&steps, "server").outputs();
    assert!(gone(&server), "the server was stopped with the run");
    let health = fx.step(&steps, "health").outputs();
    assert_eq!(health["status"], "200");
    assert!(health["body"].contains("\"status\":\"ok\""));
    assert!(health["headers"].contains("content-type"));
    let ready = fx.step(&steps, "ready").outputs();
    assert!(ready["waited"].parse::<f64>().is_ok(), "{ready:?}");
    let env = fx.step(&steps, "env");
    assert_eq!(env.exit, Some(0));
    let outputs = env.outputs();
    let stdout = Path::new(&outputs["stdout"]);
    assert!(stdout.starts_with(runs_dir(&fx.data_dir).join(&id)));
    assert!(std::fs::read_to_string(stdout)
        .unwrap()
        .contains("input=hello"));
    assert!(
        !runs_dir(&fx.data_dir).join(&id).join("scratch").exists(),
        "the scratch folder went with the run"
    );
    let notices = fx.notices.lock().unwrap();
    assert!(notices.contains(&Notice::Runs));
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, Notice::Output { step_id, .. } if step_id == "server")),
        "the server's output grew, and the window was told: {notices:?}"
    );
    let summary = fx.summary(&id);
    assert!(
        summary.starts_with("passed  health  (7 of 7 steps passed, "),
        "{summary}"
    );
}

#[test]
fn a_failing_expectation_fails_the_run_skips_the_rest_and_still_tears_down() {
    let fx = Fx::new();
    if !fx.has_bun() {
        return;
    }
    fx.write(
        "broken",
        &format!(
            r#"id: broken
name: Something is off
version: 1
steps:
  - id: seed
    action: run
    {seed}
    expect:
      stdout_contains: seeded
  - id: after
    action: run
    {after}
teardown:
  - id: reset
    action: run
    {reset}
"#,
            seed = bun(
                "console.log('nothing happened'); console.error('db is down'); process.exit(3)"
            ),
            after = bun("console.log('never')"),
            reset = bun("console.log('reset')"),
        ),
    );
    let id = fx.queue("broken", &[]);
    fx.drive();

    let run = fx.run(&id);
    assert_eq!(run.status, "failed");
    let steps = fx.steps(&id);
    let seed = fx.step(&steps, "seed");
    assert_eq!(seed.status, "failed");
    assert_eq!(seed.exit, Some(3));
    let note = seed.note.as_deref().unwrap();
    assert!(note.contains("exit: expected 0, got 3."), "{note}");
    assert!(
        note.contains("db is down"),
        "the last lines of stderr: {note}"
    );
    assert!(
        note.contains("stdout_contains \"seeded\": not found"),
        "{note}"
    );
    let after = fx.step(&steps, "after");
    assert_eq!(after.status, "skipped");
    assert_eq!(
        after.note.as_deref(),
        Some("An earlier step failed (`seed`).")
    );
    assert!(after.started_at.is_none());
    let reset = fx.step(&steps, "reset");
    assert_eq!(reset.status, "passed", "teardown ran: {:?}", reset.note);
    let summary = fx.summary(&id);
    assert!(
        summary.starts_with("failed  broken  (0 of 2 steps passed, "),
        "{summary}"
    );
    assert!(summary.contains("  seed  run  failed after "), "{summary}");
    assert!(
        summary.contains("    exit: expected 0, got 3."),
        "{summary}"
    );
    assert!(
        !summary.contains("reset"),
        "a teardown that passed is not news: {summary}"
    );
    assert_eq!(report::exit_code(&run.status), 1);
}

#[test]
fn a_cancel_mid_step_stops_the_process_and_still_tears_down() {
    let fx = Fx::new();
    if !fx.has_bun() {
        return;
    }
    fx.write(
        "slow",
        &format!(
            r#"id: slow
name: Takes a while
version: 1
steps:
  - id: wait
    action: run
    {wait}
  - id: after
    action: run
    {after}
teardown:
  - id: reset
    action: run
    {reset}
"#,
            wait = bun("console.log('working'); setTimeout(() => {}, 60000)"),
            after = bun("console.log('never')"),
            reset = bun("console.log('reset')"),
        ),
    );
    let id = fx.queue("slow", &[]);
    assert!(fx.tick());
    fx.until(&id, |steps| {
        steps
            .iter()
            .any(|s| s.step_id == "wait" && s.status == "running")
    });
    // From another connection, as `ys test cancel` is.
    let elsewhere = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    assert!(elsewhere
        .cancel_test_run(&id, "Cancelled from ys.")
        .unwrap());
    let asked = Instant::now();
    fx.driver.join();
    assert!(
        asked.elapsed() < Duration::from_secs(8),
        "stopped without waiting the minute out"
    );

    let run = fx.run(&id);
    assert_eq!(run.status, "cancelled");
    assert_eq!(run.error.as_deref(), Some("Cancelled from ys."));
    let steps = fx.steps(&id);
    let wait = fx.step(&steps, "wait");
    assert_eq!(wait.status, "cancelled");
    assert_eq!(wait.note.as_deref(), Some("Cancelled while this ran."));
    assert_eq!(fx.step(&steps, "after").status, "cancelled");
    assert_eq!(
        fx.step(&steps, "reset").status,
        "passed",
        "teardown still ran"
    );
    assert_eq!(report::exit_code(&run.status), 2);
}

#[test]
fn a_shell_line_gets_the_run_in_its_environment_not_in_its_text() {
    let fx = Fx::new();
    let line = if cfg!(windows) {
        "Write-Output \"port=$env:YS_RUN_PORT ws=$env:YS_WORKSPACE_PATH\""
    } else {
        "echo \"port=$YS_RUN_PORT ws=$YS_WORKSPACE_PATH\" | cat"
    };
    fx.write(
        "sh",
        &format!(
            r#"id: sh
name: A shell line
version: 1
steps:
  - id: say
    action: shell
    line: {line}
    expect:
      stdout_contains: "port={{{{ run.port }}}} ws={{{{ workspace.path }}}}"
"#,
            line = yaml_string(line)
        ),
    );
    let id = fx.queue("sh", &[]);
    fx.drive();
    let run = fx.run(&id);
    assert_eq!(run.status, "passed", "{}", fx.summary(&id));
    let say = &fx.steps(&id)[0];
    let stdout = std::fs::read_to_string(&say.outputs()["stdout"]).unwrap();
    assert!(
        stdout.contains(&format!("port={}", run.port.unwrap())),
        "{stdout}"
    );
}

#[test]
fn a_server_that_exits_fails_the_run_and_says_so() {
    let fx = Fx::new();
    if !fx.has_bun() {
        return;
    }
    fx.write(
        "dying",
        &format!(
            r#"id: dying
name: The server dies
version: 1
setup:
  - id: server
    action: start
    {server}
  - id: ready
    action: wait_for
    port: "{{{{ run.port }}}}"
    timeout: 20s
steps:
  - id: never
    action: run
    {never}
"#,
            server = bun("console.error('cannot bind'); process.exit(2)"),
            never = bun("console.log('never')"),
        ),
    );
    let id = fx.queue("dying", &[]);
    fx.drive();
    let run = fx.run(&id);
    assert_eq!(run.status, "failed");
    assert_eq!(
        run.error.as_deref(),
        Some("`server` exited with 2 before the run ended.")
    );
    let steps = fx.steps(&id);
    let server = fx.step(&steps, "server");
    assert_eq!(server.status, "failed");
    let note = server.note.as_deref().unwrap();
    assert!(
        note.contains("Exited with 2 before the run ended."),
        "{note}"
    );
    assert!(note.contains("cannot bind"), "{note}");
    let ready = fx.step(&steps, "ready");
    assert_eq!(ready.status, "failed");
    assert_eq!(
        ready.note.as_deref(),
        Some("`server` exited with 2 while this ran.")
    );
    assert_eq!(fx.step(&steps, "never").status, "skipped");
    let summary = fx.summary(&id);
    assert!(
        summary.contains("last lines of `server` (setup step):"),
        "{summary}"
    );
    assert!(summary.contains("      cannot bind"), "{summary}");
}

#[test]
fn a_manual_step_waits_for_the_answer_from_the_app() {
    let fx = Fx::new();
    fx.write(
        "look",
        r#"id: look
name: Have a look
version: 1
steps:
  - id: look
    action: manual
    prompt: "Does {{ workspace.name }} look right?"
"#,
    );
    let id = fx.queue("look", &[]);
    assert!(fx.tick());
    fx.until(&id, |steps| steps[0].status == "waiting");
    let waiting = &fx.steps(&id)[0];
    assert_eq!(
        waiting.outputs().get("prompt").map(String::as_str),
        Some("Does fix look right?")
    );
    assert!(fx
        .store
        .answer_test_step(&id, "look", false, Some("the chart flashed"))
        .unwrap());
    fx.driver.join();
    let run = fx.run(&id);
    assert_eq!(run.status, "failed");
    let look = &fx.steps(&id)[0];
    assert_eq!(look.status, "failed");
    assert_eq!(look.outputs()["note"], "the chart flashed");
    let summary = fx.summary(&id);
    assert!(
        summary.contains("  look  manual  failed after "),
        "{summary}"
    );
    assert!(summary.contains("    the chart flashed"), "{summary}");
}

#[test]
fn the_runs_time_limit_stops_a_step_and_fails_the_run() {
    let fx = Fx::new();
    if !fx.has_bun() {
        return;
    }
    fx.write(
        "long",
        &format!(
            r#"id: long
name: Too long
version: 1
timeout: 1s
steps:
  - id: wait
    action: run
    {wait}
  - id: after
    action: run
    {after}
"#,
            wait = bun("setTimeout(() => {}, 60000)"),
            after = bun("console.log('never')"),
        ),
    );
    let id = fx.queue("long", &[]);
    let started = Instant::now();
    fx.drive();
    assert!(started.elapsed() < Duration::from_secs(10));
    let run = fx.run(&id);
    assert_eq!(run.status, "failed");
    assert_eq!(run.error.as_deref(), Some("Timed out after 1s."));
    let steps = fx.steps(&id);
    assert_eq!(fx.step(&steps, "wait").status, "failed");
    assert_eq!(
        fx.step(&steps, "wait").note.as_deref(),
        Some("The run's time limit passed while this ran.")
    );
    assert_eq!(fx.step(&steps, "after").status, "skipped");
}

#[test]
fn a_run_the_app_stopped_during_is_ended_and_its_server_killed_at_the_next_start() {
    let fx = Fx::new();
    // A server from a previous app: started here, recorded the way the runner records one.
    let mut command = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    if cfg!(windows) {
        command.args(["/C", "ping -n 60 127.0.0.1 >NUL"]);
    } else {
        command.args(["-c", "sleep 60"]);
    }
    command.stdin(std::process::Stdio::null());
    let mut group = crate::process::Group::spawn(&mut command).unwrap();
    let pid = group.id();
    let print = group.fingerprint();

    fx.write(
        "ghost",
        r#"id: ghost
name: Left running
version: 1
setup:
  - id: server
    action: start
    program: sleep
    args: ["60"]
steps:
  - id: ping
    action: wait_for
    port: "{{ run.port }}"
"#,
    );
    let id = fx.queue("ghost", &[]);
    let out = runs_dir(&fx.data_dir).join(&id);
    std::fs::create_dir_all(&out).unwrap();
    assert!(fx
        .store
        .start_test_run(&id, 1, &out.to_string_lossy())
        .unwrap());
    let mut outputs = BTreeMap::from([("pid".to_owned(), pid.to_string())]);
    if let Some(print) = &print {
        outputs.insert("process".to_owned(), print.clone());
    }
    let outputs = serde_json::to_string(&outputs).unwrap();
    for (step, to, outputs) in [
        ("server", "passed", Some(outputs.as_str())),
        ("ping", "running", None),
    ] {
        assert!(fx
            .store
            .move_test_step(
                &id,
                step,
                &["pending"],
                &StepChange {
                    to,
                    exit: None,
                    outputs,
                    note: None,
                    screenshots: None,
                },
            )
            .unwrap());
    }
    // An output folder of a run that is not in the database any more.
    let stray = runs_dir(&fx.data_dir).join("not-a-run");
    std::fs::create_dir_all(&stray).unwrap();

    assert!(fx.tick(), "the recovery is a change");
    if print.is_some() {
        // The previous app would be gone, and init would reap what it left; here this test is
        // the parent, so it reaps. A process that was not killed is still running here.
        let asked = Instant::now();
        while matches!(group.try_wait(), Ok(None)) && asked.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            matches!(group.try_wait(), Ok(Some(_))),
            "the server was stopped"
        );
    } else {
        // Windows: the job object would have ended it with the app's handle, which this test
        // still holds. Nothing to find again, so nothing is looked for.
        group.kill_tree();
    }
    let run = fx.run(&id);
    assert_eq!(run.status, "errored");
    assert_eq!(run.error.as_deref(), Some(INTERRUPTED));
    let steps = fx.steps(&id);
    assert_eq!(fx.step(&steps, "ping").status, "failed");
    assert_eq!(fx.step(&steps, "ping").note.as_deref(), Some(INTERRUPTED));
    if let Some(print) = print {
        assert_ne!(
            crate::process::fingerprint(pid).as_deref(),
            Some(print.as_str()),
            "the server left behind was found by its start time and stopped"
        );
    }
    assert!(!stray.exists(), "a folder with no run was removed");
    assert!(out.exists(), "a folder with a run was kept");
    assert!(!fx.tick(), "nothing more to do");
}

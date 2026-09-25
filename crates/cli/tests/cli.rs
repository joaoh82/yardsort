//! `ys` end to end: the real binary, a real database, real git repositories.
//!
//! No agent is started here. Launching one needs a harness on `PATH`, which a CI runner has no
//! reason to have — what is tested instead is that everything up to the launch is right, and
//! that asking for a harness that is not there fails *before* anything is created.

use std::path::PathBuf;
use std::process::Command;

use yardsort_core::env::{EnvSource, ShellEnv};
use yardsort_core::git::testing::git;
use yardsort_core::git::{normalize, Git};
use yardsort_core::projects::Projects;
use yardsort_core::settings::WorkspaceSettings;
use yardsort_core::store::Store;
use yardsort_core::workspaces::Workspaces;

/// A throwaway profile with one project in it, and the repository that project points at.
struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
    data_dir: PathBuf,
    worktree_root: PathBuf,
    repo: PathBuf,
    project_name: String,
}

impl Fixture {
    fn new() -> Self {
        let (data, code, worktrees) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let store = Store::open(&data.path().join("yardsort.db")).unwrap();
        let git = git();
        let added = Projects {
            store: &store,
            git: &git,
        }
        .create("Demo", &normalize(code.path()))
        .unwrap();
        Self {
            data_dir: data.path().to_path_buf(),
            worktree_root: normalize(worktrees.path()),
            repo: PathBuf::from(&added.project.root_path),
            project_name: added.project.name.clone(),
            _dirs: (data, code, worktrees),
        }
    }

    /// Run `ys` against this profile. Worktrees go somewhere disposable, and no daemon is ever
    /// wanted here — `YARDSORT_NO_DAEMON` keeps a stray process from being started.
    fn ys(&self, args: &[&str]) -> Run {
        self.ys_from(None, args)
    }

    /// `cwd` is where a harness would be standing: inside the workspace it is about to delete.
    fn ys_from(&self, cwd: Option<&std::path::Path>, args: &[&str]) -> Run {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ys"));
        command
            .args(["--data-dir", &self.data_dir.to_string_lossy()])
            .args(args)
            .env("YARDSORT_WORKTREE_ROOT", &self.worktree_root)
            .env("YARDSORT_NO_DAEMON", "1");
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command.output().expect("ys should run");
        Run {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn git(&self) -> Git {
        git()
    }
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn ok(self) -> String {
        assert_eq!(
            self.code,
            Some(0),
            "expected success.\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        self.stdout
    }

    fn failed(self) -> String {
        assert_eq!(
            self.code,
            Some(1),
            "expected failure.\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        self.stderr
    }
}

#[test]
fn projects_are_listed_as_a_table_and_as_json() {
    let fx = Fixture::new();

    let table = fx.ys(&["project", "list"]).ok();
    assert!(table.contains("Demo"), "{table}");
    assert!(
        table.contains(&fx.repo.to_string_lossy().to_string()),
        "the path should be shown: {table}"
    );

    let json = fx.ys(&["project", "list", "--json"]).ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed[0]["name"], "Demo");
    assert_eq!(parsed[0]["workspaces"], 0);
}

#[test]
fn a_new_workspace_is_a_real_branch_and_a_real_worktree() {
    let fx = Fixture::new();

    let out = fx
        .ys(&[
            "workspace",
            "new",
            &fx.project_name,
            "fix the login bug",
            "--no-agent",
        ])
        .ok();
    assert!(out.contains("Nothing started"), "{out}");

    // The database knows about it, alongside the `local` workspace every project has.
    let json = fx.ys(&["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let made = worktrees_in(&listed);
    assert_eq!(made.len(), 1, "expected exactly one worktree: {listed}");
    let branch = made[0]["branch"].as_str().unwrap().to_owned();
    let path = PathBuf::from(made[0]["path"].as_str().unwrap());
    assert!(branch.starts_with("ys/"), "branch was {branch}");

    // …and so does git, which is the part that matters: the app and `ys` agree because they are
    // both just looking at the repository.
    assert!(
        path.join(".git").exists(),
        "{} has no worktree",
        path.display()
    );
    let worktrees = fx.git().worktrees(&fx.repo).unwrap();
    assert!(
        worktrees.iter().any(|w| w.path == normalize(&path)),
        "git does not know about {}: {worktrees:?}",
        path.display()
    );
    assert!(fx.git().branch_exists(&fx.repo, &branch).unwrap());
}

#[test]
fn deleting_removes_the_folder_and_keeps_the_branch() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "fix the login bug");
    let name = made["name"].as_str().unwrap();
    let branch = made["branch"].as_str().unwrap().to_owned();
    let path = PathBuf::from(made["path"].as_str().unwrap());

    let out = fx.ys(&["workspace", "delete", name]).ok();
    assert!(out.contains(&format!("deleted  {name}")), "{out}");
    assert!(out.contains("kept"), "{out}");
    assert!(!path.exists(), "the folder should be gone");
    assert!(
        fx.git().branch_exists(&fx.repo, &branch).unwrap(),
        "the branch is kept"
    );
    assert!(
        fx.git()
            .worktrees(&fx.repo)
            .unwrap()
            .iter()
            .all(|w| w.is_main),
        "git should forget the worktree"
    );

    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    assert!(
        worktrees_in(&listed).is_empty(),
        "the record should be gone: {listed}"
    );
}

/// A harness finishes by deleting the workspace it was started in, so the command has to work
/// with that folder as its current directory, and it has to return rather than taking the
/// process down with it.
#[test]
fn deleting_works_from_inside_the_workspace() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "tidy up");
    let name = made["name"].as_str().unwrap().to_owned();
    let branch = made["branch"].as_str().unwrap().to_owned();
    let path = PathBuf::from(made["path"].as_str().unwrap());

    let json = fx
        .ys_from(Some(&path), &["workspace", "delete", &name, "--json"])
        .ok();
    let deleted: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(deleted["name"], name);
    assert_eq!(deleted["branch"], branch);
    assert_eq!(deleted["path"], path.display().to_string());
    assert!(!path.exists());
    assert!(fx.git().branch_exists(&fx.repo, &branch).unwrap());
}

/// Another program still has the folder as its current directory. `ys` itself is started from
/// outside, which is the case the in-process step-out does not cover. Windows will not remove
/// that folder, and the refusal has to come before git deletes anything inside it.
#[cfg(windows)]
#[test]
fn a_folder_held_by_another_process_is_not_deleted() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "held open");
    let name = made["name"].as_str().unwrap().to_owned();
    let id = made["id"].as_str().unwrap().to_owned();
    let branch = made["branch"].as_str().unwrap().to_owned();
    let path = PathBuf::from(made["path"].as_str().unwrap());
    std::fs::write(path.join("notes.txt"), "still here").unwrap();

    // `ping` simply stays alive. `timeout` refuses to run without a console.
    let holder = StopProcess(
        Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .current_dir(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("a process should be able to sit in the workspace"),
    );
    // CreateProcess has already set the current directory; give it a moment to be the cwd.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let error = fx.ys(&["workspace", "delete", &name]).failed();
    assert!(
        error.contains("Nothing was deleted"),
        "the refusal should say nothing was removed: {error}"
    );
    assert!(
        error.contains("current directory"),
        "the refusal should say why, and how to retry: {error}"
    );

    assert!(path.is_dir(), "the folder should still be there");
    assert_eq!(
        std::fs::read_to_string(path.join("notes.txt")).unwrap(),
        "still here",
        "files inside the folder should be untouched"
    );
    assert!(
        fx.git().branch_exists(&fx.repo, &branch).unwrap(),
        "the branch is kept"
    );
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    assert!(
        worktrees_in(&listed)
            .iter()
            .any(|w| w["id"].as_str() == Some(id.as_str())),
        "the record should still be there: {listed}"
    );
    drop(holder);
}

/// Kills the child however the test ends, including a failed assertion.
#[cfg(windows)]
struct StopProcess(std::process::Child);

#[cfg(windows)]
impl Drop for StopProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The refusal is the confirmation. `--force` is what "yes, destroy that work" looks like from a
/// script, and leaving it off must leave the files, the folder and the record exactly as they were.
#[test]
fn a_dirty_workspace_is_kept_until_force_is_given() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "wip");
    let name = made["name"].as_str().unwrap().to_owned();
    let branch = made["branch"].as_str().unwrap().to_owned();
    let path = PathBuf::from(made["path"].as_str().unwrap());
    std::fs::write(path.join("draft.txt"), "not committed").unwrap();

    let error = fx.ys(&["workspace", "delete", &name]).failed();
    assert!(error.contains("--force"), "{error}");
    assert!(error.contains(&name), "{error}");
    assert!(
        path.join("draft.txt").exists(),
        "declining to force must not delete the work"
    );
    assert!(fx.git().branch_exists(&fx.repo, &branch).unwrap());
    let still: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    assert_eq!(worktrees_in(&still).len(), 1, "{still}");

    let forced = fx
        .ys(&["workspace", "delete", &name, "--force", "--json"])
        .ok();
    let deleted: serde_json::Value = serde_json::from_str(&forced).unwrap();
    assert_eq!(deleted["id"], made["id"]);
    assert_eq!(deleted["branch"], branch);
    assert!(
        !path.exists(),
        "force removes the folder and the uncommitted file"
    );
    assert!(
        fx.git().branch_exists(&fx.repo, &branch).unwrap(),
        "force still keeps the branch"
    );
}

#[test]
fn the_local_workspace_cannot_be_deleted() {
    let fx = Fixture::new();
    let error = fx.ys(&["workspace", "delete", "local", "--force"]).failed();
    assert!(error.to_ascii_lowercase().contains("local"), "{error}");

    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["kind"] == "local"),
        "local must still be there: {listed}"
    );
}

#[test]
fn an_unknown_workspace_says_which_ones_there_are() {
    let fx = Fixture::new();
    make_workspace(&fx, "fix the login bug");
    let error = fx.ys(&["workspace", "delete", "nope"]).failed();
    assert!(error.contains("nope"), "{error}");
    assert!(
        error.contains("fix-login-bug") || error.contains("Demo"),
        "it should name what exists: {error}"
    );
}

/// Two projects can each have a workspace of the same name. Deleting must not pick one.
#[test]
fn a_shared_name_is_refused_rather_than_guessed() {
    let fx = Fixture::new();
    let first = make_workspace(&fx, "fix the login bug");
    let name = first["name"].as_str().unwrap().to_owned();

    let other = tempfile::tempdir().unwrap();
    let git = fx.git();
    git.init(other.path()).unwrap();
    git.initial_commit(other.path()).unwrap();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let added = Projects {
        store: &store,
        git: &git,
    }
    .open(other.path(), false)
    .unwrap();
    let settings = WorkspaceSettings::default();
    let second = Workspaces {
        env: &ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        },
        store: &store,
        git: &git,
        worktree_root: &fx.worktree_root,
        settings: &settings,
    }
    .create(&added.project.id, None, "fix the login bug")
    .unwrap();
    assert_eq!(second.name, name, "the two workspaces share a name");

    let error = fx.ys(&["workspace", "delete", &name]).failed();
    assert!(error.contains(first["id"].as_str().unwrap()), "{error}");
    assert!(error.contains(&second.id), "{error}");

    let first_path = PathBuf::from(first["path"].as_str().unwrap());
    assert!(
        first_path.exists(),
        "neither workspace should have been deleted"
    );
    assert!(PathBuf::from(&second.path).exists());
}

#[test]
fn a_harness_that_is_not_there_is_refused_before_anything_is_created() {
    let fx = Fixture::new();

    let error = fx
        .ys(&[
            "workspace",
            "new",
            &fx.project_name,
            "some task",
            "--harness",
            "no-such-harness",
        ])
        .failed();
    assert!(error.contains("no-such-harness"), "{error}");

    // Nothing half-created: no row, no branch, no folder. The project's own `local` workspace
    // is always there and is not one of ours.
    let json = fx.ys(&["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        worktrees_in(&listed).is_empty(),
        "a refused launch left a workspace behind: {listed}"
    );
    assert!(
        fx.git()
            .worktrees(&fx.repo)
            .unwrap()
            .iter()
            .all(|w| w.is_main),
        "a refused launch left a worktree behind"
    );
}

#[test]
fn an_unknown_project_says_which_ones_there_are() {
    let fx = Fixture::new();
    let error = fx
        .ys(&["workspace", "new", "Nope", "task", "--no-agent"])
        .failed();
    assert!(error.contains("Nope"), "{error}");
    assert!(
        error.contains("Demo"),
        "it should name the projects: {error}"
    );
}

/// The app reads this line to tell whether the `ys` on `PATH` is its own, and which version it is
/// (`src-tauri/src/ys.rs`). Change the format there too.
#[test]
fn the_version_is_printed_the_way_the_app_reads_it() {
    let output = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("ys {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_profile_with_no_database_is_an_error_naming_the_path() {
    let empty = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args(["--data-dir", &empty.path().to_string_lossy()])
        .args(["project", "list"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(&empty.path().join("yardsort.db").display().to_string()),
        "the error should name the database it looked for: {error}"
    );
    // It must not have made one: that is what would make a wrong path look like an empty app.
    assert!(!empty.path().join("yardsort.db").exists());
}

#[test]
fn doctor_reports_a_profile_without_touching_it() {
    let fx = Fixture::new();
    let json = fx.ys(&["doctor", "--json"]).ok();
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(report["databaseFound"], true);
    assert_eq!(report["projects"], 1);
    assert_eq!(
        report["dataDir"],
        fx.data_dir.display().to_string(),
        "doctor should report the profile it was pointed at"
    );
}

/// Create one worktree workspace and return its `workspace list --json` entry.
fn make_workspace(fx: &Fixture, prompt: &str) -> serde_json::Value {
    fx.ys(&["workspace", "new", &fx.project_name, prompt, "--no-agent"])
        .ok();
    let json = fx.ys(&["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let made = worktrees_in(&listed);
    assert_eq!(made.len(), 1, "expected one worktree: {listed}");
    made[0].clone()
}

/// The workspaces Yardsort made, which is everything but each project's own checkout.
fn worktrees_in(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("a list")
        .iter()
        .filter(|w| w["kind"] == "worktree")
        .collect()
}

/// `attach` takes over the terminal, so the interesting part cannot run under `cargo test` —
/// there is no tty. What is testable is everything it refuses to do before touching one.
#[test]
fn attach_refuses_rather_than_writing_escape_sequences_into_a_pipe() {
    let fx = Fixture::new();
    // Captured output is a pipe, not a terminal, which is exactly the case this guards.
    let error = fx.ys(&["attach"]).failed();
    assert!(
        error.contains("needs a terminal"),
        "should refuse a pipe before attaching: {error}"
    );
}

#[test]
fn attach_says_json_makes_no_sense_before_it_says_anything_else() {
    let fx = Fixture::new();
    let error = fx.ys(&["attach", "--json"]).failed();
    assert!(error.contains("--json"), "{error}");
}

/// Screens are held by the daemon, not the database, so this is the answer whenever there is no
/// daemon — and it has to say so, or it reads as "that session printed nothing".
#[test]
fn logs_says_where_screens_live_when_nothing_is_holding_them() {
    let fx = Fixture::new();
    let error = fx.ys(&["logs"]).failed();
    assert!(
        error.contains("daemon") && error.contains("their screens are gone"),
        "should explain that screens live in the daemon: {error}"
    );
}

/// What `ys activity` shows is what the core recorded — here, a run written the way a launch
/// writes one — and `export` is the same rows as NDJSON.
#[test]
fn activity_is_listed_as_a_table_as_json_and_exported_as_ndjson() {
    use yardsort_core::activity::{Continuation, LaunchedBy, Recorder, RunDraft, RunKind};
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    let recorder = Recorder::new(&store, &Default::default(), LaunchedBy::Cli);
    let draft = RunDraft {
        workspace_id: workspace.id.clone(),
        session_id: None,
        kind: RunKind::Harness,
        harness_id: Some("claude".into()),
        harness_session_id: None,
        model: Some("opus".into()),
        effort: None,
        program: "claude".into(),
        continuation: Continuation::Fresh,
    };
    let run = recorder.begin(&draft).unwrap();
    recorder.spawned(&run, &draft, "pty-1", None);
    yardsort_core::activity::record_exit(
        &store,
        "pty-1",
        &yardsort_core::activity::ExitFacts {
            code: Some(4),
            success: false,
            signal: None,
            at: None,
        },
        yardsort_core::activity::Via::Live,
    );
    drop(store);

    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("process.started"), "{table}");
    assert!(table.contains("exit 4, via live"), "{table}");
    assert!(table.contains("yardsort/lifecycle"), "{table}");
    assert!(table.contains("local"), "the workspace's name: {table}");

    let json = fx.ys(&["activity", "list", "--json", "--limit", "1"]).ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed.as_array().unwrap().len(), 1);
    assert_eq!(parsed[0]["kind"], "process.exited", "newest first");
    assert_eq!(parsed[0]["payload"]["exitCode"], 4);
    assert_eq!(parsed[0]["producer"], "yardsort");

    let ndjson = fx.ys(&["activity", "export", "--workspace", "local"]).ok();
    let lines: Vec<serde_json::Value> = ndjson
        .lines()
        .map(|line| serde_json::from_str(line).expect("one object per line"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["kind"], "process.started", "oldest first");
    assert_eq!(lines[0]["payload"]["model"], "opus");
    assert_eq!(lines[1]["runId"], run);

    let missing = fx.ys(&["activity", "list", "--workspace", "nope"]).failed();
    assert!(missing.contains("No workspace called"), "{missing}");
}

/// An agent that ended while nothing was connected left its exit in the daemon's spool. Any
/// `ys` command that would ask the daemon drains it first, so the record stops saying `running`
/// and says how the process really ended.
#[test]
fn a_spooled_exit_is_taken_into_the_records_before_sessions_are_listed() {
    use yardsort_core::activity::{Continuation, LaunchedBy, Recorder, RunDraft, RunKind};
    use yardsort_core::store::NewSession;
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    let recorder = Recorder::new(&store, &Default::default(), LaunchedBy::App);
    let draft = RunDraft {
        workspace_id: workspace.id.clone(),
        session_id: None,
        kind: RunKind::Harness,
        harness_id: Some("claude".into()),
        harness_session_id: Some("h-1".into()),
        model: None,
        effort: None,
        program: "claude".into(),
        continuation: Continuation::Fresh,
    };
    let run = recorder.begin(&draft).unwrap();
    recorder.spawned(&run, &draft, "pty-gone", None);
    store
        .add_session(&NewSession {
            id: "rec-1",
            workspace_id: &workspace.id,
            harness_id: "claude",
            harness_session_id: Some("h-1"),
            title: "fix it",
            pty_session_id: "pty-gone",
            ..Default::default()
        })
        .unwrap();
    recorder.link_session(&run, "rec-1");
    drop(store);

    let spool = pty_ipc::spool::Spool::new(yardsort_core::activity::spool_dir(&fx.data_dir));
    spool
        .record(&pty_ipc::spool::SpoolEntry {
            version: pty_ipc::spool::SPOOL_VERSION,
            at_ms: 1,
            daemon_pid: 1,
            session: pty_host::SessionId("pty-gone".into()),
            labels: Default::default(),
            exit: pty_host::ExitInfo {
                code: 0,
                success: true,
                signal: None,
            },
        })
        .unwrap();

    let json = fx.ys(&["session", "list", "--json", "--all"]).ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed[0]["id"], "rec-1");
    assert_eq!(parsed[0]["running"], false, "the spooled exit settled it");
    assert!(spool.entries().unwrap().is_empty(), "drained");

    let doctor = fx.ys(&["doctor", "--json"]).ok();
    let report: serde_json::Value = serde_json::from_str(&doctor).unwrap();
    assert_eq!(report["activityEvents"], 2);
    assert_eq!(report["activityRuns"], 1);
    assert_eq!(report["spoolPending"], 0);
}

/// `ys --yardsort-hook claude <inbox>` is what a Claude Code launched from here runs at each
/// step: the real binary, a recorded payload on stdin, the launcher's ids in the environment.
/// The next `ys` command drains what it left and the event is on the timeline, placed.
#[test]
fn the_binary_is_a_hook_that_leaves_an_entry_the_next_command_takes_in() {
    use std::io::Write;
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    drop(store);
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/claude-hooks")
        .join(yardsort_core::activity::claude::FIXTURE_VERSION)
        .join("04-PostToolUse.json");

    let mut hook = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("claude")
        .arg(&inbox)
        .env(yardsort_core::activity::WORKSPACE_ENV, &workspace.id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(&fixture).unwrap())
        .unwrap();
    let output = hook.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "a hook says nothing to the agent");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 1);

    // Garbage on stdin is still exit 0 — the agent must never be blocked — and no file.
    let mut bad = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("claude")
        .arg(&inbox)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    bad.stdin.take().unwrap().write_all(b"not json").unwrap();
    let output = bad.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not JSON"));
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 1);

    let json = fx.ys(&["activity", "list", "--json"]).ok();
    let events: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(events[0]["kind"], "tool.completed");
    assert_eq!(events[0]["producer"], "claude");
    assert_eq!(events[0]["method"], "hook");
    assert_eq!(events[0]["workspaceId"], workspace.id);
    let payload = &events[0]["payload"];
    assert_eq!(payload["tool"], "Write");
    assert_eq!(payload["path"], "hello.txt");
    assert!(payload.get("content").is_none(), "metadata only");
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 0, "drained");

    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("claude/hook"), "{table}");
    assert!(table.contains("Write hello.txt, 3 ms"), "{table}");

    let doctor = fx.ys(&["doctor", "--json"]).ok();
    let report: serde_json::Value = serde_json::from_str(&doctor).unwrap();
    assert_eq!(report["inboxPending"], 0);
    assert_eq!(report["inboxDir"].as_str(), Some(&*inbox.to_string_lossy()));
}

/// `ys --yardsort-hook codex <inbox> <payload>` is what Codex's `notify` runs at the end of a
/// turn: the payload as the last argument, the launcher's ids in the environment. The next `ys`
/// command reads the turn out of Codex's session file — here, the fixture, placed where Codex
/// keeps them under a throwaway `CODEX_HOME` — and the commands, the file and the tokens are on
/// the timeline.
#[test]
fn the_binary_as_codex_notify_leaves_a_trigger_the_next_command_expands_from_the_session_file() {
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    drop(store);
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/codex")
        .join(yardsort_core::activity::codex::FIXTURE_VERSION);
    let codex_home = fx.data_dir.join("codex-home");
    let day = codex_home.join("sessions/2026/09/25");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::copy(
        fixtures.join("rollout.jsonl"),
        day.join("rollout-2026-09-25T13-07-32-01a0d83f-c3ea-7ae0-88df-82c9431d3f8b.jsonl"),
    )
    .unwrap();
    let notify = std::fs::read_to_string(fixtures.join("notify/01.json")).unwrap();
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("codex")
        .arg(&inbox)
        .arg(&notify)
        .env(yardsort_core::activity::WORKSPACE_ENV, &workspace.id)
        .env("CODEX_HOME", &codex_home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 1);

    let json = fx.ys(&["activity", "list", "--json", "--limit", "20"]).ok();
    let events: serde_json::Value = serde_json::from_str(&json).unwrap();
    let kinds: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"file.reported_write"), "{kinds:?}");
    assert!(kinds.contains(&"usage.reported"), "{kinds:?}");
    assert!(kinds.contains(&"tool.failed"), "{kinds:?}");
    let turn = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "turn.completed")
        .unwrap();
    assert_eq!(turn["producer"], "codex");
    assert_eq!(turn["method"], "session_file");
    assert_eq!(turn["workspaceId"], workspace.id);
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 0, "drained");

    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("codex/session_file"), "{table}");
    assert!(table.contains("add hello.txt"), "{table}");
    assert!(table.contains("29842 tokens"), "{table}");
    assert!(!table.contains("cat hello.txt"), "no command text: {table}");
}

/// `ys --yardsort-hook opencode <inbox>` with a plugin delivery on stdin is what OpenCode's
/// plugin runs at each step. The next `ys` command drains it and the event is on the timeline.
#[test]
fn the_binary_as_the_opencode_plugins_hook_leaves_an_entry_the_next_command_takes_in() {
    use std::io::Write;
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    drop(store);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/opencode")
        .join(yardsort_core::activity::opencode::FIXTURE_VERSION)
        .join("18-tool.execute.after-write.json");
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);

    let mut hook = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("opencode")
        .arg(&inbox)
        .env(yardsort_core::activity::WORKSPACE_ENV, &workspace.id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(&fixture).unwrap())
        .unwrap();
    let output = hook.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = fx.ys(&["activity", "list", "--json"]).ok();
    let events: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(events[0]["kind"], "tool.completed");
    assert_eq!(events[0]["producer"], "opencode");
    assert_eq!(events[0]["method"], "plugin");
    assert_eq!(events[0]["payload"]["tool"], "write");
    assert_eq!(events[0]["payload"]["path"], "hello.txt");
    assert_eq!(events[0]["payload"]["created"], true);
    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("opencode/plugin"), "{table}");
    assert!(table.contains("write hello.txt"), "{table}");
}

/// Grok writes its own session directory; when reading it is switched on, any `ys` command
/// that drains takes what is there for Yardsort's own Grok runs — found by the id Yardsort
/// chose — and the tools, turns and tokens are on the timeline.
#[test]
fn groks_session_directory_is_read_for_its_runs_when_switched_on() {
    use yardsort_core::activity::{Continuation, LaunchedBy, Recorder, RunDraft, RunKind};
    let fx = Fixture::new();
    std::fs::write(
        fx.data_dir.join("settings.toml"),
        "[activity]\ncapture_grok = true\n",
    )
    .unwrap();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    let session = "11111111-1111-4111-8111-111111111111";
    let recorder = Recorder::new(&store, &Default::default(), LaunchedBy::Cli);
    let draft = RunDraft {
        workspace_id: workspace.id.clone(),
        session_id: None,
        kind: RunKind::Harness,
        harness_id: Some("grok".into()),
        harness_session_id: Some(session.into()),
        model: None,
        effort: None,
        program: "grok".into(),
        continuation: Continuation::Fresh,
    };
    let run = recorder.begin(&draft).unwrap();
    recorder.spawned(&run, &draft, "pty-grok", Some("session_file"));
    drop(store);
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/grok")
        .join(yardsort_core::activity::grok::FIXTURE_VERSION);
    let grok_home = fx.data_dir.join("grok-home");
    let dir = grok_home.join("sessions/%2Fsomewhere").join(session);
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["events.jsonl", "usage.json", "summary.json"] {
        std::fs::copy(fixtures.join(file), dir.join(file)).unwrap();
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_ys"));
    command
        .args(["--data-dir", &fx.data_dir.to_string_lossy()])
        .args(["activity", "list", "--json", "--limit", "20"])
        .env("YARDSORT_NO_DAEMON", "1")
        .env("GROK_HOME", &grok_home);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let kinds: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"tool.failed"), "{kinds:?}");
    assert!(kinds.contains(&"usage.reported"), "{kinds:?}");
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    let usage = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "usage.reported")
        .unwrap();
    assert_eq!(usage["producer"], "grok");
    assert_eq!(usage["method"], "session_file");
    assert_eq!(usage["runId"], run);
    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("grok/session_file"), "{table}");
    assert!(table.contains("57107 tokens"), "{table}");
}

/// `ys --yardsort-hook omp <inbox>` with an extension delivery on stdin is what OMP's — and
/// pi's — extension runs at each step. The next `ys` command drains it.
#[test]
fn the_binary_as_the_pi_familys_extension_hook_leaves_an_entry_the_next_command_takes_in() {
    use std::io::Write;
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    drop(store);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/omp")
        .join(yardsort_core::activity::pi::OMP_FIXTURE_VERSION)
        .join("15-turn_end.json");
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);
    let mut hook = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("omp")
        .arg(&inbox)
        .env(yardsort_core::activity::WORKSPACE_ENV, &workspace.id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(&fixture).unwrap())
        .unwrap();
    let output = hook.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = fx.ys(&["activity", "list", "--json"]).ok();
    let events: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(events[0]["kind"], "turn.completed");
    assert_eq!(events[0]["producer"], "omp");
    assert_eq!(events[0]["method"], "extension");
    assert_eq!(events[0]["payload"]["totalTokens"], 19963);
    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("omp/extension"), "{table}");
    assert!(table.contains("19963 tokens"), "{table}");
}

/// `ys --yardsort-hook cursor <inbox>` with a hook payload on stdin is what the plugin's
/// `hooks.json` runs at each step of a Cursor agent. The shape is the documented one: the
/// Cursor agent has not been recorded yet (see `scripts/record-cursor.sh`).
#[test]
fn the_binary_as_a_cursor_hook_leaves_an_entry_the_next_command_takes_in() {
    use std::io::Write;
    let fx = Fixture::new();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspaces().unwrap().remove(0);
    drop(store);
    let payload = serde_json::json!({
        "conversation_id": "conv-1",
        "generation_id": "gen-1",
        "model": "composer-2",
        "hook_event_name": "afterFileEdit",
        "cursor_version": yardsort_core::activity::cursor::DOCUMENTED_VERSION,
        "workspace_roots": [workspace.path],
        "transcript_path": "/home/user/.cursor/projects/x/agent-transcripts/conv-1.jsonl",
        "file_path": format!("{}/src/lib.rs", workspace.path),
        "edits": [{ "old_string": "fn a()", "new_string": "fn b()" }],
    });
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);
    let mut hook = Command::new(env!("CARGO_BIN_EXE_ys"))
        .arg(yardsort_core::activity::hook::HOOK_FLAG)
        .arg("cursor")
        .arg(&inbox)
        .env(yardsort_core::activity::WORKSPACE_ENV, &workspace.id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = hook.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = fx.ys(&["activity", "list", "--json"]).ok();
    let events: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(events[0]["kind"], "file.reported_write");
    assert_eq!(events[0]["producer"], "cursor");
    assert_eq!(events[0]["method"], "hook");
    assert_eq!(events[0]["payload"]["path"], "src/lib.rs");
    assert_eq!(events[0]["payload"]["edits"], 1);
    assert!(
        !json.contains("fn a()"),
        "the edit's text stays out: {json}"
    );
    let table = fx.ys(&["activity", "list"]).ok();
    assert!(table.contains("cursor/hook"), "{table}");
}

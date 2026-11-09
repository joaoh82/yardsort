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
        // Run from inside an agent Yardsort started, `ys` would find that agent's workspace in
        // its environment before the folder it stands in; these tests mean the folder.
        for name in [
            yardsort_core::activity::RUN_ENV,
            yardsort_core::activity::WORKSPACE_ENV,
            yardsort_core::activity::RECORD_ENV,
        ] {
            command.env_remove(name);
        }
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
fn a_reader_that_stops_reading_ends_it_quietly() {
    let fx = Fixture::new();
    // As `ys project list | head -1` leaves it once `head` has its line: nobody at the other end.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args([
            "--data-dir",
            &fx.data_dir.to_string_lossy(),
            "project",
            "list",
        ])
        .env("YARDSORT_NO_DAEMON", "1")
        .stdout(writer)
        .output()
        .expect("ys should run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "");
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
fn the_handoff_packet_is_printed_from_the_record_and_git_together() {
    use yardsort_core::activity::{Continuation, LaunchedBy, Recorder, RunDraft, RunKind};
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Fix the login redirect");
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store
        .workspaces()
        .unwrap()
        .into_iter()
        .find(|w| w.id == made["id"].as_str().unwrap())
        .unwrap();
    // A file the user changed, and a recorded Claude run that reported writing another.
    std::fs::write(
        std::path::Path::new(&workspace.path).join("notes.md"),
        "hello\n",
    )
    .unwrap();
    // --no-agent recorded no conversation, so the task is written the way the app writes it.
    store
        .add_session(&yardsort_core::store::NewSession {
            id: "rec-1",
            workspace_id: &workspace.id,
            harness_id: "claude",
            model: Some("opus"),
            title: "Fix the login redirect",
            pty_session_id: "pty-1",
            prompt: Some("Fix the login redirect"),
            ..Default::default()
        })
        .unwrap();
    let recorder = Recorder::new(&store, &Default::default(), LaunchedBy::Cli);
    let draft = RunDraft {
        workspace_id: workspace.id.clone(),
        session_id: Some("rec-1".into()),
        kind: RunKind::Harness,
        harness_id: Some("claude".into()),
        harness_session_id: None,
        model: Some("opus".into()),
        effort: None,
        program: "claude".into(),
        continuation: Continuation::Fresh,
    };
    let run = recorder.begin(&draft).unwrap();
    recorder.spawned(&run, &draft, "pty-1", Some("hook"));
    store
        .add_event(&yardsort_core::store::NewEvent {
            id: "e-write",
            schema_version: 1,
            workspace_id: &workspace.id,
            session_id: None,
            run_id: Some(&run),
            occurred_at: 1_790_000_000_000,
            kind: "tool.completed",
            producer: "claude",
            method: "hook",
            fidelity: "reported",
            source_key: None,
            privacy_class: "metadata",
            payload: r#"{"tool":"Write","toolUseId":"t","path":"src/login.rs"}"#,
        })
        .unwrap();
    drop(store);

    let name = made["name"].as_str().unwrap();
    let text = fx.ys(&["workspace", "handoff", name]).ok();
    assert!(
        text.starts_with(&format!("# Handoff from Yardsort: workspace \"{name}\"")),
        "{text}"
    );
    assert!(
        text.contains("> Fix the login redirect"),
        "the task, whole: {text}"
    );
    assert!(
        text.contains("`notes.md` (untracked) — no agent reported writing it"),
        "{text}"
    );
    assert!(text.contains("- **claude** (opus), started"), "{text}");
    assert!(text.contains("**still running**"), "{text}");
    assert!(
        text.contains("  - Reported writing: `src/login.rs`.\n"),
        "{text}"
    );
    assert!(text.contains("## What is not here"), "{text}");
    assert!(!text.contains("Assist"), "no ranking from the command line");

    let json = fx.ys(&["workspace", "handoff", name, "--json"]).ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["runs"], 1);
    assert!(parsed["text"]
        .as_str()
        .unwrap()
        .contains("## Where the work stands"));

    let missing = fx.ys(&["workspace", "handoff", "nope"]).failed();
    assert!(
        missing.contains("No workspace called \"nope\""),
        "{missing}"
    );
}

/// Forking makes a new workspace on a new branch from the workspace's branch: its commits come
/// along, its uncommitted files do not, and the agent's message is the user's, then the packet.
#[test]
fn forking_branches_from_the_workspace_and_sends_its_record_after_the_message() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Fix the login redirect");
    let name = made["name"].as_str().unwrap().to_owned();
    let branch = made["branch"].as_str().unwrap().to_owned();
    let source = PathBuf::from(made["path"].as_str().unwrap());
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .arg("-C")
            .arg(&source)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    std::fs::write(source.join("login.rs"), "fn login() {}\n").unwrap();
    git(&["add", "login.rs"]);
    git(&["commit", "-q", "-m", "Redirect after sign-in"]);
    std::fs::write(source.join("draft.md"), "not yet\n").unwrap();

    // --print makes nothing: it is the message, the user's words first.
    let printed = fx
        .ys(&["workspace", "fork", &name, "add CSV export", "--print"])
        .ok();
    assert!(
        printed.starts_with(&format!(
            "add CSV export\n\n# Forked from Yardsort: workspace \"{name}\""
        )),
        "{printed}"
    );
    assert!(
        printed.contains("  - Redirect after sign-in\n"),
        "{printed}"
    );
    assert!(
        printed.contains("- Uncommitted in that workspace, and so **not** in yours: 1 file"),
        "{printed}"
    );
    let json: serde_json::Value = serde_json::from_str(
        &fx.ys(&[
            "workspace",
            "fork",
            &name,
            "add CSV export",
            "--print",
            "--json",
        ])
        .ok(),
    )
    .unwrap();
    assert_eq!(json["branch"], branch.as_str());
    assert_eq!(json["uncommitted"], 1);
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    assert_eq!(worktrees_in(&listed).len(), 1, "--print made nothing");

    let out = fx
        .ys(&["workspace", "fork", &name, "add CSV export", "--no-agent"])
        .ok();
    assert!(out.contains(&format!("  from   {name}")), "{out}");
    assert!(
        out.contains(&format!("1 file uncommitted in {name} stayed there")),
        "{out}"
    );
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workspace", "list", "--json"]).ok()).unwrap();
    let forked = worktrees_in(&listed)
        .into_iter()
        .find(|w| w["name"] != name.as_str())
        .expect("the fork")
        .clone();
    let path = PathBuf::from(forked["path"].as_str().unwrap());
    assert_ne!(forked["branch"], branch.as_str(), "a branch of its own");
    assert!(path.join("login.rs").exists(), "the commit came along");
    assert!(
        !path.join("draft.md").exists(),
        "the uncommitted file did not"
    );
    assert!(
        source.join("draft.md").exists(),
        "and is still where it was"
    );

    git(&["checkout", "-q", "--detach"]);
    let detached = fx
        .ys(&["workspace", "fork", &name, "add CSV export", "--no-agent"])
        .failed();
    assert!(detached.contains("is not on a branch"), "{detached}");
    let missing = fx.ys(&["workspace", "fork", "nope", "x"]).failed();
    assert!(
        missing.contains("No workspace called \"nope\""),
        "{missing}"
    );
}

/// An agent asks what the workspace's other conversation said: nothing while the project has
/// turned sharing off; otherwise the user's messages and the replies, from the recorded
/// transcript, without the tool output and without the asking agent's own conversation.
#[test]
fn an_agent_reads_the_other_conversations_only_while_the_project_shares_them() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Fix the login redirect");
    let workspace_id = made["id"].as_str().unwrap().to_owned();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspace(&workspace_id).unwrap().unwrap();
    for (record, native) in [("rec-other", "conv-other"), ("rec-asking", "conv-asking")] {
        store
            .add_session(&yardsort_core::store::NewSession {
                id: record,
                workspace_id: &workspace_id,
                harness_id: "claude",
                harness_session_id: Some(native),
                title: record,
                pty_session_id: &format!("pty-{record}"),
                ..Default::default()
            })
            .unwrap();
    }
    drop(store);
    // Claude Code's config directory, holding both conversations' transcripts.
    let claude = tempfile::tempdir().unwrap();
    let folder = claude.path().join("projects").join("-wt");
    std::fs::create_dir_all(&folder).unwrap();
    let recorded = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/claude-transcript/2.1.302");
    std::fs::copy(
        recorded.join("conversation.jsonl"),
        folder.join("conv-other.jsonl"),
    )
    .unwrap();
    std::fs::copy(
        recorded.join("fork.jsonl"),
        folder.join("conv-asking.jsonl"),
    )
    .unwrap();
    let nowhere = tempfile::tempdir().unwrap();

    let as_agent = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_ys"))
            .args(["--data-dir", &fx.data_dir.to_string_lossy()])
            .args(args)
            .env_remove("YARDSORT_RUN_ID")
            .env("YARDSORT_WORKSPACE_ID", &workspace_id)
            .env("YARDSORT_SESSION_RECORD_ID", "rec-asking")
            .env("CLAUDE_CONFIG_DIR", claude.path())
            .env("CODEX_HOME", nowhere.path())
            .env("YARDSORT_NO_DAEMON", "1")
            .current_dir(&workspace.path)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // On by default; turned off, nothing is read.
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    store
        .set_conversations_shared(&workspace.project_id, false)
        .unwrap();
    drop(store);
    let (code, _, refused) = as_agent(&["workspace", "context"]);
    assert_eq!(code, Some(1));
    assert!(
        refused.contains("does not share its agents' conversations"),
        "{refused}"
    );
    // The handoff packet keeps to the same switch.
    let (code, packet, err) = as_agent(&["workspace", "handoff", &workspace_id]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!packet.contains("## What was said"), "{packet}");
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    store
        .set_conversations_shared(&workspace.project_id, true)
        .unwrap();
    drop(store);

    // Now it carries every conversation here, under its own part.
    let (code, packet, err) = as_agent(&["workspace", "handoff", &workspace_id]);
    assert_eq!(code, Some(0), "{err}");
    let said = &packet[packet.find("## What was said").expect(&packet)..];
    assert!(
        said.contains("### claude · conversation rec-othe"),
        "{packet}"
    );
    assert!(
        said.contains("> Thanks. In one sentence, what is left to do?"),
        "{packet}"
    );
    assert!(said.find("## What is not here").is_some(), "{packet}");

    let (code, text, err) = as_agent(&["workspace", "context"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        text.starts_with("# What was said in this workspace"),
        "{text}"
    );
    assert!(
        text.contains("## claude · conversation rec-othe · 2 turns"),
        "{text}"
    );
    assert!(
        text.contains("> Thanks. In one sentence, what is left to do?"),
        "{text}"
    );
    assert!(text.contains("> Nothing is left"), "{text}");
    assert!(!text.contains("File created successfully"), "{text}");
    // Its own conversation — the fork's last question — is not read back to it.
    assert!(!text.contains("say which file you created"), "{text}");

    let (_, json, _) = as_agent(&["workspace", "context", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["conversations"][0]["turnCount"], 2);
    assert_eq!(json["conversations"][0]["turns"][0]["n"], 2);

    let (code, whole, err) = as_agent(&["workspace", "context", "--turn", "1"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(whole.contains("turn 1 of 2"), "{whole}");
    assert!(whole.matches("\nAgent:\n").count() >= 2, "{whole}");

    let (code, _, err) = as_agent(&["workspace", "context", "--turn", "9"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("there is no turn 9"), "{err}");
    let (code, _, err) = as_agent(&["workspace", "context", "--since", "yesterday"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("--since takes a time in UTC"), "{err}");
    let (_, later, _) = as_agent(&["workspace", "context", "--since", "2099-01-01T00:00Z"]);
    assert!(!later.contains("> Thanks."), "{later}");
}

#[test]
fn memory_is_proposed_by_agents_approved_only_in_the_app_and_read_by_anyone() {
    use yardsort_core::store::NewRun;
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Fix the login redirect");
    let workspace_id = made["id"].as_str().unwrap().to_owned();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = store.workspace(&workspace_id).unwrap().unwrap();
    store
        .add_run(&NewRun {
            id: "run-claude",
            workspace_id: &workspace_id,
            session_id: None,
            kind: "harness",
            harness_id: Some("claude"),
            harness_session_id: None,
            launched_by: "app",
        })
        .unwrap();
    drop(store);

    // As the agent Yardsort launched: its run id in the environment, standing in its worktree.
    // The test's own environment is cleared of launch variables, so where it runs cannot leak in.
    let as_agent = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_ys"))
            .args(["--data-dir", &fx.data_dir.to_string_lossy()])
            .args(args)
            .env_remove("YARDSORT_WORKSPACE_ID")
            .env("YARDSORT_RUN_ID", "run-claude")
            .env("YARDSORT_NO_DAEMON", "1")
            .current_dir(&workspace.path)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, said, _) = as_agent(&["memory", "propose", "The tests need TZ=UTC."]);
    assert_eq!(code, Some(0));
    assert!(said.contains("waits for the user to approve it"), "{said}");
    let (_, again, _) = as_agent(&["memory", "propose", "the tests need tz=utc."]);
    assert!(again.contains("Already there"), "{again}");

    // Nothing is read while the project does not share its memory with its agents.
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    store
        .set_memory_shared(&workspace.project_id, false)
        .unwrap();
    drop(store);
    let (code, _, refused) = as_agent(&["memory", "list"]);
    assert_eq!(code, Some(1));
    assert!(
        refused.contains("does not give its agents its memory"),
        "{refused}"
    );
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    store
        .set_memory_shared(&workspace.project_id, true)
        .unwrap();
    drop(store);
    // And a candidate is nobody's to read until the user approves it.
    let (_, listed, _) = as_agent(&["memory", "list"]);
    assert!(listed.contains("No approved entries."), "{listed}");
    // And there is nothing an agent could run to approve it.
    for verb in ["approve", "edit", "reject", "revoke"] {
        let (code, _, err) = as_agent(&["memory", verb, "x"]);
        assert_ne!(code, Some(0), "`ys memory {verb}` must not exist: {err}");
    }

    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let entry = store
        .memory_entries(&workspace.project_id)
        .unwrap()
        .remove(0);
    assert_eq!(
        (
            entry.state.as_str(),
            entry.author.as_str(),
            entry.harness_id.as_deref(),
            entry.run_id.as_deref()
        ),
        ("candidate", "agent", Some("claude"), Some("run-claude"))
    );
    yardsort_core::memory::decide(&store, &entry.id, yardsort_core::memory::Decision::Approve)
        .unwrap();
    drop(store);

    let (_, found, _) = as_agent(&["memory", "search", "tz"]);
    assert!(found.contains("The tests need TZ=UTC."), "{found}");
    assert!(
        found.contains(&format!("claude in {}", made["name"].as_str().unwrap())),
        "{found}"
    );
    let json = fx
        .ys(&["memory", "list", "--project", &fx.project_name, "--json"])
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["text"], "The tests need TZ=UTC.");
    let nowhere = fx.ys(&["memory", "list"]).failed();
    assert!(
        nowhere.contains("Not inside a Yardsort workspace"),
        "{nowhere}"
    );
}

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

/// The hook again, with what Claude Code sent after it opened a pull request in a nested clone:
/// the next `ys` command takes it in, and the workspace follows that pull request — tracked by
/// `hook`, with only the link kept on the event.
#[test]
fn a_pull_request_claude_opened_elsewhere_is_followed_without_anyone_saying_so() {
    use std::io::Write;
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Bump the library");
    let workspace_id = made["id"].as_str().unwrap().to_owned();
    fx.git()
        .run(
            &fx.repo,
            &["remote", "add", "origin", "git@github.com:o/parent.git"],
        )
        .unwrap();
    let inbox = yardsort_core::activity::inbox_dir(&fx.data_dir);
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/fixtures/claude-hooks-pull-requests/2.1.319");
    for name in ["01-PreToolUse.json", "02-PostToolUse.json"] {
        let mut hook = Command::new(env!("CARGO_BIN_EXE_ys"))
            .arg(yardsort_core::activity::hook::HOOK_FLAG)
            .arg("claude")
            .arg(&inbox)
            .env(yardsort_core::activity::WORKSPACE_ENV, &workspace_id)
            .env_remove(yardsort_core::activity::RUN_ENV)
            .env_remove(yardsort_core::activity::RECORD_ENV)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        hook.stdin
            .take()
            .unwrap()
            .write_all(&std::fs::read(fixtures.join(name)).unwrap())
            .unwrap();
        let output = hook.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    let listed: serde_json::Value = serde_json::from_str(
        &fx.ys(&["pr", "tracked", "--workspace", &workspace_id, "--json"])
            .ok(),
    )
    .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["url"], "https://github.com/example/lib/pull/41");
    assert_eq!(listed[0]["trackedBy"], "hook");

    let events: serde_json::Value =
        serde_json::from_str(&fx.ys(&["activity", "list", "--json"]).ok()).unwrap();
    let done = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "tool.completed")
        .unwrap();
    assert_eq!(
        done["payload"]["pullRequestUrl"],
        "https://github.com/example/lib/pull/41"
    );
    assert!(
        !done["payload"].to_string().contains("Bump the client"),
        "{}",
        done["payload"]
    );
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

/// A workflow file with one mistake: a variable naming a field the pull request does not have.
const WORKFLOW_WITH_A_TYPO: &str = "id: fix-ci
name: Fix CI
version: 1
trigger:
  kind: manual
steps:
  - id: fix
    action: start_session
    harness: claude
    prompt: CI failed on {{ pr.nubmer }}.
";

fn write_workflow(fx: &Fixture, name: &str, text: &str) -> PathBuf {
    let folder = fx.data_dir.join("workflows");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn workflows_are_listed_with_the_built_ins_and_where_yours_go() {
    let fx = Fixture::new();
    let table = fx.ys(&["workflow", "list"]).ok();
    assert!(table.contains("code-review"), "{table}");
    assert!(table.contains("ready"), "{table}");
    assert!(table.contains("built in"), "{table}");
    let folder = fx.data_dir.join("workflows").display().to_string();
    assert!(
        table.contains(&folder),
        "the folder should be named: {table}"
    );

    write_workflow(&fx, "fix-ci.yaml", WORKFLOW_WITH_A_TYPO);
    let json = fx.ys(&["workflow", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let fix = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "fix-ci")
        .expect("the user's file is listed");
    assert_eq!(fix["runnable"], false);
    assert_eq!(fix["problems"][0]["line"], 10);
    assert_eq!(fix["source"]["kind"], "file");
    assert_eq!(fix["source"]["replacesBuiltIn"], false);
    let table = fx.ys(&["workflow", "list"]).ok();
    assert!(table.contains("1 problem"), "{table}");
}

#[test]
fn a_file_with_a_built_ins_id_is_listed_in_its_place() {
    let fx = Fixture::new();
    let mine = WORKFLOW_WITH_A_TYPO
        .replace("id: fix-ci", "id: code-review")
        .replace("pr.nubmer", "pr.number");
    write_workflow(&fx, "code-review.yaml", &mine);
    let json = fx.ys(&["workflow", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let reviews: Vec<&serde_json::Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["id"] == "code-review")
        .collect();
    assert_eq!(reviews.len(), 1, "{json}");
    assert_eq!(reviews[0]["source"]["replacesBuiltIn"], true);
    let shown = fx.ys(&["workflow", "show", "code-review"]).ok();
    assert_eq!(shown, mine, "show prints the file that is used, exactly");
}

#[test]
fn show_prints_the_file_and_its_problems_apart() {
    let fx = Fixture::new();
    let shown = fx.ys(&["workflow", "show", "code-review"]).ok();
    assert!(shown.starts_with("# Built into Yardsort."), "{shown}");

    let path = write_workflow(&fx, "fix-ci.yaml", WORKFLOW_WITH_A_TYPO);
    let run = fx.ys(&["workflow", "show", "fix-ci"]);
    assert_eq!(run.stdout, WORKFLOW_WITH_A_TYPO, "stdout is the file alone");
    let expected = format!("{}:10:26: `{{{{ pr.nubmer }}}}`", path.display());
    assert!(run.stderr.contains(&expected), "{}", run.stderr);
    assert_eq!(run.code, Some(0), "showing a broken file is not a failure");

    let json = fx.ys(&["workflow", "show", "fix-ci", "--json"]).ok();
    let entry: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(entry["text"], WORKFLOW_WITH_A_TYPO);
    assert!(entry["workflow"].is_null());
}

#[test]
fn an_unknown_workflow_says_which_ones_there_are() {
    let fx = Fixture::new();
    let error = fx.ys(&["workflow", "show", "nope"]).failed();
    assert!(error.contains("There is no workflow `nope`"), "{error}");
    assert!(error.contains("code-review"), "{error}");
}

#[test]
fn validate_names_each_problem_by_file_line_and_column_and_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fix-ci.yaml");
    std::fs::write(&path, WORKFLOW_WITH_A_TYPO).unwrap();
    // No profile at all: checking a file must not need one.
    let nowhere = dir.path().join("no-profile");
    let validate = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ys"))
            .args(["--data-dir", &nowhere.to_string_lossy()])
            .args(args)
            .output()
            .unwrap()
    };
    let output = validate(&["workflow", "validate", &path.to_string_lossy()]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    let expected = format!(
        "{}:10:26: `{{{{ pr.nubmer }}}}`: `pr` has no `nubmer`.",
        path.display()
    );
    assert!(error.starts_with(&expected), "{error}");

    let output = validate(&["--json", "workflow", "validate", &path.to_string_lossy()]);
    assert_eq!(output.status.code(), Some(1));
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(checked["valid"], false);
    assert_eq!(checked["id"], "fix-ci");
    assert_eq!(checked["problems"][0]["column"], 26);

    std::fs::write(&path, WORKFLOW_WITH_A_TYPO.replace("nubmer", "number")).unwrap();
    let output = validate(&["workflow", "validate", &path.to_string_lossy()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("a valid workflow, `fix-ci`"));
    assert!(!nowhere.exists(), "validating made no profile");
}

#[test]
fn validate_reads_standard_input_for_a_dash() {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args(["workflow", "validate", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(WORKFLOW_WITH_A_TYPO.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.starts_with("<stdin>:10:26:"), "{error}");
}

/// The procedure the guide gives for changing a built-in, end to end in a fresh profile.
#[test]
fn copying_the_built_in_review_gives_a_file_that_is_used_instead() {
    let fx = Fixture::new();
    let copied = fx.ys(&["workflow", "copy", "code-review"]).ok();
    let path = fx.data_dir.join("workflows").join("code-review.yaml");
    assert!(copied.contains(&path.display().to_string()), "{copied}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("id: code-review"), "{text}");

    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workflow", "list", "--json"]).ok()).unwrap();
    let review = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "code-review")
        .unwrap();
    assert_eq!(review["source"]["replacesBuiltIn"], true);
    assert_eq!(review["runnable"], true, "{review}");
    fx.ys(&["workflow", "validate", &path.to_string_lossy()])
        .ok();
    assert_eq!(fx.ys(&["workflow", "show", "code-review"]).ok(), text);

    // Again: refused, and the copy is untouched.
    std::fs::write(&path, format!("{text}# mine\n")).unwrap();
    let error = fx.ys(&["workflow", "copy", "code-review"]).failed();
    assert!(error.contains("already your file"), "{error}");
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .ends_with("# mine\n"));

    let json = fx
        .ys(&[
            "workflow",
            "copy",
            "code-review",
            "--as",
            "quick-review",
            "--json",
        ])
        .ok();
    let copied: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(copied["id"], "quick-review");
    fx.ys(&["workflow", "show", "quick-review"]).ok();
}

/// Works with the workspace's pull request; needs no agent.
const PR_NOTE: &str = "id: pr-note
name: PR note
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: \"#{{ pr.number }}\"
";

/// Uses the workspace's task; needs no agent.
const TASK_NOTE: &str = "id: task-note
name: Task note
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: \"{{ workspace.task }}\"
";

/// A workflow that needs no agent, so these tests need none on PATH either.
const PING: &str = "id: ping
name: Ping
version: 1
trigger:
  kind: manual
inputs:
  - id: note
    kind: text
steps:
  - id: tell
    action: notify
    title: \"{{ inputs.note }}\"
";

#[test]
fn a_run_is_refused_while_yardsort_is_closed_and_nothing_is_queued() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let error = fx
        .ys(&["workflow", "run", "ping", "--workspace", name])
        .failed();
    assert!(error.contains("Yardsort is not running"), "{error}");
    assert!(error.contains("Nothing was queued"), "{error}");
    let listed = fx.ys(&["workflow", "runs", "--json"]).ok();
    assert_eq!(listed.trim(), "[]");
}

/// On a computer with no screen, `yardsort-host` holds the profile, and nothing would carry a run
/// out: refused, rather than queued for nobody.
#[test]
fn nothing_is_queued_while_a_headless_host_holds_the_profile() {
    use yardsort_core::presence::{AppLock, Holder};
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let _host = AppLock::take_as(&fx.data_dir, Holder::Host)
        .unwrap()
        .expect("nobody else holds it");
    let error = fx
        .ys(&["workflow", "run", "ping", "--workspace", name])
        .failed();
    assert!(
        error.contains("This host does not run workflows"),
        "{error}"
    );
    assert!(error.contains("Nothing was queued"), "{error}");
    assert_eq!(fx.ys(&["workflow", "runs", "--json"]).ok().trim(), "[]");

    let added = fx
        .ys(&[
            "trigger",
            "add",
            "--on",
            "workspace.done",
            "--workflow",
            "ping",
            "--workspace",
            name,
        ])
        .ok();
    assert!(added.contains("does not fire triggers yet"), "{added}");
    assert!(!added.contains("Yardsort is not running"), "{added}");
}

#[test]
fn a_project_is_added_by_its_folder_and_a_folder_inside_adds_the_repository() {
    let fx = Fixture::new();
    let code = tempfile::tempdir().unwrap();
    let repo = normalize(code.path()).join("widget");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let git = fx.git();
    git.init(&repo).unwrap();
    git.initial_commit(&repo).unwrap();

    let added: serde_json::Value = serde_json::from_str(
        &fx.ys(&[
            "project",
            "add",
            &repo.join("src").to_string_lossy(),
            "--json",
        ])
        .ok(),
    )
    .unwrap();
    assert_eq!(added["name"], "widget");
    assert_eq!(added["path"], repo.to_string_lossy().as_ref());
    assert_eq!(added["already_known"], false);
    let listed = fx.ys(&["project", "list"]).ok();
    assert!(listed.contains("widget"), "{listed}");

    let again = fx.ys(&["project", "add", &repo.to_string_lossy()]).ok();
    assert!(again.contains("already a project"), "{again}");

    let plain = tempfile::tempdir().unwrap();
    let refused = fx
        .ys(&["project", "add", &plain.path().to_string_lossy()])
        .failed();
    assert!(refused.contains("not a git repository"), "{refused}");
}

#[test]
fn a_run_is_queued_followed_and_cancelled_while_the_app_holds_its_lock() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    // This test stands in for the running app. Nothing drives the run, so it stays queued.
    let _app = yardsort_core::presence::AppLock::take(&fx.data_dir)
        .unwrap()
        .expect("nobody else holds it");

    let json = fx
        .ys(&[
            "workflow",
            "run",
            "ping",
            "--workspace",
            name,
            "--input",
            "note=Hi there",
            "--json",
        ])
        .ok();
    let queued: serde_json::Value = serde_json::from_str(&json).unwrap();
    let run = queued["run"].as_str().unwrap().to_owned();
    let short = &run[..8];

    let again = fx
        .ys(&["workflow", "run", "ping", "--workspace", name])
        .failed();
    assert!(again.contains("already running"), "{again}");
    assert!(again.contains(short), "{again}");

    let table = fx.ys(&["workflow", "runs"]).ok();
    assert!(table.contains(short) && table.contains("queued"), "{table}");
    let shown: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workflow", "runs", "--run", short, "--json"]).ok()).unwrap();
    assert_eq!(shown["status"], "queued");
    assert_eq!(shown["inputs"]["note"], "Hi there");
    assert_eq!(shown["steps"][0]["id"], "tell");
    assert_eq!(shown["steps"][0]["status"], "pending");

    let cancelled = fx.ys(&["workflow", "cancel", short]).ok();
    assert!(
        cancelled.contains("Agents it started keep running"),
        "{cancelled}"
    );
    let steps = fx.ys(&["workflow", "runs", "--run", short]).ok();
    assert!(steps.contains("cancelled"), "{steps}");
    assert!(steps.contains("Cancelled from ys."), "{steps}");
    let twice = fx.ys(&["workflow", "cancel", short]).failed();
    assert!(twice.contains("already ended"), "{twice}");
}

#[test]
fn a_run_inside_a_workspace_needs_no_name_and_bad_inputs_are_refused() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let path = PathBuf::from(made["path"].as_str().unwrap());
    let _app = yardsort_core::presence::AppLock::take(&fx.data_dir)
        .unwrap()
        .unwrap();

    let unknown = fx
        .ys_from(
            Some(&path),
            &["workflow", "run", "ping", "--input", "colour=red"],
        )
        .failed();
    assert!(unknown.contains("has no input `colour`"), "{unknown}");
    let no_equals = fx
        .ys_from(Some(&path), &["workflow", "run", "ping", "--input", "note"])
        .failed();
    assert!(no_equals.contains("needs an `=`"), "{no_equals}");
    // A workflow that works with the workspace's pull request, and starts no agent, so what
    // is installed on this machine does not matter. This repository has no GitHub remote, so
    // there is no pull request to be found, gh or no gh, and nothing is queued.
    write_workflow(&fx, "pr-note.yaml", PR_NOTE);
    let refused = fx
        .ys_from(Some(&path), &["workflow", "run", "pr-note"])
        .failed();
    assert!(refused.contains("pull request"), "{refused}");
    assert!(
        !refused.contains("--json"),
        "not gh's command line: {refused}"
    );
    let listed = fx.ys(&["workflow", "runs", "--json"]).ok();
    assert_eq!(listed.trim(), "[]", "the refusal queued nothing");

    let queued = fx.ys_from(Some(&path), &["workflow", "run", "ping"]).ok();
    assert!(queued.contains(made["name"].as_str().unwrap()), "{queued}");
}

#[test]
fn a_run_says_what_it_will_find_nothing_for_before_it_goes() {
    let fx = Fixture::new();
    write_workflow(&fx, "task-note.yaml", TASK_NOTE);
    // Made with --no-agent: no first message, so no task.
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let _app = yardsort_core::presence::AppLock::take(&fx.data_dir)
        .unwrap()
        .unwrap();
    let said = fx
        .ys(&["workflow", "run", "task-note", "--workspace", name])
        .ok();
    assert!(said.contains("Queued `task-note`"), "{said}");
    assert!(
        said.contains("Note: `{{ workspace.task }}` will be empty"),
        "{said}"
    );
    let json = fx.ys(&["workflow", "runs", "--json"]).ok();
    assert!(json.contains("task-note"), "queued all the same: {json}");
}

/// A profile whose project is on GitHub as far as its remote says, and a `gh` that answers
/// from the recorded fixtures and writes down what it was asked. `PATH` is that `gh` and git,
/// and nothing else: the machine's own `gh`, if it has one, is never run.
#[cfg(unix)]
struct Tasks {
    fx: Fixture,
    bin: tempfile::TempDir,
}

#[cfg(unix)]
impl Tasks {
    fn new(remote: Option<&str>, gh: Option<&str>) -> Self {
        let fx = Fixture::new();
        if let Some(remote) = remote {
            fx.git()
                .run(&fx.repo, &["remote", "add", "origin", remote])
                .unwrap();
        }
        let bin = tempfile::tempdir().unwrap();
        let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("git"))
            .find(|path| path.is_file())
            .expect("git on PATH");
        std::os::unix::fs::symlink(git, bin.path().join("git")).unwrap();
        if let Some(body) = gh {
            let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/fixtures/gh/2.102.0");
            let script = format!(
                // `PATH` has nothing but git and this, so `cat` is spelled out.
                "#!/bin/sh\nhere='{0}'\necho \"$*\" >> \"$here/asked\"\nfixtures='{1}'\n\
                 cat() {{ while IFS= read -r line || [ -n \"$line\" ]; do \
                 printf '%s\\n' \"$line\"; done < \"$1\"; }}\n{body}\n",
                bin.path().display(),
                fixtures.display()
            );
            let path = bin.path().join("gh");
            yardsort_core::program::testing::executable(&path, script);
        }
        Self { fx, bin }
    }

    /// On GitHub, with a `gh` that has the recorded issues.
    fn recorded() -> Self {
        Self::new(
            Some("git@github.com:example/widgets.git"),
            Some(
                "case \"$*\" in\n\
                 *'issue(number:'*) cat \"$fixtures/issue-view.json\" ;;\n\
                 *states:CLOSED*) cat \"$fixtures/issues-closed.json\" ;;\n\
                 *after=*) cat \"$fixtures/issues-page-2.json\" ;;\n\
                 *) cat \"$fixtures/issues-page-1.json\" ;;\nesac",
            ),
        )
    }

    fn ys(&self, cwd: Option<&std::path::Path>, args: &[&str]) -> Run {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ys"));
        command
            .args(["--data-dir", &self.fx.data_dir.to_string_lossy()])
            .args(args)
            .env("YARDSORT_WORKTREE_ROOT", &self.fx.worktree_root)
            .env("YARDSORT_NO_DAEMON", "1")
            .env("PATH", self.bin.path())
            // No login shell to ask, so the environment is this one: see `ShellEnv::resolve`.
            .env("SHELL", self.bin.path().join("no-such-shell"));
        for name in [
            yardsort_core::activity::RUN_ENV,
            yardsort_core::activity::WORKSPACE_ENV,
            yardsort_core::activity::RECORD_ENV,
        ] {
            command.env_remove(name);
        }
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

    fn asked(&self) -> Vec<String> {
        std::fs::read_to_string(self.bin.path().join("asked"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(unix)]
fn keys(json: &str) -> Vec<String> {
    let parsed: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
    parsed
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["key"].as_str().unwrap().to_owned())
        .collect()
}

#[cfg(unix)]
#[test]
fn tasks_are_listed_as_a_table_and_as_json_from_every_page() {
    let tasks = Tasks::recorded();
    let table = tasks.ys(None, &["task", "list", "--project", "Demo"]).ok();
    let lines: Vec<&str> = table.lines().collect();
    assert!(lines[0].starts_with("KEY"), "{table}");
    assert_eq!(lines.len(), 10, "a heading and nine tasks: {table}");
    assert!(lines[1].starts_with("#14386"), "{table}");
    assert!(lines[1].contains("open, needs answer"), "{table}");
    assert!(lines[1].contains("2026-10-03"), "{table}");
    assert!(table.contains("Replaced title of issue 7428"), "{table}");
    assert!(table.contains("ghost"), "an account that is gone: {table}");
    let asked = tasks.asked();
    assert_eq!(asked.len(), 2, "two pages: {asked:?}");
    assert!(asked[1].contains("after="), "{asked:?}");

    let json = tasks
        .ys(None, &["task", "list", "--project", "Demo", "--json"])
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed.as_array().unwrap().len(), 9);
    let first = &parsed[0];
    assert_eq!(first["source"], "github");
    assert_eq!(first["key"], "#14386");
    assert_eq!(first["state"], "open");
    assert_eq!(first["needsAnswer"], true);
    assert_eq!(first["comments"], 8);
    assert_eq!(first["author"], "grace");
    assert_eq!(
        first["url"],
        "https://github.com/example/widgets/issues/14386"
    );
    assert!(first["labels"][0]["name"].is_string());
}

#[cfg(unix)]
#[test]
fn tasks_are_narrowed_by_what_is_asked_for() {
    let tasks = Tasks::recorded();
    let list = |args: &[&str]| {
        let mut all = vec!["task", "list", "--project", "Demo", "--json"];
        all.extend(args);
        keys(&tasks.ys(None, &all).ok())
    };
    let waiting = list(&["--needs-answer"]);
    assert!(waiting.contains(&"#14584".to_owned()), "{waiting:?}");
    assert!(
        !waiting.contains(&"#14563".to_owned()),
        "a member's own issue with only bots on it: {waiting:?}"
    );
    assert!(waiting.len() < 9);

    assert_eq!(list(&["--assignee", "donald"]), ["#13876"]);
    assert_eq!(list(&["--author", "EDSGER"]), ["#13876"]);
    assert_eq!(list(&["--search", "#1387"]), ["#13876"]);
    assert_eq!(list(&["--search", "issue 7428"]), ["#7428"]);
    assert_eq!(list(&["--limit", "2"]), ["#14386", "#14584"]);
    assert!(list(&["--label", "no-such-label"]).is_empty());
    // `@me` is whoever gh says is logged in — "ada", who opened and is assigned none of them.
    assert!(list(&["--author", "@me"]).is_empty());

    let nothing = tasks
        .ys(
            None,
            &["task", "list", "--project", "Demo", "--label", "no-such"],
        )
        .ok();
    assert_eq!(nothing.trim(), "No open tasks match.");
}

#[cfg(unix)]
#[test]
fn closed_tasks_are_asked_for_only_when_wanted() {
    let tasks = Tasks::recorded();
    let closed = tasks
        .ys(
            None,
            &["task", "list", "--project", "Demo", "--state", "closed"],
        )
        .ok();
    assert!(closed.contains("closed (not planned)"), "{closed}");
    assert!(closed.contains("closed (duplicate)"), "{closed}");
    let asked = tasks.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains("states:CLOSED"), "{asked:?}");

    let all = tasks
        .ys(
            None,
            &[
                "task",
                "list",
                "--project",
                "Demo",
                "--state",
                "all",
                "--json",
            ],
        )
        .ok();
    let all = keys(&all);
    assert_eq!(all.len(), 12, "nine open and three closed: {all:?}");
    assert!(all.contains(&"#14495".to_owned()));
}

#[cfg(unix)]
#[test]
fn a_task_in_both_the_open_and_the_closed_answer_is_listed_once() {
    // The closed question is answered with two issues the open one already had: what GitHub
    // says of an issue reopened between the two.
    let tasks = Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some(
            "case \"$*\" in\n\
             *states:CLOSED*|*after=*) cat \"$fixtures/issues-page-2.json\" ;;\n\
             *) cat \"$fixtures/issues-page-1.json\" ;;\nesac",
        ),
    );
    let all = tasks
        .ys(
            None,
            &[
                "task",
                "list",
                "--project",
                "Demo",
                "--state",
                "all",
                "--json",
            ],
        )
        .ok();
    let mut all = keys(&all);
    assert_eq!(all.len(), 9, "{all:?}");
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 9, "no key twice");
}

#[cfg(unix)]
#[test]
fn a_link_to_another_repositorys_issue_is_refused_by_name() {
    let tasks = Tasks::recorded();
    let refused = tasks
        .ys(
            None,
            &[
                "task",
                "show",
                "https://github.com/someone/else/issues/14394",
                "--project",
                "Demo",
            ],
        )
        .failed();
    assert!(refused.contains("github.com/someone/else"), "{refused}");
    assert!(refused.contains("github.com/example/widgets"), "{refused}");
}

#[cfg(unix)]
#[test]
fn the_project_is_the_one_the_command_is_run_in() {
    let tasks = Tasks::recorded();
    let here = tasks
        .ys(Some(&tasks.fx.repo), &["task", "list", "--json"])
        .ok();
    assert_eq!(keys(&here).len(), 9);

    let elsewhere = tasks.ys(Some(tasks.bin.path()), &["task", "list"]).failed();
    assert!(elsewhere.contains("--project"), "{elsewhere}");
    assert!(tasks.asked().len() == 2, "nothing was asked for the second");
}

#[cfg(unix)]
#[test]
fn a_task_is_shown_in_full_by_number_key_or_link() {
    let tasks = Tasks::recorded();
    for wanted in [
        "14394",
        "#14394",
        "https://github.com/example/widgets/issues/14394",
    ] {
        let shown = tasks
            .ys(None, &["task", "show", wanted, "--project", "Demo"])
            .ok();
        assert!(
            shown.starts_with("#14394 Replaced title of issue 14394\n"),
            "{shown}"
        );
        assert!(shown.contains("open, needs answer · opened by"), "{shown}");
        assert!(shown.contains("labels: bug, priority-3"), "{shown}");
        assert!(shown.contains("Replaced description."), "{shown}");
        assert!(shown.contains("5 comments:"), "{shown}");
        assert!(shown.contains("triage-bot (bot), "), "{shown}");
        assert!(shown.contains("linus (maintainer), "), "{shown}");
        assert!(shown.contains("Replaced comment 5."), "{shown}");
    }
    assert!(tasks.asked().iter().all(|a| a.ends_with("-F number=14394")));

    let json = tasks
        .ys(
            None,
            &["task", "show", "14394", "--project", "Demo", "--json"],
        )
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["task"]["key"], "#14394");
    assert_eq!(parsed["comments"].as_array().unwrap().len(), 5);
    assert_eq!(parsed["comments"][0]["bot"], true);
    assert!(parsed["body"].as_str().unwrap().contains("Replaced"));

    let refused = tasks
        .ys(None, &["task", "show", "soon", "--project", "Demo"])
        .failed();
    assert!(refused.contains("not an issue's number"), "{refused}");
}

#[cfg(unix)]
#[test]
fn tasks_that_cannot_be_read_say_why() {
    let logged_out = "echo 'To get started with GitHub CLI, please run: gh auth login' >&2\nexit 4";
    let cases: [(Option<&str>, Option<&str>, &str); 6] = [
        (None, Some("exit 1"), "no remote on a forge"),
        (
            Some("git@gitlab.com:example/widgets.git"),
            Some("exit 1"),
            "is on GitLab",
        ),
        (
            Some("git@github.com:example/widgets.git"),
            None,
            "`gh` is not installed",
        ),
        (
            Some("git@github.com:example/widgets.git"),
            Some(logged_out),
            "gh auth login",
        ),
        (
            Some("git@github.com:example/widgets.git"),
            Some("cat \"$fixtures/issues-disabled.json\""),
            "issues switched off",
        ),
        (
            Some("git@github.com:example/widgets.git"),
            Some("echo 'HTTP 502: Bad Gateway' >&2\nexit 1"),
            "HTTP 502",
        ),
    ];
    for (remote, gh, expected) in cases {
        let tasks = Tasks::new(remote, gh);
        let said = tasks
            .ys(None, &["task", "list", "--project", "Demo"])
            .failed();
        assert!(said.contains(expected), "{expected}: {said}");
        assert!(!said.contains("query("), "not gh's command line: {said}");
    }
}

#[cfg(unix)]
#[test]
fn a_list_that_stops_short_says_so_beside_the_answer() {
    // The first page arrives and the second does not.
    let tasks = Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some(
            "case \"$*\" in\n\
             *after=*) echo 'HTTP 502: Bad Gateway' >&2; exit 1 ;;\n\
             *) cat \"$fixtures/issues-page-1.json\" ;;\nesac",
        ),
    );
    let run = tasks.ys(None, &["task", "list", "--project", "Demo", "--json"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(keys(&run.stdout).len(), 7, "what arrived is the answer");
    assert!(
        run.stderr.contains("Not every open task could be read"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("HTTP 502"), "{}", run.stderr);
}

#[cfg(unix)]
#[test]
fn starting_a_task_makes_a_workspace_named_after_it_and_remembers_which() {
    let tasks = Tasks::recorded();
    let started = tasks
        .ys(
            None,
            &["task", "start", "#14394", "--project", "Demo", "--no-agent"],
        )
        .ok();
    assert!(started.contains("created  14394-"), "{started}");
    assert!(started.contains("task   #14394"), "{started}");

    let listed = tasks.ys(None, &["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&listed).expect("valid JSON");
    let made: Vec<&serde_json::Value> = worktrees_in(&listed);
    assert_eq!(made.len(), 1);
    assert!(made[0]["name"].as_str().unwrap().starts_with("14394-"));
    assert!(std::path::Path::new(made[0]["path"].as_str().unwrap()).is_dir());
    let task = &made[0]["tasks"][0];
    assert_eq!(task["source"], "github");
    assert_eq!(task["key"], "#14394");
    assert_eq!(task["repo"], "github.com/example/widgets");
    assert_eq!(
        task["url"],
        "https://github.com/example/widgets/issues/14394"
    );
    assert_eq!(task["title"], "Replaced title of issue 14394");
    // The project's own checkout was not started from anything.
    let local = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == "local")
        .unwrap();
    assert_eq!(local["tasks"].as_array().unwrap().len(), 0);

    // A second attempt at the same task is a second workspace, with the same task on it.
    let again = tasks
        .ys(
            Some(&tasks.fx.repo),
            &["task", "start", "14394", "--no-agent", "--json"],
        )
        .ok();
    let again: serde_json::Value = serde_json::from_str(&again).expect("valid JSON");
    assert_eq!(again["task"], "#14394");
    assert_ne!(again["name"], made[0]["name"]);
    assert!(again["name"].as_str().unwrap().starts_with("14394-"));
}

#[cfg(unix)]
#[test]
fn a_workspace_made_any_other_way_has_no_task() {
    let tasks = Tasks::recorded();
    tasks
        .ys(
            None,
            &["workspace", "new", "Demo", "fix the login", "--no-agent"],
        )
        .ok();
    let listed = tasks.ys(None, &["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&listed).expect("valid JSON");
    assert_eq!(
        worktrees_in(&listed)[0]["tasks"].as_array().unwrap().len(),
        0
    );
}

#[cfg(unix)]
#[test]
fn a_task_that_cannot_be_started_leaves_nothing_behind() {
    let nothing_made = |tasks: &Tasks| {
        let listed = tasks.ys(None, &["workspace", "list", "--json"]).ok();
        let listed: serde_json::Value = serde_json::from_str(&listed).expect("valid JSON");
        assert!(worktrees_in(&listed).is_empty(), "{listed}");
    };

    // A closed one.
    let closed = Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some("cat \"$fixtures/issue-view-hidden.json\""),
    );
    let said = closed
        .ys(
            None,
            &["task", "start", "11809", "--project", "Demo", "--no-agent"],
        )
        .failed();
    assert!(said.contains("#11809 is closed"), "{said}");
    nothing_made(&closed);

    // Another repository's.
    let tasks = Tasks::recorded();
    let said = tasks
        .ys(
            None,
            &[
                "task",
                "start",
                "https://github.com/someone/else/issues/14394",
                "--project",
                "Demo",
                "--no-agent",
            ],
        )
        .failed();
    assert!(said.contains("github.com/someone/else"), "{said}");
    nothing_made(&tasks);

    // An agent that is not there: refused before `gh` is asked anything.
    let tasks = Tasks::recorded();
    let said = tasks
        .ys(
            None,
            &[
                "task",
                "start",
                "14394",
                "--project",
                "Demo",
                "--harness",
                "nope",
            ],
        )
        .failed();
    assert!(said.contains("nope"), "{said}");
    assert!(tasks.asked().is_empty(), "{:?}", tasks.asked());
    nothing_made(&tasks);
}

#[cfg(unix)]
#[test]
fn the_message_can_be_read_without_anything_being_made() {
    let tasks = Tasks::recorded();
    let printed = tasks
        .ys(
            None,
            // No such agent: `--print` starts nothing, so which one does not matter.
            &[
                "task",
                "start",
                "14394",
                "--project",
                "Demo",
                "--print",
                "--harness",
                "nope",
            ],
        )
        .ok();
    assert!(
        printed.starts_with("Work on this GitHub issue: #14394, https://github.com/example/"),
        "{printed}"
    );
    assert!(
        printed.contains("Title: Replaced title of issue 14394"),
        "{printed}"
    );
    assert!(printed.contains("Replaced description."), "{printed}");
    assert!(printed.contains("stop and ask me first"), "{printed}");
    assert!(
        !printed.contains("triage-bot"),
        "bots are left out: {printed}"
    );

    let json = tasks
        .ys(
            None,
            &[
                "task",
                "start",
                "14394",
                "--project",
                "Demo",
                "--print",
                "--json",
            ],
        )
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["task"]["key"], "#14394");
    assert!(parsed["prompt"].as_str().unwrap().contains("Fixes #14394"));

    let listed = tasks.ys(None, &["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&listed).expect("valid JSON");
    assert!(worktrees_in(&listed).is_empty(), "{listed}");
}

/// On GitHub, with a `gh` that reads the recorded issue, takes what is written to it — keeping
/// whatever comes in on standard input — and says a new issue is #93.
#[cfg(unix)]
fn writable() -> Tasks {
    Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some(
            "keep() { while IFS= read -r line || [ -n \"$line\" ]; do \
             printf '%s\\n' \"$line\"; done >> \"$here/stdin\"; }\n\
             case \"$*\" in\n\
             *'issue(number:'*) cat \"$fixtures/issue-view.json\" ;;\n\
             'issue create'*) keep; echo 'https://github.com/example/widgets/issues/93' ;;\n\
             'issue comment'*) keep ;;\n\
             esac",
        ),
    )
}

#[cfg(unix)]
impl Tasks {
    /// What `gh` was asked to change: everything but the lookups.
    fn written(&self) -> Vec<String> {
        self.asked()
            .into_iter()
            .filter(|asked| !asked.starts_with("api graphql"))
            .collect()
    }

    fn stdin(&self) -> String {
        std::fs::read_to_string(self.bin.path().join("stdin")).unwrap_or_default()
    }
}

#[cfg(unix)]
#[test]
fn a_task_is_created_and_its_key_and_link_are_printed() {
    let tasks = writable();
    let body = tasks.bin.path().join("body.md");
    std::fs::write(&body, "It **crashes** on start.\n").unwrap();
    let made = tasks
        .ys(
            Some(&tasks.fx.repo),
            &[
                "task",
                "create",
                "Crash on start",
                "--body-file",
                &body.to_string_lossy(),
                "--label",
                "bug",
                "--label",
                "good first issue",
                "--assignee",
                "@me",
            ],
        )
        .ok();
    assert!(made.contains("created  #93"), "{made}");
    assert!(
        made.contains("https://github.com/example/widgets/issues/93"),
        "{made}"
    );
    assert_eq!(
        tasks.written(),
        ["issue create --title Crash on start --body-file - --label bug --label good first issue --assignee @me"]
    );
    assert_eq!(tasks.stdin(), "It **crashes** on start.\n");

    let json = tasks
        .ys(
            None,
            &[
                "task",
                "create",
                "Another",
                "--body",
                "Words.",
                "--project",
                "Demo",
                "--json",
            ],
        )
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["key"], "#93");
    assert_eq!(
        parsed["url"],
        "https://github.com/example/widgets/issues/93"
    );
}

#[cfg(unix)]
#[test]
fn a_comment_is_posted_with_its_words_on_standard_input() {
    let tasks = writable();
    let said = tasks
        .ys(
            None,
            &[
                "task",
                "comment",
                "#14394",
                "Which version is this?",
                "--project",
                "Demo",
            ],
        )
        .ok();
    assert_eq!(said.trim(), "Commented on #14394.");
    assert_eq!(tasks.written(), ["issue comment 14394 --body-file -"]);
    assert_eq!(tasks.stdin(), "Which version is this?\n");

    // Nothing to say is nothing sent.
    let refused = tasks
        .ys(
            None,
            &["task", "comment", "14394", "  ", "--project", "Demo"],
        )
        .failed();
    assert!(refused.contains("write something first"), "{refused}");
    assert_eq!(tasks.written().len(), 1);
}

#[cfg(unix)]
#[test]
fn close_and_reopen_do_nothing_without_yes() {
    let tasks = writable();
    // The recorded issue is open.
    let would = tasks
        .ys(None, &["task", "close", "14394", "--project", "Demo"])
        .failed();
    assert!(
        would.contains("This would close #14394 as completed"),
        "{would}"
    );
    assert!(would.contains("Replaced title of issue 14394"), "{would}");
    assert!(would.contains("add --yes"), "{would}");
    assert!(tasks.written().is_empty(), "{:?}", tasks.written());

    let closed = tasks
        .ys(
            None,
            &[
                "task",
                "close",
                "14394",
                "--reason",
                "not-planned",
                "--yes",
                "--project",
                "Demo",
            ],
        )
        .ok();
    assert_eq!(closed.trim(), "Closed #14394.");
    assert_eq!(tasks.written(), ["issue close 14394 --reason not planned"]);

    // It is open as far as the source says, so there is nothing to reopen, --yes or not.
    let already = tasks
        .ys(
            None,
            &["task", "reopen", "14394", "--yes", "--project", "Demo"],
        )
        .failed();
    assert!(already.contains("#14394 is already open"), "{already}");
    assert_eq!(tasks.written().len(), 1);
}

#[cfg(unix)]
#[test]
fn a_closed_task_is_reopened_only_with_yes_and_not_closed_again() {
    let tasks = Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some("case \"$*\" in *'issue(number:'*) cat \"$fixtures/issue-view-hidden.json\" ;; esac"),
    );
    let written = |tasks: &Tasks| {
        tasks
            .asked()
            .into_iter()
            .filter(|asked| !asked.starts_with("api graphql"))
            .collect::<Vec<_>>()
    };
    let would = tasks
        .ys(None, &["task", "reopen", "11809", "--project", "Demo"])
        .failed();
    assert!(would.contains("This would reopen #11809"), "{would}");
    let already = tasks
        .ys(
            None,
            &["task", "close", "11809", "--yes", "--project", "Demo"],
        )
        .failed();
    assert!(already.contains("#11809 is already closed"), "{already}");
    assert!(written(&tasks).is_empty(), "{:?}", written(&tasks));

    let reopened = tasks
        .ys(
            None,
            &[
                "task",
                "reopen",
                "11809",
                "--yes",
                "--project",
                "Demo",
                "--json",
            ],
        )
        .ok();
    let parsed: serde_json::Value = serde_json::from_str(&reopened).expect("valid JSON");
    assert_eq!(parsed["task"], "#11809");
    assert_eq!(parsed["did"], "reopened");
    assert_eq!(written(&tasks), ["issue reopen 11809"]);
}

#[cfg(unix)]
#[test]
fn a_task_is_edited_and_an_edit_of_nothing_is_refused() {
    let tasks = writable();
    let nothing = tasks
        .ys(None, &["task", "edit", "14394", "--project", "Demo"])
        .failed();
    assert!(nothing.contains("Nothing to change"), "{nothing}");
    assert!(tasks.asked().is_empty());

    let changed = tasks
        .ys(
            None,
            &[
                "task",
                "edit",
                "14394",
                "--add-label",
                "bug",
                "--remove-label",
                "question",
                "--assign",
                "@me",
                "--unassign",
                "grace",
                "--title",
                "A better title",
                "--project",
                "Demo",
            ],
        )
        .ok();
    assert_eq!(changed.trim(), "Changed #14394.");
    assert_eq!(
        tasks.written(),
        ["issue edit 14394 --title A better title --add-label bug --remove-label question --add-assignee @me --remove-assignee grace"]
    );
}

#[cfg(unix)]
#[test]
fn nothing_is_written_through_a_link_to_another_repositorys_issue() {
    let tasks = writable();
    let elsewhere = "https://github.com/someone/else/issues/14394";
    for args in [
        vec!["task", "comment", elsewhere, "hello"],
        vec!["task", "close", elsewhere, "--yes"],
        vec!["task", "edit", elsewhere, "--add-label", "bug"],
    ] {
        let mut args = args;
        args.extend(["--project", "Demo"]);
        let refused = tasks.ys(None, &args).failed();
        assert!(refused.contains("github.com/someone/else"), "{refused}");
    }
    assert!(tasks.written().is_empty(), "{:?}", tasks.written());
}

/// A smoke test file with one mistake: a variable naming a field the workspace does not have.
const SMOKE_TEST_WITH_A_TYPO: &str = "id: ping
name: Ping
version: 1
steps:
  - id: ping
    action: run
    program: echo
    args: [\"{{ workspace.brnch }}\"]
";

fn write_smoke_test(worktree: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let folder = worktree.join(".yardsort").join("tests");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn smoke_tests_are_listed_from_the_workspace_s_own_folder() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "add a health check");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());

    // Nothing yet: say where they would go, and how to start one.
    let empty = fx.ys(&["test", "list", "--workspace", name]).ok();
    assert!(empty.contains("No smoke tests in"), "{empty}");
    assert!(empty.contains("ys test example"), "{empty}");

    let example = fx.ys(&["test", "example"]).ok();
    write_smoke_test(&path, "health.yaml", &example);
    write_smoke_test(&path, "ping.yaml", SMOKE_TEST_WITH_A_TYPO);
    // A test that needs no browser, so there is a `ready` row whether or not this machine has a
    // browser for the example's `browser` step.
    write_smoke_test(&path, "demo.yaml", SMOKE_TEST_WITH_GIT);
    let table = fx.ys(&["test", "list", "--workspace", name]).ok();
    assert!(table.contains("health"), "{table}");
    assert!(table.contains("ready"), "{table}");
    assert!(table.contains("1 problem"), "{table}");
    assert!(
        table.contains(&path.join(".yardsort").join("tests").display().to_string()),
        "the folder should be named: {table}"
    );

    // From inside the worktree, the workspace needs no naming.
    let json = fx.ys_from(Some(&path), &["test", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let ping = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "ping")
        .expect("the broken file is listed");
    assert_eq!(ping["runnable"], false);
    assert_eq!(ping["problems"][0]["line"], 8);
    let health = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "health")
        .unwrap();
    assert_eq!(health["runnable"], true);
    assert_eq!(health["usesBrowser"], true);

    // Outside any workspace, with none named, it says so.
    let error = fx.ys(&["test", "list"]).failed();
    assert!(error.contains("--workspace"), "{error}");
}

#[test]
fn a_smoke_test_is_shown_with_its_problems_on_stderr() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "add a health check");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());
    let file = write_smoke_test(&path, "ping.yaml", SMOKE_TEST_WITH_A_TYPO);

    let run = fx.ys(&["test", "show", "ping", "--workspace", name]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("id: ping"), "{}", run.stdout);
    assert!(
        run.stderr.contains(&format!("{}:8:13:", file.display())),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("has no `brnch`"), "{}", run.stderr);

    let error = fx
        .ys(&["test", "show", "nope", "--workspace", name])
        .failed();
    assert!(
        error.contains("There is no smoke test `nope`. There are: ping."),
        "{error}"
    );
}

#[test]
fn a_smoke_test_file_is_checked_without_a_profile() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ping.yaml");
    std::fs::write(&file, SMOKE_TEST_WITH_A_TYPO).unwrap();
    let nowhere = dir.path().join("no-profile");
    let output = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args(["--data-dir", &nowhere.to_string_lossy()])
        .args(["test", "validate", &file.to_string_lossy()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "{}:8:13: `{{{{ workspace.brnch }}}}`",
            file.display()
        )),
        "{stderr}"
    );

    // The example validates, read from standard input, and the guide's own commands exist.
    let example = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args(["--data-dir", &nowhere.to_string_lossy(), "test", "example"])
        .output()
        .unwrap();
    assert!(example.status.success());
    let mut validate = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args([
            "--data-dir",
            &nowhere.to_string_lossy(),
            "test",
            "validate",
            "-",
            "--json",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(validate.stdin.as_mut().unwrap(), &example.stdout).unwrap();
    let checked = validate.wait_with_output().unwrap();
    assert!(checked.status.success());
    let json: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(json["valid"], true);
    assert_eq!(json["id"], "health");

    let guide = Command::new(env!("CARGO_BIN_EXE_ys"))
        .args(["--data-dir", &nowhere.to_string_lossy(), "test", "guide"])
        .output()
        .unwrap();
    assert!(guide.status.success());
    assert!(String::from_utf8_lossy(&guide.stdout).contains("ys test validate"));
}

/// A smoke test that needs nothing but git, which these tests already need.
const SMOKE_TEST_WITH_GIT: &str = "id: demo
name: Git is there
version: 1
steps:
  - id: version
    action: run
    program: git
    args: [--version]
    expect:
      stdout_contains: git version
";

/// Carry out what is queued, as the app's thread would, and wait for it.
fn carry_out_test_runs(fx: &Fixture) {
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let env = std::sync::Arc::new(ShellEnv {
        vars: std::env::vars().collect(),
        source: EnvSource::Process,
        warning: None,
    });
    let driver = yardsort_core::smoke::runner::Driver::new(std::sync::Arc::new(|_| {}));
    driver.tick(&store, &fx.data_dir, &env).unwrap();
    driver.join();
}

#[test]
fn a_smoke_test_run_waits_for_approval_which_ys_cannot_give_then_is_carried_out() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "add a health check");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());
    write_smoke_test(&path, "demo.yaml", SMOKE_TEST_WITH_GIT);

    // With the app closed nothing would carry a run out, so nothing is queued.
    let closed = fx.ys_from(Some(&path), &["test", "run", "demo"]).failed();
    assert!(closed.contains("Yardsort is not running"), "{closed}");
    assert_eq!(fx.ys(&["test", "runs", "--json"]).ok().trim(), "[]");

    let _app = yardsort_core::presence::AppLock::take(&fx.data_dir)
        .unwrap()
        .unwrap();
    let bad_input = fx
        .ys_from(
            Some(&path),
            &["test", "run", "demo", "--input", "colour=red"],
        )
        .failed();
    assert!(bad_input.contains("has no input `colour`"), "{bad_input}");
    let missing = fx
        .ys(&["test", "run", "nope", "--workspace", name])
        .failed();
    assert!(
        missing.contains("There is no smoke test `nope`"),
        "{missing}"
    );

    // A file nobody has seen waits, and says so; the run's id is how to follow it.
    let queued = fx.ys_from(Some(&path), &["test", "run", "demo"]).ok();
    assert!(
        queued.contains("waiting for you to approve it in Yardsort"),
        "{queued}"
    );
    let json = fx.ys(&["test", "runs", "--json"]).ok();
    let runs: serde_json::Value = serde_json::from_str(&json).unwrap();
    let run = &runs[0];
    assert_eq!(run["status"], "awaiting_approval");
    assert_eq!(run["test"], "demo");
    let id = run["id"].as_str().unwrap().to_owned();
    let short = &id[..8];
    assert!(
        queued.contains(&format!("ys test wait {short}")),
        "{queued}"
    );
    let listed = fx.ys(&["test", "runs"]).ok();
    assert!(listed.contains("awaiting approval"), "{listed}");

    // Waiting for a run nobody approved ends with the limit, and exit 2: not a verdict.
    let waited = fx.ys(&["test", "wait", short, "--timeout", "1s"]);
    assert_eq!(waited.code, Some(2), "{}", waited.stderr);
    assert!(
        waited
            .stdout
            .contains("waiting for you to approve it in Yardsort"),
        "{}",
        waited.stdout
    );
    assert!(
        waited.stdout.contains("still awaiting approval after 1s"),
        "{}",
        waited.stdout
    );
    let nothing = fx.ys(&["test", "log", short]).failed();
    assert!(nothing.contains("has not run any step yet"), "{nothing}");
    // A second ask while one waits is refused, not queued twice.
    let twice = fx.ys_from(Some(&path), &["test", "run", "demo"]).failed();
    assert!(twice.contains("is already going"), "{twice}");

    // The user approves in the app — here, the store — and the app's runner carries it out.
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    assert!(store.approve_test_run(&id).unwrap());
    carry_out_test_runs(&fx);

    let passed = fx.ys(&["test", "wait", short]);
    assert_eq!(passed.code, Some(0), "{}", passed.stderr);
    assert!(
        passed
            .stdout
            .starts_with("passed  demo  (1 of 1 steps passed, "),
        "{}",
        passed.stdout
    );
    let log = fx.ys(&["test", "log", short]).ok();
    assert!(log.contains("git version"), "{log}");
    let log_json: serde_json::Value =
        serde_json::from_str(&fx.ys(&["test", "log", short, "--json"]).ok()).unwrap();
    assert_eq!(log_json["step"], "version");
    assert!(log_json["path"]
        .as_str()
        .unwrap()
        .ends_with("version.stdout"));
    assert_eq!(log_json["outputs"]["exit"], "0");
    let no_such = fx.ys(&["test", "log", short, "--step", "nope"]).failed();
    assert!(no_such.contains("has no step `nope`"), "{no_such}");
    let shown = fx.ys(&["test", "runs", "--run", short]).ok();
    assert!(shown.contains("passed  demo"), "{shown}");
    let row = shown
        .lines()
        .find(|l| l.starts_with("version"))
        .unwrap_or_default();
    let cells: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(cells, ["version", "steps", "run", "passed"], "{shown}");
    let ended = fx.ys(&["test", "cancel", short]).failed();
    assert!(ended.contains("has already ended: passed"), "{ended}");

    // The same file is approved now: the next run goes straight to the queue.
    let again = fx.ys_from(Some(&path), &["test", "run", "demo"]).ok();
    assert!(!again.contains("approve"), "{again}");
    let json = fx.ys(&["test", "runs", "--json"]).ok();
    let runs: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(runs[0]["status"], "queued");
    let second = runs[0]["id"].as_str().unwrap().to_owned();
    let cancelled = fx.ys(&["test", "cancel", &second[..8]]).ok();
    assert!(cancelled.contains("Cancelled run"), "{cancelled}");
    let waited = fx.ys(&["test", "wait", &second[..8]]);
    assert_eq!(waited.code, Some(2));
    assert!(
        waited.stdout.contains("Cancelled from ys."),
        "{}",
        waited.stdout
    );

    // A changed file is a new file: it waits again.
    write_smoke_test(
        &path,
        "demo.yaml",
        &SMOKE_TEST_WITH_GIT.replace("Git is there", "Git is still there"),
    );
    let changed = fx.ys_from(Some(&path), &["test", "run", "demo"]).ok();
    assert!(changed.contains("waiting for you to approve"), "{changed}");
}

#[test]
fn a_schedule_is_added_listed_shown_paused_resumed_and_removed() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();

    let empty = fx.ys(&["schedule", "list"]).ok();
    assert!(empty.contains("No schedules"), "{empty}");

    let json = fx
        .ys(&[
            "schedule",
            "add",
            "--workflow",
            "ping",
            "--workspace",
            name,
            "--input",
            "note=Hi there",
            "--weekly",
            "mon-fri",
            "9:00",
            "--json",
        ])
        .ok();
    let added: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(added["name"], "Ping", "named after what it runs");
    assert_eq!(added["when"], "Weekdays at 09:00");
    assert_eq!(added["workspace"], name);
    assert_eq!(added["inputs"]["note"], "Hi there");
    assert_eq!(added["standing"], "waiting");
    assert_eq!(added["ifMissed"], "run_once");
    assert!(added["nextAt"].is_i64(), "{added}");

    let nightly = fx
        .ys(&[
            "schedule",
            "add",
            "--workflow",
            "ping",
            "--workspace",
            name,
            "--name",
            "Nightly",
            "--cron",
            "0 3 * * *",
            "--if-missed",
            "skip",
            "--notify",
            "always",
        ])
        .ok();
    assert!(nightly.contains("Added `Nightly`"), "{nightly}");
    assert!(nightly.contains("Cron `0 3 * * *`"), "{nightly}");
    assert!(nightly.contains("Next: "), "{nightly}");
    assert!(
        nightly.contains("Yardsort is not running. Schedules fire while it is open."),
        "{nightly}"
    );

    let table = fx.ys(&["schedule", "list"]).ok();
    assert!(
        table.contains("Nightly") && table.contains("Ping"),
        "{table}"
    );
    assert!(table.contains("workflow ping"), "{table}");
    assert!(table.contains("Weekdays at 09:00"), "{table}");

    let shown = fx.ys(&["schedule", "show", "nightly"]).ok();
    assert!(shown.contains("Runs the workflow `ping`"), "{shown}");
    assert!(shown.contains("skips it"), "{shown}");
    assert!(shown.contains("how every run ends"), "{shown}");
    let next = shown.split("Next:").nth(1).expect("its next times");
    assert_eq!(
        next.lines().filter(|line| line.contains("03:00")).count(),
        5,
        "{shown}"
    );

    let off = fx.ys(&["schedule", "pause", "Nightly"]).ok();
    assert!(off.contains("`Nightly` is off"), "{off}");
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["schedule", "list", "--json"]).ok()).unwrap();
    let paused = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Nightly")
        .unwrap();
    assert_eq!(paused["enabled"], false);
    assert_eq!(paused["standing"], "off");
    assert!(
        paused["nextAt"].is_null(),
        "off, it has no next time to show"
    );
    let on = fx.ys(&["schedule", "resume", "Nightly"]).ok();
    assert!(on.contains("`Nightly` is on. Next: "), "{on}");

    let history = fx.ys(&["schedule", "runs", "Nightly"]).ok();
    assert!(history.contains("has not come due yet"), "{history}");

    // By the start of its id, too.
    let id = paused["id"].as_str().unwrap();
    let removed = fx.ys(&["schedule", "remove", &id[..8]]).ok();
    assert!(removed.contains("Removed `Nightly`"), "{removed}");
    let left: serde_json::Value =
        serde_json::from_str(&fx.ys(&["schedule", "list", "--json"]).ok()).unwrap();
    assert_eq!(left.as_array().unwrap().len(), 1);
    let gone = fx.ys(&["schedule", "show", "Nightly"]).failed();
    assert!(gone.contains("No schedule called"), "{gone}");
    assert!(gone.contains("Ping"), "it says which there are: {gone}");
}

#[test]
fn a_schedule_is_refused_for_what_is_wrong_with_it_and_nothing_is_written() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let add = |rest: &[&str]| {
        let mut args = vec!["schedule", "add", "--workspace", name];
        args.extend_from_slice(rest);
        fx.ys(&args)
    };

    let unknown = add(&["--workflow", "nope", "--daily", "09:00"]).failed();
    assert!(unknown.contains("There is no workflow `nope`"), "{unknown}");
    let no_test = add(&["--test", "nope", "--daily", "09:00"]).failed();
    assert!(
        no_test.contains("There is no smoke test `nope`"),
        "{no_test}"
    );
    let input = add(&[
        "--workflow",
        "ping",
        "--input",
        "colour=red",
        "--daily",
        "09:00",
    ])
    .failed();
    assert!(input.contains("has no input `colour`"), "{input}");

    let time = add(&["--workflow", "ping", "--daily", "nine"]).failed();
    assert!(time.contains("`nine` is not a time of day"), "{time}");
    let days = add(&["--workflow", "ping", "--weekly", "funday", "09:00"]).failed();
    assert!(days.contains("is not days of the week"), "{days}");
    let day = add(&["--workflow", "ping", "--monthly", "31", "09:00"]).failed();
    assert!(day.contains("1 to 28"), "{day}");
    let every = add(&["--workflow", "ping", "--every", "soon"]).failed();
    assert!(every.contains("is not an interval"), "{every}");
    let cron = add(&["--workflow", "ping", "--cron", "0 9 * *"]).failed();
    assert!(cron.contains("has 4 fields"), "{cron}");
    let past = add(&["--workflow", "ping", "--at", "2020-01-01 09:00"]).failed();
    assert!(past.contains("That time has already passed"), "{past}");

    // Clap's own: what to run and when are each asked for once. It exits 2, as for any misuse.
    let nothing = add(&["--daily", "09:00"]);
    assert_eq!(nothing.code, Some(2), "{}", nothing.stderr);
    let twice = add(&["--workflow", "ping", "--daily", "09:00", "--every", "1h"]);
    assert_eq!(twice.code, Some(2), "{}", twice.stderr);

    let listed = fx.ys(&["schedule", "list", "--json"]).ok();
    assert_eq!(listed.trim(), "[]");
}

#[test]
fn a_schedules_run_is_started_now_only_while_the_app_holds_its_lock() {
    let fx = Fixture::new();
    write_workflow(&fx, "ping.yaml", PING);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    fx.ys(&[
        "schedule",
        "add",
        "--workflow",
        "ping",
        "--workspace",
        name,
        "--every",
        "2h",
    ])
    .ok();

    let closed = fx.ys(&["schedule", "run", "Ping"]).failed();
    assert!(closed.contains("Yardsort is not running"), "{closed}");
    assert!(closed.contains("Nothing was queued"), "{closed}");

    // This test stands in for the running app. Nothing drives the run, so it stays queued.
    let _app = yardsort_core::presence::AppLock::take(&fx.data_dir)
        .unwrap()
        .expect("nobody else holds it");
    let json = fx.ys(&["schedule", "run", "Ping", "--json"]).ok();
    let queued: serde_json::Value = serde_json::from_str(&json).unwrap();
    let run = queued["run"].as_str().unwrap().to_owned();
    assert_eq!(queued["target"], "workflow");

    let runs: serde_json::Value =
        serde_json::from_str(&fx.ys(&["workflow", "runs", "--json"]).ok()).unwrap();
    assert_eq!(runs[0]["id"], run.as_str());
    assert_eq!(runs[0]["status"], "queued");
    assert_eq!(runs[0]["requestedBy"], "app");

    let again = fx.ys(&["schedule", "run", "Ping"]).failed();
    assert!(again.contains("already running"), "{again}");

    let history = fx.ys(&["schedule", "runs", "Ping"]).ok();
    assert!(history.contains(&run[..8]), "{history}");
    assert!(history.contains("running"), "{history}");
    assert!(history.contains("Run now"), "{history}");
    let listed = fx.ys(&["schedule", "list"]).ok();
    assert!(listed.contains("running"), "{listed}");
    assert!(
        !listed.contains("Yardsort is not running"),
        "the app is open: {listed}"
    );
}

#[test]
fn a_test_scheduled_from_ys_says_its_file_still_needs_approving() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());
    write_smoke_test(&path, "demo.yaml", SMOKE_TEST_WITH_GIT);

    let added = fx
        .ys_from(
            Some(&path),
            &["schedule", "add", "--test", "demo", "--every", "1d"],
        )
        .ok();
    assert!(added.contains("Every day"), "{added}");
    assert!(
        added.contains("`demo` has not been approved in Yardsort"),
        "{added}"
    );
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["schedule", "list", "--json"]).ok()).unwrap();
    assert_eq!(listed[0]["target"], "test");
    assert_eq!(listed[0]["workspace"], name, "the workspace it was run in");
}

#[test]
fn an_agent_tracks_a_pull_request_it_opened_in_another_repository() {
    let fx = Fixture::new();
    let made = make_workspace(&fx, "Bump the library");
    let workspace_id = made["id"].as_str().unwrap().to_owned();
    let name = made["name"].as_str().unwrap().to_owned();
    fx.git()
        .run(
            &fx.repo,
            &["remote", "add", "origin", "git@github.com:o/parent.git"],
        )
        .unwrap();
    // Where the agent stands when it opens the pull request: a nested clone, outside its
    // worktree. Its launch environment is what says which workspace it is.
    let nested = tempfile::tempdir().unwrap();
    let as_agent = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_ys"))
            .args(["--data-dir", &fx.data_dir.to_string_lossy()])
            .args(args)
            .env_remove(yardsort_core::activity::RUN_ENV)
            .env_remove(yardsort_core::activity::RECORD_ENV)
            .env(yardsort_core::activity::WORKSPACE_ENV, &workspace_id)
            .env("YARDSORT_NO_DAEMON", "1")
            .current_dir(nested.path())
            .output()
            .unwrap();
        Run {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    };

    let said = as_agent(&["pr", "track", "https://github.com/o/lib/pull/7/files"]).ok();
    assert!(
        said.contains(&format!(
            "Tracking https://github.com/o/lib/pull/7 with {name}"
        )),
        "{said}"
    );
    let again = as_agent(&["pr", "track", "https://github.com/o/lib/pull/7"]).ok();
    assert!(again.contains("tracked with"), "{again}");

    let listed: serde_json::Value =
        serde_json::from_str(&as_agent(&["pr", "tracked", "--json"]).ok()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["repo"], "github.com/o/lib");
    assert_eq!(listed[0]["number"], 7);
    assert_eq!(listed[0]["trackedBy"], "cli");

    // The project's own repository is followed already, and a link to something else is no
    // pull request at all.
    let own = as_agent(&["pr", "track", "https://github.com/o/parent/pull/3"]).failed();
    assert!(own.contains("this project's own repository"), "{own}");
    let issue = as_agent(&["pr", "track", "https://github.com/o/lib/issues/3"]).failed();
    assert!(issue.contains("not a link to a pull request"), "{issue}");

    // Outside any workspace, with nothing in the environment: say how to name one.
    let lost = fx
        .ys_from(
            Some(nested.path()),
            &["pr", "track", "https://github.com/o/lib/pull/8"],
        )
        .failed();
    assert!(lost.contains("--workspace"), "{lost}");
    fx.ys_from(
        Some(nested.path()),
        &[
            "pr",
            "track",
            "https://github.com/o/lib/pull/8",
            "--workspace",
            &name,
        ],
    )
    .ok();

    as_agent(&["pr", "untrack", "https://github.com/o/lib/pull/7"]).ok();
    let gone = as_agent(&["pr", "untrack", "https://github.com/o/lib/pull/7"]).failed();
    assert!(gone.contains("is not tracked with"), "{gone}");
    let left = as_agent(&["pr", "tracked"]).ok();
    assert!(left.contains("https://github.com/o/lib/pull/8"), "{left}");
    assert!(!left.contains("/pull/7"), "{left}");
}

/// Tells the user what the agent said when it was done.
const TELL: &str = "id: tell
name: Tell
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: \"Done: {{ event.note }}\"
";

#[test]
fn a_trigger_is_added_listed_shown_paused_resumed_and_removed() {
    let fx = Fixture::new();
    write_workflow(&fx, "tell.yaml", TELL);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();

    let empty = fx.ys(&["trigger", "list"]).ok();
    assert!(empty.contains("No triggers"), "{empty}");

    let json = fx
        .ys(&[
            "trigger",
            "add",
            "--on",
            "workspace.done",
            "--workflow",
            "tell",
            "--workspace",
            name,
            "--quiet",
            "30s",
            "--json",
        ])
        .ok();
    let added: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(added["name"], "Tell", "named after what it runs");
    assert_eq!(added["events"], serde_json::json!(["workspace.done"]));
    assert_eq!(added["quietMs"], 30_000);
    assert_eq!(added["cap"], 3);
    assert_eq!(added["standing"], "listening");
    assert_eq!(
        added["when"],
        "When an agent says its work is done, run the workflow `tell`"
    );
    let id = added["id"].as_str().unwrap().to_owned();

    let table = fx.ys(&["trigger", "list"]).ok();
    assert!(table.contains("workspace.done"), "{table}");
    assert!(table.contains("workflow tell"), "{table}");
    assert!(
        table.contains("Yardsort is not running. Triggers fire while it is open."),
        "{table}"
    );

    let shown = fx.ys(&["trigger", "show", "tell"]).ok();
    assert!(shown.contains("Waits 30s after the last event"), "{shown}");
    assert!(shown.contains("It is on."), "{shown}");

    let off = fx.ys(&["trigger", "pause", "Tell"]).ok();
    assert!(off.contains("is off"), "{off}");
    let listed: serde_json::Value =
        serde_json::from_str(&fx.ys(&["trigger", "list", "--json"]).ok()).unwrap();
    assert_eq!(listed[0]["standing"], "off");
    let on = fx.ys(&["trigger", "resume", "Tell"]).ok();
    assert!(on.contains("is on, from now"), "{on}");

    let history = fx.ys(&["trigger", "fires", "Tell"]).ok();
    assert!(history.contains("has not fired yet"), "{history}");

    let removed = fx.ys(&["trigger", "remove", &id[..8]]).ok();
    assert!(removed.contains("Removed `Tell`"), "{removed}");
    let gone = fx.ys(&["trigger", "show", "Tell"]).failed();
    assert!(gone.contains("There are no triggers"), "{gone}");
}

#[test]
fn a_trigger_is_refused_for_what_is_wrong_with_it_and_nothing_is_written() {
    let fx = Fixture::new();
    write_workflow(&fx, "tell.yaml", TELL);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let refused = |extra: &[&str], wanted: &str| {
        let mut args = vec!["trigger", "add", "--workspace", name];
        args.extend_from_slice(extra);
        let error = fx.ys(&args).failed();
        assert!(error.contains(wanted), "{args:?}: {error}");
    };
    refused(
        &["--on", "pr.approved", "--workflow", "tell"],
        "There is no event `pr.approved`",
    );
    refused(
        &["--preset", "review-when-done"],
        "Who reviews? Say with `--input reviewer=…`",
    );
    refused(
        &["--on", "pr.review", "--workflow", "tell"],
        "`pr.review` does not leave",
    );
    refused(
        &["--on", "workspace.done", "--workflow", "nothing"],
        "There is no workflow `nothing`",
    );
    refused(
        &[
            "--on",
            "workspace.done",
            "--new-commits",
            "--workflow",
            "tell",
        ],
        "which this trigger does not listen for",
    );
    refused(
        &["--on", "agent.settled", "--workflow", "tell"],
        "`agent.settled` does not leave",
    );
    refused(
        &[
            "--on",
            "agent.settled",
            "--on",
            "workspace.done",
            "--workflow",
            "tell",
        ],
        "cannot share a trigger",
    );
    let listed = fx.ys(&["trigger", "list", "--json"]).ok();
    assert_eq!(listed.trim(), "[]");
}

#[test]
fn done_is_written_down_for_the_app_and_says_whether_anything_listens() {
    let fx = Fixture::new();
    write_workflow(&fx, "tell.yaml", TELL);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());

    let unheard = fx
        .ys_from(Some(&path), &["workspace", "done", "--note", "first"])
        .ok();
    assert!(
        unheard.contains(&format!("the work in {name} is done")),
        "{unheard}"
    );
    assert!(unheard.contains("No trigger"), "{unheard}");

    fx.ys(&[
        "trigger",
        "add",
        "--on",
        "workspace.done",
        "--workflow",
        "tell",
        "--workspace",
        name,
    ])
    .ok();
    let json = fx
        .ys_from(
            Some(&path),
            &["workspace", "done", "--note", "tests added", "--json"],
        )
        .ok();
    let said: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(said["heard"], true);
    assert_eq!(said["note"], "tests added");
    assert!(
        said["head"].as_str().is_some_and(|h| h.len() == 40),
        "{said}"
    );

    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let signals = store.done_signals_after(0, 10).unwrap();
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[1].note.as_deref(), Some("tests added"));
    assert_eq!(signals[1].head.as_deref(), said["head"].as_str());
}

#[test]
fn a_custom_event_is_raised_for_the_triggers_that_listen_for_its_name() {
    let fx = Fixture::new();
    write_workflow(&fx, "tell.yaml", TELL);
    let made = make_workspace(&fx, "task");
    let name = made["name"].as_str().unwrap();
    let path = PathBuf::from(made["path"].as_str().unwrap());

    let unheard = fx
        .ys_from(Some(&path), &["event", "emit", "migration-written"])
        .ok();
    assert!(
        unheard.contains(&format!("Raised `custom.migration-written` in {name}")),
        "{unheard}"
    );
    assert!(unheard.contains("No trigger"), "{unheard}");

    // A name that does not fit is refused when listened for, and when raised.
    let bad = fx
        .ys(&[
            "trigger",
            "add",
            "--on",
            "custom.Migration",
            "--workflow",
            "tell",
            "--workspace",
            name,
        ])
        .failed();
    assert!(bad.contains("`migration` would do"), "{bad}");
    let bad = fx
        .ys_from(Some(&path), &["event", "emit", "two words"])
        .failed();
    assert!(bad.contains("cannot be an event's name"), "{bad}");

    let added = fx
        .ys(&[
            "trigger",
            "add",
            "--on",
            "custom.migration-written",
            "--workflow",
            "tell",
            "--workspace",
            name,
            "--json",
        ])
        .ok();
    let added: serde_json::Value = serde_json::from_str(&added).unwrap();
    assert_eq!(
        added["events"],
        serde_json::json!(["custom.migration-written"])
    );
    assert_eq!(
        added["when"],
        "When `custom.migration-written` is raised, run the workflow `tell`"
    );

    // A typo is told what is listened for instead.
    let typo = fx
        .ys_from(Some(&path), &["event", "emit", "migration-writen"])
        .ok();
    assert!(
        typo.contains("They listen for: `custom.migration-written`"),
        "{typo}"
    );

    // `custom.` in front is taken too, as a trigger is written.
    let json = fx
        .ys_from(
            Some(&path),
            &[
                "event",
                "emit",
                "custom.migration-written",
                "--note",
                "0042 adds the column",
                "--json",
            ],
        )
        .ok();
    let raised: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(raised["heard"], true);
    assert_eq!(raised["event"], "custom.migration-written");
    assert!(
        raised["head"].as_str().is_some_and(|h| h.len() == 40),
        "{raised}"
    );

    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let rows = store.custom_events_after(0, 10).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["migration-written", "migration-writen", "migration-written"]
    );
    assert_eq!(rows[2].note.as_deref(), Some("0042 adds the column"));
    assert_eq!(rows[2].run_id, None);
}

#[cfg(unix)]
#[test]
fn a_preset_watches_the_pull_request_from_what_is_on_it_when_it_is_added() {
    let tasks = Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some(
            "case \"$*\" in\n\
             *'pr list'*) cat \"$here/list.json\" ;;\n\
             *graphql*) cat \"$here/look.json\" ;;\nesac",
        ),
    );
    let presets = tasks.ys(None, &["trigger", "presets"]).ok();
    assert!(
        presets.contains("address-reviews — Address reviews"),
        "{presets}"
    );
    assert!(presets.contains("--input reviewer=…"), "{presets}");

    let made = make_workspace(&tasks.fx, "task");
    let name = made["name"].as_str().unwrap();
    let branch = made["branch"].as_str().unwrap();
    let here = tasks.bin.path();
    std::fs::write(
        here.join("list.json"),
        serde_json::json!([{
            "number": 7, "url": "https://github.com/example/widgets/pull/7", "title": "Fix",
            "headRefName": branch, "state": "OPEN", "isDraft": false, "statusCheckRollup": []
        }])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        here.join("look.json"),
        serde_json::json!({"data": {"repository": {"pullRequest": {
            "number": 7, "url": "https://github.com/example/widgets/pull/7", "title": "Fix",
            "state": "OPEN", "isDraft": false, "headRefName": branch, "headRefOid": "abc",
            "mergeable": "MERGEABLE", "commits": {"nodes": []},
            "reviews": {"nodes": [{"databaseId": 1, "state": "COMMENTED",
                "submittedAt": "2026-11-07T07:51:50Z", "body": "Hm.", "url": "r1",
                "author": {"__typename": "User", "login": "alice"},
                "comments": {"totalCount": 0, "nodes": []}}]},
            "comments": {"nodes": []}
        }}, "viewer": {"login": "me"}}})
        .to_string(),
    )
    .unwrap();

    let added = tasks
        .ys(
            None,
            &[
                "trigger",
                "add",
                "--preset",
                "address-reviews",
                "--workspace",
                name,
            ],
        )
        .ok();
    assert!(added.contains("Added `Address reviews`"), "{added}");
    assert!(
        added.contains("Watching #7: what is there now is not news"),
        "{added}"
    );
    let listed: serde_json::Value =
        serde_json::from_str(&tasks.ys(None, &["trigger", "list", "--json"]).ok()).unwrap();
    assert_eq!(
        listed[0]["events"],
        serde_json::json!(["pr.review", "pr.comment"])
    );
    assert_eq!(
        listed[0]["quietMs"], 120_000,
        "two minutes, for a burst of posts"
    );
    assert_eq!(listed[0]["target"], "workflow");
    assert_eq!(listed[0]["targetId"], "address-review");

    let store = Store::open(&tasks.fx.data_dir.join("yardsort.db")).unwrap();
    let workspace = listed[0]["workspaceId"].as_str().unwrap();
    let watch = store.pr_watch(workspace).unwrap().unwrap();
    assert!(watch.seeded);
    assert_eq!(watch.number, Some(7));
    assert!(watch.seen.contains("review:1"), "{watch:?}");
    assert!(
        tasks
            .asked()
            .iter()
            .any(|asked| asked.contains("api graphql")),
        "{:?}",
        tasks.asked()
    );
}

/// On GitHub, with a `gh` that has the recorded open pull requests, two pages of them.
#[cfg(unix)]
fn pull_requests() -> Tasks {
    Tasks::new(
        Some("git@github.com:example/widgets.git"),
        Some(
            "case \"$*\" in\n\
             *pullRequests*after=*) cat \"$fixtures/open-page-2.json\" ;;\n\
             *pullRequests*) cat \"$fixtures/open-page-1.json\" ;;\n\
             *) echo \"not recorded: $*\" >&2; exit 1 ;;\nesac",
        ),
    )
}

#[cfg(unix)]
fn numbers(json: &str) -> Vec<u64> {
    let parsed: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
    parsed
        .as_array()
        .expect("a list")
        .iter()
        .map(|pr| pr["number"].as_u64().unwrap())
        .collect()
}

#[cfg(unix)]
#[test]
fn open_pull_requests_are_listed_from_every_page_with_what_needs_attention() {
    let prs = pull_requests();
    let table = prs.ys(None, &["pr", "list"]).ok();
    let lines: Vec<&str> = table.lines().collect();
    assert!(lines[0].starts_with("PROJECT  PR"), "{table}");
    assert_eq!(lines.len(), 8, "a heading and seven pull requests: {table}");
    assert!(
        table.contains("#13506  open   changes requested"),
        "{table}"
    );
    let asked = prs.asked();
    assert_eq!(asked.len(), 2, "two pages: {asked:?}");
    assert!(asked[1].contains("after="), "{asked:?}");

    // Failing checks, conflicts, changes asked for, a review asked of you, or none at all.
    let json = prs.ys(None, &["pr", "list", "--attention", "--json"]).ok();
    assert_eq!(numbers(&json), [14572, 13506, 13947]);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["attention"], serde_json::json!(["unreviewed"]));
    assert_eq!(
        parsed[1]["attention"],
        serde_json::json!(["changesRequested"])
    );
    assert_eq!(parsed[2]["attention"], serde_json::json!(["conflicts"]));
    assert_eq!(parsed[0]["viewer"], "ada");
}

#[cfg(unix)]
#[test]
fn pull_requests_are_narrowed_by_the_pull_request_views_review_rules() {
    let prs = pull_requests();
    let list = |args: &[&str]| {
        let mut all = vec!["pr", "list", "--json"];
        all.extend_from_slice(args);
        numbers(&prs.ys(None, &all).ok())
    };
    assert_eq!(list(&["--reviews", "none"]), [14572, 14044]);
    assert_eq!(list(&["--reviews", "approved"]), [13665]);
    assert_eq!(list(&["--reviews", "changes"]), [13506]);
    // "ada" is who gh is logged in as: her own pull request is not one she failed to review.
    assert_eq!(
        list(&["--reviews", "not-by-me"]),
        [14578, 14572, 14044, 13665, 13506, 14017]
    );
    assert_eq!(list(&["--author", "@me"]), [13947]);
    assert_eq!(list(&["--author", "Frances"]), [13506]);
    assert_eq!(list(&["--no-drafts"]), [14572, 13665, 13506]);
    assert_eq!(list(&["--limit", "2"]), [14578, 14572]);
    let none = prs
        .ys(None, &["pr", "list", "--reviews", "awaiting-me"])
        .ok();
    assert_eq!(none.trim(), "No open pull requests match.");
}

#[cfg(unix)]
#[test]
fn a_pull_request_with_a_workspace_says_which() {
    let prs = pull_requests();
    let fx = &prs.fx;
    fx.git().run(&fx.repo, &["branch", "branch-14572"]).unwrap();
    let store = Store::open(&fx.data_dir.join("yardsort.db")).unwrap();
    let project = store.projects().unwrap().remove(0);
    let git = fx.git();
    let settings = WorkspaceSettings::default();
    Workspaces {
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
    .open_branch(&project.id, "branch-14572")
    .unwrap();

    let table = prs.ys(None, &["pr", "list"]).ok();
    assert!(
        table.contains("Add a retry to the uploader (in branch-14572)"),
        "{table}"
    );
    let json = prs.ys(None, &["pr", "list", "--json"]).ok();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[1]["workspace"], "branch-14572");
    assert_eq!(parsed[0]["workspace"], serde_json::Value::Null);
}

#[cfg(unix)]
#[test]
fn a_project_not_on_github_is_said_and_the_rest_still_listed() {
    let prs = Tasks::new(None, Some("echo \"not asked: $*\" >&2; exit 1"));
    let run = prs.ys(None, &["pr", "list"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.stdout.trim(), "No open pull requests.");
    assert!(
        run.stderr.contains("Demo: no remote on GitHub"),
        "{}",
        run.stderr
    );
    assert!(
        prs.asked().is_empty(),
        "gh was not asked: {:?}",
        prs.asked()
    );
}

#[cfg(unix)]
#[test]
fn listing_pull_requests_without_gh_says_how_to_get_it() {
    let prs = Tasks::new(Some("git@github.com:example/widgets.git"), None);
    let refused = prs.ys(None, &["pr", "list"]).failed();
    assert!(refused.contains("https://cli.github.com"), "{refused}");
}

#[cfg(unix)]
#[test]
fn a_pull_request_is_checked_out_into_a_workspace_once() {
    let remote = tempfile::tempdir().unwrap();
    let git = git();
    git.run(remote.path(), &["init", "--bare", "-q"]).unwrap();
    let prs = Tasks::new(
        Some(&remote.path().to_string_lossy()),
        Some(
            "case \"$*\" in\n\
             'pr view 13665 '*) cat \"$fixtures/pr-view.json\" ;;\n\
             *) echo \"not recorded: $*\" >&2; exit 1 ;;\nesac",
        ),
    );
    let fx = &prs.fx;
    // Their branch is on the remote and not here.
    git.run(
        &fx.repo,
        &["push", "-q", "origin", "HEAD:refs/heads/branch-13665"],
    )
    .unwrap();
    assert!(!git.branch_exists(&fx.repo, "branch-13665").unwrap());

    let json = prs
        .ys(
            None,
            &["pr", "checkout", "#13665", "--project", "Demo", "--json"],
        )
        .ok();
    let made: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(made["existing"], false);
    assert_eq!(made["fork"], false);
    assert_eq!(made["branch"], "branch-13665");
    assert_eq!(made["pullRequest"], 13665);
    let path = PathBuf::from(made["path"].as_str().unwrap());
    assert!(
        path.join(".git").exists(),
        "a worktree at {}",
        path.display()
    );
    assert_eq!(
        git.run(&path, &["branch", "--show-current"])
            .unwrap()
            .trim(),
        "branch-13665"
    );

    // Asked again, the same workspace, and nothing new is made.
    let again = prs
        .ys(None, &["pr", "checkout", "13665", "--project", "Demo"])
        .ok();
    assert!(
        again.starts_with("#13665 is already in branch-13665"),
        "{again}"
    );
    assert!(
        again.contains("ys workflow run code-review --workspace branch-13665"),
        "{again}"
    );
    let listed = fx.ys(&["workspace", "list", "--json"]).ok();
    let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(worktrees_in(&listed).len(), 1);
    assert!(!again.contains("behind"), "{again}");

    // The pull request moves on. The workspace is left as it is, and said to be behind, so a
    // second review is not quietly run on the old code.
    let before = git.run(&path, &["rev-parse", "HEAD"]).unwrap();
    let pushed = git
        .run(
            &fx.repo,
            &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "One more"],
        )
        .unwrap();
    git.run(
        &fx.repo,
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/branch-13665", pushed.trim()),
        ],
    )
    .unwrap();
    let moved = prs
        .ys(
            None,
            &["pr", "checkout", "13665", "--project", "Demo", "--json"],
        )
        .ok();
    let moved: serde_json::Value = serde_json::from_str(&moved).unwrap();
    assert_eq!(moved["existing"], true);
    assert_eq!(moved["behind"], 1);
    assert_eq!(git.run(&path, &["rev-parse", "HEAD"]).unwrap(), before);
    let said = prs
        .ys(None, &["pr", "checkout", "13665", "--project", "Demo"])
        .ok();
    assert!(
        said.contains("The workspace is 1 commit behind the pull request"),
        "{said}"
    );

    // With the remote out of reach the workspace is still handed back, and how far behind it is
    // is said to be unknown rather than none.
    let gone = remote.path().join("gone");
    git.run(
        &fx.repo,
        &["remote", "set-url", "origin", &gone.to_string_lossy()],
    )
    .unwrap();
    let unreachable = prs.ys(
        None,
        &["pr", "checkout", "13665", "--project", "Demo", "--json"],
    );
    let stderr = unreachable.stderr.clone();
    let unreachable: serde_json::Value = serde_json::from_str(&unreachable.ok()).unwrap();
    assert_eq!(unreachable["existing"], true);
    assert_eq!(unreachable["behind"], serde_json::Value::Null);
    assert!(
        stderr.contains("Could not check whether branch-13665 is behind #13665"),
        "{stderr}"
    );

    let refused = prs
        .ys(None, &["pr", "checkout", "twelve", "--project", "Demo"])
        .failed();
    assert!(refused.contains("not a pull request number"), "{refused}");
}

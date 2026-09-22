//! `ys` end to end: the real binary, a real database, real git repositories.
//!
//! No agent is started here. Launching one needs a harness on `PATH`, which a CI runner has no
//! reason to have — what is tested instead is that everything up to the launch is right, and
//! that asking for a harness that is not there fails *before* anything is created.

use std::path::PathBuf;
use std::process::Command;

use yardsort_core::git::testing::git;
use yardsort_core::git::{normalize, Git};
use yardsort_core::projects::Projects;
use yardsort_core::store::Store;

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
        let output = Command::new(env!("CARGO_BIN_EXE_ys"))
            .args(["--data-dir", &self.data_dir.to_string_lossy()])
            .args(args)
            .env("YARDSORT_WORKTREE_ROOT", &self.worktree_root)
            .env("YARDSORT_NO_DAEMON", "1")
            .output()
            .expect("ys should run");
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

/// The workspaces Yardsort made, which is everything but each project's own checkout.
fn worktrees_in(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("a list")
        .iter()
        .filter(|w| w["kind"] == "worktree")
        .collect()
}

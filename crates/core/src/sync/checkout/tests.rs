//! Checking out another computer's branch, with real repositories: a bare remote, the other
//! computer's clone that pushes, and this one's clone, whose store has the other's record.

use std::path::{Path, PathBuf};

use remote_proto::sync::{RecordKind, SyncId, SyncRecord};
use remote_proto::HostId;

use super::*;
use crate::git::{normalize, testing::git};
use crate::projects::Projects;
use crate::settings::WorkspaceSettings;
use crate::store::{RemoteHostRow, Store};
use crate::sync::{ProjectBody, WorkspaceBody};

const SYNCED: &str = "4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e";
const UNPUSHED: &str = "5c4d3e2f-1a0b-4c9d-8e7f-6a5b4c3d2e1f";

struct Here {
    env: crate::env::ShellEnv,
    store: Store,
    git: Git,
    elsewhere: PathBuf,
    root: PathBuf,
    worktrees: PathBuf,
    project: String,
    settings: WorkspaceSettings,
    _dirs: tempfile::TempDir,
}

impl Here {
    fn workspaces(&self) -> Workspaces<'_> {
        Workspaces {
            env: &self.env,
            store: &self.store,
            git: &self.git,
            worktree_root: &self.worktrees,
            settings: &self.settings,
        }
    }

    fn run(&self, at: &Path, args: &[&str]) -> String {
        self.git.run(at, args).unwrap()
    }

    /// A commit on `branch` at the other computer, pushed.
    fn push_from_elsewhere(&self, branch: &str, file: &str) -> String {
        let at = &self.elsewhere;
        self.run(at, &["checkout", "-q", "-B", branch]);
        std::fs::write(at.join(file), file).unwrap();
        self.run(at, &["add", file]);
        self.run(at, &["commit", "-q", "-m", file]);
        self.run(at, &["push", "-q", "origin", branch]);
        self.run(at, &["rev-parse", "HEAD"])
    }

    fn head(&self, branch: &str) -> String {
        self.run(&self.root, &["rev-parse", branch])
    }

    fn worktree_count(&self) -> usize {
        self.run(&self.root, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count()
    }
}

/// The other computer's workspace `id`, on `branch`, as its record reaches this store.
fn record(project: &SyncId, id: &str, branch: &str, revision: u64) -> SyncRecord {
    let body = WorkspaceBody {
        name: "fix-login".into(),
        branch: Some(branch.into()),
        base_branch: Some("main".into()),
        archived: false,
        tasks: Vec::new(),
        origin: "host-there".into(),
        host: "Build box".into(),
    };
    SyncRecord {
        kind: RecordKind::Workspace,
        id: SyncId::new(id).unwrap(),
        project: project.clone(),
        revision,
        schema: 1,
        body: Some(body.to_value()),
        revoked: false,
        deleted: false,
        host: HostId::new("host-there").unwrap(),
        changed_at: 1,
        cursor: revision,
    }
}

fn here() -> Here {
    let dirs = tempfile::tempdir().unwrap();
    let git = git();
    let base = normalize(dirs.path());
    let remote = base.join("remote.git");
    let elsewhere = base.join("elsewhere");
    let root = base.join("here");
    git.run(
        &base,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    )
    .unwrap();
    git.run(
        &base,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            elsewhere.to_str().unwrap(),
        ],
    )
    .unwrap();
    git.run(&elsewhere, &["checkout", "-q", "-b", "main"])
        .unwrap();
    git.run(
        &elsewhere,
        &["commit", "-q", "--allow-empty", "-m", "first"],
    )
    .unwrap();
    git.run(&elsewhere, &["push", "-q", "origin", "main"])
        .unwrap();
    git.run(
        &base,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    )
    .unwrap();

    let store = Store::in_memory();
    store
        .set_remote_host(&RemoteHostRow {
            server_url: "https://server.example".into(),
            server_id: "server".into(),
            host_id: "host-here".into(),
            device_id: "device".into(),
            name: "Workstation".into(),
            enabled_at: 1,
        })
        .unwrap();
    let project = Projects {
        store: &store,
        git: &git,
    }
    .open(&root, false)
    .unwrap()
    .project
    .id;
    store
        .turn_on_sync(
            &project,
            ProjectBody {
                name: "widgets".into(),
                remote: None,
            }
            .to_value(),
            1,
        )
        .unwrap();
    let synced = store.synced_project(&project).unwrap().unwrap().sync_id;
    store
        .take_sync_records(
            &[
                record(&synced, SYNCED, "ys/fix-login", 1),
                record(&synced, UNPUSHED, "ys/never-pushed", 1),
            ],
            Some(2),
        )
        .unwrap();
    let worktrees = base.join("worktrees");
    std::fs::create_dir_all(&worktrees).unwrap();
    Here {
        env: crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        },
        store,
        git,
        elsewhere,
        root,
        worktrees,
        project,
        settings: WorkspaceSettings::default(),
        _dirs: dirs,
    }
}

#[test]
fn another_computers_pushed_branch_is_checked_out_here_as_a_workspace_of_its_own() {
    let fx = here();
    let pushed = fx.push_from_elsewhere("ys/fix-login", "login.txt");
    let (row, branch) = check_out_here(&fx.workspaces(), &fx.project, SYNCED).unwrap();
    assert_eq!(branch, Branch::Made);
    assert_eq!(row.branch.as_deref(), Some("ys/fix-login"));
    assert!(Path::new(&row.path).join("login.txt").exists());
    assert_eq!(fx.head("ys/fix-login"), pushed);
    assert_eq!(
        fx.run(
            &fx.root,
            &["rev-parse", "--abbrev-ref", "ys/fix-login@{upstream}"]
        ),
        "origin/ys/fix-login",
        "it tracks the remote's"
    );
    // The other computer's record is untouched: still listed, still its.
    let listed = fx.store.synced_workspaces(&fx.project).unwrap();
    assert!(listed
        .iter()
        .any(|w| w.id == SYNCED && w.host == "Build box"));

    let again = check_out_here(&fx.workspaces(), &fx.project, SYNCED).unwrap_err();
    assert_eq!(again.code, "branch_here", "{again:?}");
}

#[test]
fn a_branch_here_with_work_the_remote_has_not_is_refused_and_left_as_it_is() {
    let fx = here();
    fx.push_from_elsewhere("ys/fix-login", "login.txt");
    fx.run(&fx.root, &["fetch", "-q", "origin"]);
    fx.run(
        &fx.root,
        &[
            "checkout",
            "-q",
            "-b",
            "ys/fix-login",
            "origin/ys/fix-login",
        ],
    );
    fx.run(&fx.root, &["commit", "-q", "--allow-empty", "-m", "mine"]);
    fx.run(&fx.root, &["checkout", "-q", "main"]);
    let mine = fx.head("ys/fix-login");
    let worktrees = fx.worktree_count();

    let refused = check_out_here(&fx.workspaces(), &fx.project, SYNCED).unwrap_err();
    assert_eq!(refused.code, "branch_diverged", "{refused:?}");
    assert!(
        refused.message.contains("1 commit that origin's has not"),
        "{}",
        refused.message
    );
    assert_eq!(fx.head("ys/fix-login"), mine, "not moved");
    assert_eq!(fx.worktree_count(), worktrees, "no worktree made");
    assert!(fx
        .store
        .workspaces()
        .unwrap()
        .iter()
        .all(|w| w.branch.as_deref() != Some("ys/fix-login")));
}

#[test]
fn a_branch_here_that_is_only_behind_is_moved_up_to_the_remotes() {
    let fx = here();
    fx.push_from_elsewhere("ys/fix-login", "login.txt");
    fx.run(&fx.root, &["fetch", "-q", "origin"]);
    fx.run(
        &fx.root,
        &["branch", "-q", "ys/fix-login", "origin/ys/fix-login"],
    );
    let newer = fx.push_from_elsewhere("ys/fix-login", "more.txt");

    let (row, branch) = check_out_here(&fx.workspaces(), &fx.project, SYNCED).unwrap();
    assert_eq!(branch, Branch::MovedUp);
    assert_eq!(fx.head("ys/fix-login"), newer);
    assert!(Path::new(&row.path).join("more.txt").exists());
}

#[test]
fn a_branch_never_pushed_is_refused_with_nothing_changed() {
    let fx = here();
    let worktrees = fx.worktree_count();
    let refused = check_out_here(&fx.workspaces(), &fx.project, UNPUSHED).unwrap_err();
    assert_eq!(refused.code, "branch_not_pushed", "{refused:?}");
    assert!(
        refused.message.contains("Push it from Build box first"),
        "{}",
        refused.message
    );
    assert!(!fx.git.branch_exists(&fx.root, "ys/never-pushed").unwrap());
    assert_eq!(fx.worktree_count(), worktrees);
}

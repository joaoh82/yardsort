//! The Floor Manager: one agent that sees every project, in a folder of its own.
//!
//! It is an ordinary harness in an ordinary terminal — whichever agent the user picks — started
//! in `<data dir>/coordinator`, not in any project's worktree. What makes it a coordinator is
//! what it is told and what it can reach:
//!
//! - **Told:** that folder holds `AGENTS.md`, the instructions every harness Yardsort ships reads,
//!   and a `CLAUDE.md` that points at it. They are written afresh at every start, and again when
//!   the permission setting changes, so they describe the level that is actually in force.
//! - **Reach:** `ys`, the command line, which reads the same database the app does. Its `PATH`
//!   begins with the folder this copy's own `ys` is in, and `YARDSORT_DATA_DIR` names this
//!   profile, so `ys` answers about the projects the window shows and nobody else's.
//!   `YARDSORT_COORDINATOR` is set too. While it is set, `ys` enforces the permission level from
//!   settings: it is not enough that the instructions say where the lines are.
//!
//! Nothing here reads what it prints: the window shows its terminal, and the user talks to it
//! there. It has no workspace, no session record and no activity run — it is not work on a
//! project, it is the one who asks about it.

use std::path::{Path, PathBuf};

use pty_host::{SessionInfo, SessionState, TermSize, TerminalHost};

use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::launch::{resolve_launch, HarnessRequest, Launch, Launcher, HARNESS_LABEL};

mod confirm;
mod level;

pub use confirm::{answer, ask, pending, CoordinatorConfirmation};
pub use level::{decide, instructions, Action, Decision, FloorManagerLevel};

/// Label on the coordinator's PTY session. Its value is the harness it runs.
pub const LABEL: &str = "coordinator";

/// Its folder, inside the profile's data directory.
pub const DIR: &str = "coordinator";

/// Set on the Floor Manager's process. `ys` enforces the permission level only when this is
/// present. The value is ignored: the level is read from settings on every command, so changing
/// it applies to the next command without a restart. Unsetting it, or running something that is
/// not `ys`, is outside the gate — the guide says so, and it is not a sandbox.
pub const ENV: &str = "YARDSORT_COORDINATOR";

/// What `CLAUDE.md` says: Claude Code reads that file, not `AGENTS.md`, and `@` imports one.
const CLAUDE_MD: &str = "@AGENTS.md\n";

/// The coordinator's folder for this profile.
pub fn home(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// Make the folder and write the instructions for `level` into it, replacing whatever an earlier
/// version or an earlier level wrote. Anything else the user or the agent put there is left alone.
pub fn prepare(data_dir: &Path, level: FloorManagerLevel) -> std::io::Result<PathBuf> {
    let dir = home(data_dir);
    std::fs::create_dir_all(&dir)?;
    write_instructions(&dir, level)?;
    Ok(dir)
}

/// Rewrite the instructions when the folder already exists, so saving the setting updates the
/// file at once. A Floor Manager that has not been started yet has no folder, and the next start
/// writes the level then. A running one has already read the old file; it reads this one the
/// next time it starts. The gate itself does not wait for that: `ys` reads the setting.
pub fn rewrite_if_present(data_dir: &Path, level: FloorManagerLevel) -> std::io::Result<()> {
    let dir = home(data_dir);
    if !dir.is_dir() {
        return Ok(());
    }
    write_instructions(&dir, level)
}

fn write_instructions(dir: &Path, level: FloorManagerLevel) -> std::io::Result<()> {
    std::fs::write(dir.join("AGENTS.md"), instructions(level))?;
    std::fs::write(dir.join("CLAUDE.md"), CLAUDE_MD)?;
    Ok(())
}

/// The coordinator the daemon holds, if any: the running one, or else the last to end, so the
/// window can show what it said before it stopped.
pub fn current(host: &dyn TerminalHost) -> Option<SessionInfo> {
    let mut found: Vec<SessionInfo> = host
        .list()
        .into_iter()
        .filter(|session| session.labels.contains_key(LABEL))
        .collect();
    let running = found
        .iter()
        .position(|session| matches!(session.state, SessionState::Running));
    match running {
        Some(index) => Some(found.swap_remove(index)),
        None => found.pop(),
    }
}

/// The variables the coordinator gets on top of the user's environment: the profile `ys` is to
/// read, and a `PATH` that finds this copy's `ys` before any other.
///
/// `ys_dir` is the folder holding the `ys` that came with the app. `None` — a development build
/// before `ys` was built — leaves `PATH` as it is, and the coordinator finds whatever `ys` the
/// user installed, if any.
pub fn environment(
    env: &ShellEnv,
    data_dir: &Path,
    ys_dir: Option<&Path>,
) -> Vec<(String, String)> {
    let mut vars = vec![
        (
            "YARDSORT_DATA_DIR".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        ),
        // Presence is the marker. The level is not put here: `ys` reads it from settings, and
        // a value the agent can edit would be a level the agent can edit.
        (ENV.to_owned(), "1".to_owned()),
    ];
    if let Some(dir) = ys_dir {
        // Windows spells it `Path` as often as `PATH`, and a second spelling would be a second
        // variable to the child: use the one the environment already has.
        let key = env
            .vars
            .keys()
            .find(|name| name.eq_ignore_ascii_case("PATH"))
            .cloned()
            .unwrap_or_else(|| "PATH".to_owned());
        let mut dirs = vec![dir.to_path_buf()];
        if let Some(existing) = env.get(&key) {
            dirs.extend(std::env::split_paths(existing).filter(|entry| entry != dir));
        }
        if let Ok(joined) = std::env::join_paths(dirs) {
            vars.push((key, joined.to_string_lossy().into_owned()));
        }
    }
    vars
}

impl Launcher<'_> {
    /// Start the coordinator with `harness`, or hand back the one already running: there is
    /// only ever one, whichever harness it runs. An ended one is forgotten first.
    ///
    /// The request's first message, if any, is sent like any prompt; the window sends none, and
    /// the user types the first one.
    pub fn coordinator(
        &self,
        harness: HarnessRequest,
        ys_dir: Option<&Path>,
        size: TermSize,
        level: FloorManagerLevel,
    ) -> IpcResult<SessionInfo> {
        for session in self
            .host
            .list()
            .into_iter()
            .filter(|session| session.labels.contains_key(LABEL))
        {
            match session.state {
                SessionState::Running => return Ok(session),
                SessionState::Exited { .. } => {
                    let _ = self.host.remove(&session.id);
                }
            }
        }
        let home = prepare(self.data_dir, level).map_err(|e| {
            IpcError::new(
                "coordinator_folder",
                format!(
                    "Cannot write the Floor Manager's folder, {}: {e}",
                    home(self.data_dir).display()
                ),
            )
        })?;
        let mut resolved = resolve_launch(Launch::Harness(harness), self.harnesses)?;
        // Not a workspace's conversation: there is nothing to record it against, and the record
        // a harness launch carries would make it look like one.
        resolved.record = None;
        let harness_id = resolved
            .labels
            .get(HARNESS_LABEL)
            .cloned()
            .unwrap_or_default();
        resolved.labels.insert(LABEL.to_owned(), harness_id);
        resolved
            .env
            .extend(environment(self.env, self.data_dir, ys_dir));
        self.start(resolved, Some(home.to_string_lossy().into_owned()), size)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::env::EnvSource;

    // Only the test that starts a real process in a real PTY needs these, and it runs on Unix.
    #[cfg(unix)]
    use {
        crate::harness::HarnessOverride,
        crate::settings::ActivitySettings,
        crate::store::Store,
        std::sync::Arc,
        std::time::{Duration, Instant},
    };

    fn env_with(vars: &[(&str, &str)]) -> ShellEnv {
        ShellEnv {
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            source: EnvSource::Process,
            warning: None,
        }
    }

    #[test]
    fn the_instructions_are_written_for_every_harness_and_rewritten_each_time() {
        let data = tempfile::tempdir().unwrap();
        let dir = prepare(data.path(), FloorManagerLevel::Ask).unwrap();
        assert_eq!(dir, data.path().join("coordinator"));
        assert_eq!(
            std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
            instructions(FloorManagerLevel::Ask)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
            "@AGENTS.md\n"
        );

        // An old version's words are replaced; a note of the user's own is not touched.
        // A new level replaces the wording too, which is what saving the setting does.
        std::fs::write(dir.join("AGENTS.md"), "stale").unwrap();
        std::fs::write(dir.join("notes.md"), "mine").unwrap();
        prepare(data.path(), FloorManagerLevel::ReadOnly).unwrap();
        let written = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert_eq!(written, instructions(FloorManagerLevel::ReadOnly));
        assert!(written.contains("## Read only"), "{written}");
        assert_eq!(
            std::fs::read_to_string(dir.join("notes.md")).unwrap(),
            "mine"
        );
        // No folder yet: saving the setting does not create one. The next start does.
        let fresh = tempfile::tempdir().unwrap();
        rewrite_if_present(fresh.path(), FloorManagerLevel::Trusted).unwrap();
        assert!(!home(fresh.path()).exists());
    }

    #[test]
    fn ys_comes_first_on_the_path_and_the_profile_is_named() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let (ys, other) = if cfg!(windows) {
            (r"C:\Yardsort", r"C:\Windows")
        } else {
            ("/opt/yardsort", "/usr/bin")
        };
        let path = format!("{other}{sep}{ys}");
        let env = env_with(&[("Path", &path)]);
        let vars = environment(&env, Path::new("/tmp/profile"), Some(Path::new(ys)));
        assert_eq!(vars[0].0, "YARDSORT_DATA_DIR");
        assert_eq!(vars[0].1, "/tmp/profile");
        // Present, and not the level: the agent must not be able to edit its permission by
        // editing the value.
        assert_eq!(vars[1], (ENV.to_owned(), "1".to_owned()));
        // The spelling the environment has, and the folder once, at the front.
        assert_eq!(vars[2], ("Path".to_owned(), format!("{ys}{sep}{other}")));

        // No `ys` of its own: `PATH` is left as it is. The marker is still set.
        let vars = environment(&env, Path::new("/tmp/profile"), None);
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[1].0, ENV);
    }

    /// A harness that is a shell script, so a real process runs in a real PTY.
    #[cfg(unix)]
    fn script_harness(dir: &Path, body: &str) -> HarnessOverride {
        let path = dir.join("fake-agent");
        crate::program::testing::executable(&path, format!("#!/bin/sh\n{body}\n"));
        HarnessOverride {
            id: "fake".into(),
            builtin: Some(false),
            command: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        }
    }

    #[cfg(unix)]
    fn request() -> HarnessRequest {
        HarnessRequest {
            id: "fake".into(),
            model: None,
            effort: None,
            prompt: None,
            handoff: false,
            skip_memory: false,
            quiet_memory: false,
            skip_context: false,
            fork_packet: None,
        }
    }

    #[cfg(unix)]
    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_coordinator_runs_in_its_folder_reads_this_profile_and_is_only_ever_one() {
        let data = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let ys_dir = tempfile::tempdir().unwrap();
        let out = data.path().join("seen");
        let harnesses = vec![script_harness(
            bin.path(),
            &format!(
                "pwd > '{0}'; echo \"$YARDSORT_DATA_DIR\" >> '{0}'; echo \"$PATH\" >> '{0}'; \
                 echo \"$YARDSORT_COORDINATOR\" >> '{0}'; sleep 30",
                out.display()
            ),
        )];
        let store = Store::open(&data.path().join("yardsort.db")).unwrap();
        let host = pty_host::PtyHost::new(Arc::new(|_| {}));
        let env = Arc::new(ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        });
        let activity = ActivitySettings::default();
        let launcher = Launcher {
            store: &store,
            host: &host,
            env: &env,
            harnesses: &harnesses,
            activity: &activity,
            launched_by: crate::activity::LaunchedBy::App,
            data_dir: data.path(),
        };
        let size = TermSize { cols: 80, rows: 24 };

        let first = launcher
            .coordinator(request(), Some(ys_dir.path()), size, FloorManagerLevel::Ask)
            .unwrap();
        assert_eq!(first.labels.get(LABEL).map(String::as_str), Some("fake"));
        assert!(!first.labels.contains_key(crate::launch::WORKSPACE_LABEL));
        wait_until(|| std::fs::read_to_string(&out).is_ok_and(|seen| seen.lines().count() >= 4));
        let seen = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = seen.lines().collect();
        let home = std::fs::canonicalize(home(data.path())).unwrap();
        assert_eq!(std::fs::canonicalize(lines[0]).unwrap(), home);
        assert_eq!(lines[1], data.path().to_string_lossy());
        assert!(
            lines[2].starts_with(&*ys_dir.path().to_string_lossy()),
            "ys first on PATH: {}",
            lines[2]
        );
        assert_eq!(
            lines[3], "1",
            "the gate's marker, not a level the agent can edit"
        );
        assert!(home.join("AGENTS.md").is_file());
        // Not a workspace's conversation, so nothing was recorded as one.
        assert!(store.workspaces().unwrap().is_empty());

        // Asked again while it runs: the same one, whatever harness is asked for.
        let again = launcher
            .coordinator(
                request(),
                Some(ys_dir.path()),
                size,
                FloorManagerLevel::Trusted,
            )
            .unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(current(&host).map(|s| s.id), Some(first.id.clone()));

        // Once it has ended it is still what `current` shows, until a new one replaces it.
        host.kill(&first.id).unwrap();
        wait_until(|| {
            matches!(
                host.info(&first.id).map(|s| s.state),
                Ok(SessionState::Exited { .. })
            )
        });
        assert_eq!(current(&host).map(|s| s.id), Some(first.id.clone()));
        let next = launcher
            .coordinator(request(), Some(ys_dir.path()), size, FloorManagerLevel::Ask)
            .unwrap();
        assert_ne!(next.id, first.id);
        assert!(host.info(&first.id).is_err(), "the ended one is forgotten");
        assert_eq!(current(&host).map(|s| s.id), Some(next.id.clone()));
        host.kill(&next.id).unwrap();
    }
}

//! Explicit, locally stored project automation. Repository files never enable commands.
use std::fs::{self, OpenOptions};
use std::path::{Component, Path};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::program::Program;
use crate::store::{Store, WorkspaceRow};

#[derive(Debug, Clone, Default, Deserialize, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAutomation {
    pub copy_files: Vec<String>,
    pub setup: Option<ProjectCommand>,
    pub run: Option<ProjectCommand>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Type)]
pub struct ProjectCommand {
    pub program: String,
    pub args: Vec<String>,
}

fn invalid(message: impl Into<String>) -> IpcError {
    IpcError::new("project_automation", message)
}

impl ProjectAutomation {
    pub fn load(store: &Store, project_id: &str) -> IpcResult<Self> {
        if store.project(project_id)?.is_none() {
            return Err(invalid("That project no longer exists."));
        }
        let config = store.project_automation(project_id)?;
        config.map_or_else(
            || Ok(Self::default()),
            |json| {
                serde_json::from_str(&json)
                    .map_err(|e| invalid(format!("Cannot read project settings: {e}")))
            },
        )
    }

    pub fn save(&self, store: &Store, project_id: &str) -> IpcResult<()> {
        Self::load(store, project_id)?;
        self.validate()?;
        let json = serde_json::to_string(self).map_err(|e| invalid(e.to_string()))?;
        Ok(store.set_project_automation(project_id, &json)?)
    }

    pub fn validate(&self) -> IpcResult<()> {
        for file in &self.copy_files {
            relative_file(file)?;
        }
        for command in [&self.setup, &self.run].into_iter().flatten() {
            if command.program.trim().is_empty()
                || command.program.contains('\0')
                || command.args.iter().any(|arg| arg.contains('\0'))
            {
                return Err(invalid(
                    "Enter an executable and arguments without NUL characters.",
                ));
            }
        }
        Ok(())
    }

    /// Run only for a newly materialized worktree, never for local/imported workspaces.
    /// The caller records it first and retains it on errors: a script may have produced work.
    pub fn prepare(&self, root: &Path, workspace: &WorkspaceRow, env: &ShellEnv) -> IpcResult<()> {
        self.validate()?;
        let destination = Path::new(&workspace.path);
        for file in &self.copy_files {
            copy_file(root, destination, file)?;
        }
        if let Some(setup) = &self.setup {
            run_setup(setup, root, workspace, env)?;
        }
        Ok(())
    }
}

fn relative_file(file: &str) -> IpcResult<()> {
    // Apply the same rules on every OS, including Windows drive/UNC and alternate streams.
    if file.is_empty()
        || file.contains(['\\', ':', '\0'])
        || file.split('/').any(|part| {
            part.is_empty()
                || part.ends_with([' ', '.'])
                || part == "."
                || part == ".."
                || part.eq_ignore_ascii_case(".git")
        })
        || Path::new(file)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "Use a relative file path with / separators, without .git or parent traversal: {file}"
        )));
    }
    Ok(())
}

fn io(error: std::io::Error) -> IpcError {
    invalid(error.to_string())
}

fn copy_file(root: &Path, destination: &Path, file: &str) -> IpcResult<()> {
    relative_file(file)?;
    let mut source = root.to_path_buf();
    let parts: Vec<_> = file.split('/').collect();
    for part in &parts {
        source.push(part);
        if fs::symlink_metadata(&source)
            .map_err(io)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid(format!("Cannot copy symlinks: {file}")));
        }
    }
    if !source.is_file() {
        return Err(invalid(format!("Not a regular file: {file}")));
    }
    let mut target = destination.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        target.push(part);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(invalid(format!("Unsafe destination for {file}")))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&target).map_err(io)?
            }
            Err(e) => return Err(io(e)),
        }
    }
    target.push(parts[parts.len() - 1]);
    let mut input = fs::File::open(&source).map_err(io)?;
    // create_new refuses existing files, including dangling symlinks. Never overwrite work.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(&target).map_err(|e| {
        invalid(format!(
            "Cannot copy {file}: {e}. Existing files are never overwritten."
        ))
    })?;
    std::io::copy(&mut input, &mut output).map_err(io)?;
    output
        .set_permissions(input.metadata().map_err(io)?.permissions())
        .map_err(io)?;
    Ok(())
}

fn run_setup(
    command: &ProjectCommand,
    root: &Path,
    workspace: &WorkspaceRow,
    env: &ShellEnv,
) -> IpcResult<()> {
    let cwd = Path::new(&workspace.path);
    let program = env
        .find_program(&command.program, cwd)
        .ok_or_else(|| invalid(format!("Setup executable not found: {}", command.program)))?;
    // Linked worktrees have a private Git directory outside the checkout. Logs cannot be
    // staged with `git add -A`, and git removes them when the worktree is archived/deleted.
    let git_dir = crate::git::Git::new(env)?.run(cwd, &["rev-parse", "--absolute-git-dir"])?;
    let log_path = Path::new(&git_dir).join(format!("yardsort-setup-{}.log", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(&log_path).map_err(io)?;
    let mut child = Program::at(program, env)
        .command(cwd)
        .args(&command.args)
        .env("YARDSORT_PROJECT", root)
        .env("YARDSORT_WORKSPACE", cwd)
        .stdout(Stdio::from(log.try_clone().map_err(io)?))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(io)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(io)? {
            if status.success() {
                return Ok(());
            }
            return Err(invalid(format!(
                "Setup exited with {status}. Read {}.",
                log_path.display()
            )));
        }
        if started.elapsed() >= Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid(format!(
                "Setup exceeded 10 minutes. Read {}. Check for child processes before retrying.",
                log_path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cross_platform_escapes_and_git_metadata() {
        for file in [
            "../secret",
            "/etc/passwd",
            "C:/secret",
            "a\\b",
            ".git/config",
            ".GIT/config",
            ".git./config",
            "a/../../b",
            "",
            "a//b",
            "a/./b",
        ] {
            assert!(relative_file(file).is_err(), "{file}");
        }
        assert!(relative_file("config/.env.local").is_ok());
    }

    #[test]
    fn configuration_persists_and_remains_when_project_is_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let id;
        {
            let store = Store::open(&path).unwrap();
            id = store.add_project("demo", "/demo").unwrap().id;
            ProjectAutomation {
                copy_files: vec![".env".into()],
                ..Default::default()
            }
            .save(&store, &id)
            .unwrap();
            store.remove_project(&id, true).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(store
            .project_automation(&id)
            .unwrap()
            .unwrap()
            .contains(".env"));
    }

    #[cfg(unix)]
    #[test]
    fn copy_rejects_source_and_destination_symlinks() {
        use std::os::unix::fs::symlink;
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "private").unwrap();
        symlink(outside.path(), source.path().join("linked")).unwrap();
        assert!(copy_file(source.path(), target.path(), "linked/secret").is_err());
        fs::create_dir(source.path().join("config")).unwrap();
        fs::write(source.path().join("config/secret"), "new").unwrap();
        symlink(outside.path(), target.path().join("config")).unwrap();
        assert!(copy_file(source.path(), target.path(), "config/secret").is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("secret")).unwrap(),
            "private"
        );
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use crate::launch::Launcher;
    use pty_host::{PtyHost, TermSize};
    use std::sync::Arc;

    #[test]
    fn run_is_a_real_workspace_terminal_and_reuses_the_live_process() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let project = store
            .add_project("demo", &dir.path().to_string_lossy())
            .unwrap();
        let workspace = store.workspaces().unwrap().remove(0);
        let env = Arc::new(ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        });
        let host = PtyHost::new(Arc::new(|_| {}));
        let launcher = Launcher {
            store: &store,
            host: &host,
            env: &env,
            harnesses: &[],
        };
        assert!(launcher
            .project_run(&workspace.id, TermSize { cols: 80, rows: 24 })
            .is_err());
        ProjectAutomation {
            run: Some(ProjectCommand {
                program: if cfg!(windows) { "cmd.exe" } else { "sh" }.into(),
                args: vec![],
            }),
            ..Default::default()
        }
        .save(&store, &project.id)
        .unwrap();
        let first = launcher
            .project_run(&workspace.id, TermSize { cols: 80, rows: 24 })
            .unwrap();
        let second = launcher
            .project_run(&workspace.id, TermSize { cols: 80, rows: 24 })
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.labels.get("projectRun"), Some(&workspace.id));
        assert_eq!(first.labels.get("workspace"), Some(&workspace.id));
        assert_eq!(host.list().len(), 1);
        host.remove(&first.id).unwrap();
        let restarted = launcher
            .project_run(&workspace.id, TermSize { cols: 80, rows: 24 })
            .unwrap();
        assert_ne!(first.id, restarted.id);
        host.remove(&restarted.id).unwrap();
    }
}

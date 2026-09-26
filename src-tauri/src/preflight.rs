//! The first-run check: is everything Yardsort depends on actually here?
//!
//! Yardsort bundles nothing. It needs `git`, and at least one coding agent the user installed
//! themselves. When one is missing the app used to fail late and obscurely — a red line in the
//! sidebar, a composer with nobody to send to. This reports the state of things up front, with
//! what to do about it.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::env::ShellEnv;
use crate::error::IpcResult;
use crate::git::Git;
use crate::harness::{self, HarnessDef};
use crate::state::blocking;
use crate::terminal::EnvInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub git: GitStatus,
    pub harnesses: Vec<HarnessStatus>,
    pub env: EnvInfo,
    /// `linux`, `macos` or `windows`: install advice differs.
    pub os: String,
    /// The `ys` command. Optional: it never stands in the way of `ready`.
    pub ys: crate::ys::YsStatus,
    /// Nothing stands between the user and their first workspace.
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub path: Option<String>,
    /// As `git --version` prints it, e.g. `git version 2.55.0`.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessStatus {
    pub id: String,
    pub label: String,
    pub command: String,
    pub enabled: bool,
    /// Where the command was found; `None` if it is not installed (or not on `PATH`).
    pub path: Option<String>,
    /// How to get it. Known for the built-in harnesses only.
    pub install: Option<InstallHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstallHint {
    /// An install command for the current OS; shown for the user to copy, never executed.
    pub command: String,
    /// The project's own install instructions, for every other way.
    pub url: String,
}

/// Install advice for the built-in harnesses. Package names and links verified 2026-09-19
/// (OMP, Cursor and Pi: 2026-09-24); like the harness flags themselves they can drift, which is
/// why a link is always included.
fn install_hint(harness_id: &str) -> Option<InstallHint> {
    let (command, url) = match harness_id {
        "claude" => (
            "npm install -g @anthropic-ai/claude-code",
            "https://docs.claude.com/en/docs/claude-code/setup",
        ),
        "codex" => (
            "npm install -g @openai/codex",
            "https://github.com/openai/codex",
        ),
        "grok" => (
            "npm install -g @xai-official/grok",
            "https://www.npmjs.com/package/@xai-official/grok",
        ),
        "opencode" => ("npm install -g opencode-ai", "https://opencode.ai/docs"),
        "omp" => (
            if cfg!(windows) {
                "irm https://omp.sh/install.ps1 | iex"
            } else {
                "curl -fsSL https://omp.sh/install | sh"
            },
            "https://omp.sh/docs/quickstart",
        ),
        "cursor" => (
            if cfg!(windows) {
                "irm 'https://cursor.com/install?win32=true' | iex"
            } else {
                "curl https://cursor.com/install -fsS | bash"
            },
            "https://cursor.com/docs/cli/installation",
        ),
        "pi" => (
            "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
            "https://pi.dev/",
        ),
        _ => return None,
    };
    Some(InstallHint {
        command: command.to_owned(),
        url: url.to_owned(),
    })
}

pub fn check(env: &ShellEnv, harnesses: Vec<HarnessDef>) -> Preflight {
    let cwd = std::env::current_dir().unwrap_or_default();
    let locate = |program: &str| {
        env.find_program(program, &cwd)
            .map(|path| path.to_string_lossy().into_owned())
    };

    let git = GitStatus {
        path: locate("git"),
        version: Git::new(env).ok().and_then(|git| git.version().ok()),
    };
    let harnesses: Vec<_> = harnesses
        .into_iter()
        .map(|def| HarnessStatus {
            path: locate(&def.command),
            install: install_hint(&def.id),
            id: def.id,
            label: def.label,
            command: def.command,
            enabled: def.enabled,
        })
        .collect();

    Preflight {
        ready: git.path.is_some() && harnesses.iter().any(|h| h.enabled && h.path.is_some()),
        git,
        harnesses,
        env: EnvInfo::from(env),
        os: std::env::consts::OS.to_owned(),
        ys: crate::ys::status(&crate::ys::Layout::detect(env), env),
    }
}

/// With `reload`, the login shell is asked again first — for "I just installed it, look again".
#[tauri::command]
#[specta::specta]
pub async fn preflight(app: AppHandle, reload: bool) -> IpcResult<Preflight> {
    blocking(app, move |state| {
        let env = if reload {
            state.reload_env()
        } else {
            state.env()
        };
        let harnesses = harness::resolve_all(&state.settings.get().harnesses)
            .into_iter()
            .map(|resolved| resolved.def)
            .collect();
        Ok(check(&env, harnesses))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvSource;

    fn env_with_path(path: &str) -> ShellEnv {
        let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        vars.retain(|name, _| !name.eq_ignore_ascii_case("PATH"));
        vars.insert("PATH".into(), path.into());
        ShellEnv {
            vars,
            source: EnvSource::Process,
            warning: None,
        }
    }

    fn real_env() -> ShellEnv {
        ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        }
    }

    #[test]
    fn a_bare_machine_is_told_exactly_what_is_missing() {
        let empty = tempfile::tempdir().unwrap();
        let report = check(
            &env_with_path(&empty.path().to_string_lossy()),
            harness::builtin(),
        );

        assert!(!report.ready);
        assert_eq!(
            report.git,
            GitStatus {
                path: None,
                version: None
            }
        );
        assert!(report.harnesses.iter().all(|h| h.path.is_none()));
        for harness in &report.harnesses {
            let hint = harness
                .install
                .as_ref()
                .expect("every built-in says how to get it");
            assert!(!hint.command.is_empty(), "{}", hint.command);
            assert!(hint.url.starts_with("https://"));
        }
    }

    #[test]
    fn new_agents_have_install_advice_for_the_current_platform() {
        let omp = install_hint("omp").unwrap();
        let cursor = install_hint("cursor").unwrap();
        if cfg!(windows) {
            assert_eq!(omp.command, "irm https://omp.sh/install.ps1 | iex");
            assert_eq!(
                cursor.command,
                "irm 'https://cursor.com/install?win32=true' | iex"
            );
        } else {
            assert_eq!(omp.command, "curl -fsSL https://omp.sh/install | sh");
            assert_eq!(
                cursor.command,
                "curl https://cursor.com/install -fsS | bash"
            );
        }
        assert_eq!(
            install_hint("pi").unwrap().command,
            "npm install -g --ignore-scripts @earendil-works/pi-coding-agent"
        );
        assert_eq!(omp.url, "https://omp.sh/docs/quickstart");
        assert_eq!(cursor.url, "https://cursor.com/docs/cli/installation");
    }

    #[test]
    fn git_is_found_with_its_version_where_the_tests_run() {
        let report = check(&real_env(), vec![]);
        assert!(report.git.path.is_some());
        assert!(report.git.version.unwrap().starts_with("git version "));
        assert!(
            !report.ready,
            "git alone is not enough: there is no agent to run"
        );
    }

    #[test]
    fn one_installed_enabled_agent_is_enough_and_a_disabled_one_does_not_count() {
        // Any program that certainly exists stands in for an agent.
        let mut agent = HarnessDef::custom("stand-in");
        agent.command = if cfg!(windows) { "cmd.exe" } else { "sh" }.into();
        assert!(check(&real_env(), vec![agent.clone()]).ready);

        agent.enabled = false;
        let report = check(&real_env(), vec![agent]);
        assert!(!report.ready);
        assert!(
            report.harnesses[0].path.is_some(),
            "still reported as installed"
        );
        assert_eq!(
            report.harnesses[0].install, None,
            "no advice for harnesses we do not know"
        );
    }
}

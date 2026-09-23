//! Having a model write a commit message or a pull request — see [`yardsort_core::draft`] for
//! why this exists at all and why Assist cannot do it.
//!
//! Two backends, tried in that order:
//!
//! 1. **The agent the workspace already uses**, in its non-interactive mode. It is installed,
//!    logged in and paid for, and it is the one the user chose for this work.
//! 2. **The user's own Anthropic API key**, when no configured harness can write.
//!
//! Nothing runs until the button is pressed, and what is sent is the diff — the same diff the
//! user is looking at — plus the workspace's task when there is one.

pub mod anthropic;
pub mod commands;

use std::path::Path;

use serde::Serialize;
use specta::Type;
use yardsort_core::draft::{self, Want};

use crate::error::{IpcError, IpcResult};
use crate::harness::HarnessDef;
use crate::program::Program;

/// How long an agent gets to write a few lines before we give up on it. Generous: a cold agent
/// may authenticate, load a project and think before it says anything.
const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Who would do the writing, and whether anyone can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DraftStatus {
    /// The setting: whether the user wants to be offered this at all.
    pub enabled: bool,
    /// The button is actually shown — switched on *and* somebody able to answer.
    pub available: bool,
    /// The harness that would write, when one can.
    pub harness: Option<String>,
    /// An Anthropic key is in force, so drafting works even with no agent able to write.
    pub key: bool,
    /// The model that key would ask for.
    pub model: String,
    /// Why nothing can write, when nothing can.
    pub problem: Option<String>,
}

/// Run an agent in its non-interactive mode and take what it prints.
///
/// The agent runs **in the workspace**, which is what makes its answer worth having: it can see
/// the repository it is describing. Output is read from stdout; nothing here attaches a
/// terminal, so this is not the "never parse agent output" rule — that one is about deriving
/// status from an interactive PTY, and this is an ordinary subprocess with an ordinary pipe.
pub fn ask_agent(
    program: &Program,
    harness: &HarnessDef,
    root: &Path,
    prompt: &str,
) -> IpcResult<String> {
    let args: Vec<String> = harness
        .base_args
        .iter()
        .chain(harness.write_args.iter())
        .map(|arg| arg.replace("{prompt}", prompt))
        .collect();

    let mut child = program
        .command(root)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            IpcError::new(
                "draft_agent_failed",
                format!("Could not run {}: {error}", harness.command),
            )
        })?;

    let output = match wait_with_timeout(&mut child, AGENT_TIMEOUT) {
        Some(output) => output.map_err(|error| {
            IpcError::new(
                "draft_agent_failed",
                format!("{} did not finish: {error}", harness.label),
            )
        })?,
        None => {
            let _ = child.kill();
            return Err(IpcError::new(
                "draft_agent_timeout",
                format!(
                    "{} took longer than {} seconds to answer.",
                    harness.label,
                    AGENT_TIMEOUT.as_secs()
                ),
            ));
        }
    };

    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(IpcError::new(
            "draft_agent_failed",
            match said.is_empty() {
                true => format!("{} exited without writing anything.", harness.label),
                false => format!("{} said: {said}", harness.label),
            },
        ));
    }

    let written = draft::tidy(&String::from_utf8_lossy(&output.stdout));
    if written.is_empty() {
        return Err(IpcError::new(
            "draft_agent_empty",
            format!("{} printed nothing to write with.", harness.label),
        ));
    }
    Ok(written)
}

/// Wait for a child, giving up after `limit`. `None` means it was still running.
///
/// `Child::wait_with_output` has no timeout, and an agent that hangs would hang the button with
/// it. The pipes are drained on threads of their own — a child that fills a pipe buffer and is
/// never read blocks for ever — while *this* thread does nothing but watch the clock. Killing
/// the child closes the pipes, which is what lets the readers finish afterwards.
fn wait_with_timeout(
    child: &mut std::process::Child,
    limit: std::time::Duration,
) -> Option<std::io::Result<std::process::Output>> {
    use std::io::Read;

    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buffer);
            }
            buffer
        })
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let collect = move || {
        (
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default(),
        )
    };

    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (stdout, stderr) = collect();
                return Some(Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                }));
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => return None,
            Err(error) => return Some(Err(error)),
        }
    }
}

/// The harness that should do the writing: the one this workspace last used if it can write,
/// otherwise the first enabled harness that can.
///
/// The workspace's own agent first because it is the one the user picked for this work, and
/// because it is the one already warm in this repository.
pub fn writer<'a>(harnesses: &'a [HarnessDef], preferred: Option<&str>) -> Option<&'a HarnessDef> {
    let usable = |def: &&HarnessDef| def.enabled && !def.write_args.is_empty();
    preferred
        .and_then(|id| harnesses.iter().find(|def| def.id == id).filter(usable))
        .or_else(|| harnesses.iter().find(usable))
}

/// The system prompt and user prompt for what is being asked.
pub fn ask(want: Want, task: Option<&str>, diff: &str) -> (&'static str, String) {
    (want.system(), draft::prompt(want, task, diff))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessDef;

    fn def(id: &str, enabled: bool, can_write: bool) -> HarnessDef {
        let mut def = HarnessDef::custom(id);
        def.enabled = enabled;
        def.write_args = if can_write {
            vec!["-p".into(), "{prompt}".into()]
        } else {
            vec![]
        };
        def
    }

    #[test]
    fn the_workspaces_own_agent_writes_when_it_can() {
        let all = vec![def("claude", true, true), def("codex", true, true)];
        assert_eq!(
            writer(&all, Some("codex")).map(|d| d.id.as_str()),
            Some("codex")
        );
        assert_eq!(writer(&all, None).map(|d| d.id.as_str()), Some("claude"));
    }

    #[test]
    fn an_agent_that_cannot_write_hands_over_to_one_that_can() {
        let all = vec![def("mine", true, false), def("claude", true, true)];
        assert_eq!(
            writer(&all, Some("mine")).map(|d| d.id.as_str()),
            Some("claude"),
            "the preferred one has no write args"
        );
    }

    #[test]
    fn a_disabled_agent_is_never_asked() {
        let all = vec![def("claude", false, true), def("codex", true, true)];
        assert_eq!(
            writer(&all, Some("claude")).map(|d| d.id.as_str()),
            Some("codex")
        );
        assert_eq!(writer(&[def("claude", false, true)], None), None);
    }

    #[test]
    fn nobody_writes_when_nobody_can() {
        assert_eq!(writer(&[def("mine", true, false)], Some("mine")), None);
        assert_eq!(writer(&[], None), None);
    }

    #[test]
    fn the_built_in_agents_all_know_their_non_interactive_mode() {
        for def in crate::harness::builtin() {
            assert!(
                !def.write_args.is_empty(),
                "{} should be able to write; its flags are verified against its own --help",
                def.id
            );
            assert!(
                def.write_args.iter().any(|arg| arg.contains("{prompt}")),
                "{} must be given the prompt somewhere",
                def.id
            );
        }
    }
}

//! Working out which session somebody meant.
//!
//! `ys attach yardsort` should do the obvious thing when that workspace has one agent in it, and
//! say what the choices are when it does not. Kept apart from the attaching itself because this
//! is the half that can be tested without a terminal.

use pty_host::SessionInfo;
use yardsort_core::store::{SessionRow, WorkspaceRow};

/// A session somebody could attach to: the live process, and the record it belongs to when it
/// has one. Shells have no record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub pty: String,
    pub workspace: Option<String>,
    pub label: String,
    /// The process is still there. `attach` wants only these; `logs` reads a finished one too.
    pub running: bool,
    pub size: pty_host::TermSize,
}

/// Why a name did not resolve to exactly one session.
#[derive(Debug, PartialEq, Eq)]
pub enum NoMatch {
    /// Nothing is running at all.
    Nothing,
    /// Nothing matched, but these exist.
    Unknown { choices: Vec<String> },
    /// Several matched.
    Several { choices: Vec<String> },
}

/// Every session the daemon holds, finished ones included, described for a person.
///
/// The daemon keeps a session after its process ends so the last screen stays readable, which is
/// what `logs` is for; callers that need a live process filter on [`Candidate::running`].
pub fn candidates(
    live: &[SessionInfo],
    workspaces: &[WorkspaceRow],
    records: &[SessionRow],
) -> Vec<Candidate> {
    live.iter()
        .map(|s| {
            let record = records
                .iter()
                .find(|r| r.pty_session_id.as_deref() == Some(&s.id.0));
            let workspace = record
                .and_then(|r| workspaces.iter().find(|w| w.id == r.workspace_id))
                .map(|w| w.name.clone());
            let label = match (&workspace, record) {
                (Some(name), Some(record)) => format!("{name} ({})", record.harness_id),
                // A shell, or a session whose record is gone: the program is all we can say.
                _ => s
                    .program
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("?")
                    .to_owned(),
            };
            Candidate {
                pty: s.id.0.clone(),
                workspace,
                label,
                running: s.state == pty_host::SessionState::Running,
                size: s.size,
            }
        })
        .collect()
}

/// Just the ones with a process still in them.
pub fn running(candidates: &[Candidate]) -> Vec<Candidate> {
    candidates.iter().filter(|c| c.running).cloned().collect()
}

/// Pick the one session `wanted` means, or say why not.
///
/// `None` is "whatever is running", which is unambiguous only when one thing is. A name matches a
/// workspace, or the start of a session id — ids are uuids, so a few characters are plenty and
/// that is what `session list` prints.
pub fn resolve(candidates: &[Candidate], wanted: Option<&str>) -> Result<Candidate, NoMatch> {
    let choices = || {
        candidates
            .iter()
            .map(|c| c.label.clone())
            .collect::<Vec<_>>()
    };
    if candidates.is_empty() {
        return Err(NoMatch::Nothing);
    }
    let Some(wanted) = wanted else {
        return match candidates {
            [one] => Ok(one.clone()),
            _ => Err(NoMatch::Several { choices: choices() }),
        };
    };

    let matched: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| {
            c.workspace
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                || c.pty.starts_with(wanted)
        })
        .collect();
    match matched.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(NoMatch::Unknown { choices: choices() }),
        several => Err(NoMatch::Several {
            choices: several.iter().map(|c| c.label.clone()).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(pty: &str, workspace: Option<&str>) -> Candidate {
        Candidate {
            pty: pty.to_owned(),
            workspace: workspace.map(str::to_owned),
            label: workspace.unwrap_or("bash").to_owned(),
            running: true,
            size: pty_host::TermSize { cols: 80, rows: 24 },
        }
    }

    fn finished(pty: &str, workspace: Option<&str>) -> Candidate {
        Candidate {
            running: false,
            ..candidate(pty, workspace)
        }
    }

    #[test]
    fn running_keeps_only_the_live_ones() {
        let mixed = [
            candidate("abc123", Some("alive")),
            finished("def456", Some("done")),
        ];
        let live = running(&mixed);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].pty, "abc123");
    }

    /// `logs` resolves against everything, so a finished session is reachable by name.
    #[test]
    fn a_finished_session_still_resolves_when_it_is_offered() {
        let both = [
            candidate("abc123", Some("alive")),
            finished("def456", Some("done")),
        ];
        assert_eq!(resolve(&both, Some("done")).unwrap().pty, "def456");
    }

    #[test]
    fn one_running_session_needs_no_name() {
        let only = [candidate("abc123", Some("login-bug"))];
        assert_eq!(resolve(&only, None).unwrap().pty, "abc123");
    }

    #[test]
    fn several_running_sessions_need_one() {
        let both = [
            candidate("abc123", Some("login-bug")),
            candidate("def456", Some("flaky-test")),
        ];
        let Err(NoMatch::Several { choices }) = resolve(&both, None) else {
            panic!("should have refused to choose");
        };
        assert_eq!(choices, ["login-bug", "flaky-test"]);
    }

    #[test]
    fn a_workspace_name_matches_whatever_its_case() {
        let both = [
            candidate("abc123", Some("login-bug")),
            candidate("def456", Some("flaky-test")),
        ];
        assert_eq!(resolve(&both, Some("Login-Bug")).unwrap().pty, "abc123");
    }

    #[test]
    fn the_start_of_a_session_id_is_enough() {
        let both = [
            candidate("abc123", Some("login-bug")),
            candidate("def456", Some("flaky-test")),
        ];
        assert_eq!(resolve(&both, Some("def")).unwrap().pty, "def456");
    }

    #[test]
    fn an_id_prefix_matching_two_sessions_is_refused_rather_than_guessed() {
        let both = [
            candidate("abc123", Some("one")),
            candidate("abc999", Some("two")),
        ];
        let Err(NoMatch::Several { choices }) = resolve(&both, Some("abc")) else {
            panic!("an ambiguous prefix must not pick one");
        };
        assert_eq!(choices, ["one", "two"]);
    }

    #[test]
    fn an_unknown_name_says_what_there_is() {
        let only = [candidate("abc123", Some("login-bug"))];
        assert_eq!(
            resolve(&only, Some("nope")),
            Err(NoMatch::Unknown {
                choices: vec!["login-bug".to_owned()]
            })
        );
    }

    #[test]
    fn nothing_running_is_its_own_answer() {
        assert_eq!(resolve(&[], Some("anything")), Err(NoMatch::Nothing));
        assert_eq!(resolve(&[], None), Err(NoMatch::Nothing));
    }

    /// A shell has no record and so no workspace; it can still be attached to by id.
    #[test]
    fn a_shell_is_reachable_by_id_even_with_no_workspace() {
        let mixed = [
            candidate("abc123", Some("login-bug")),
            candidate("f00d99", None),
        ];
        assert_eq!(resolve(&mixed, Some("f00d")).unwrap().pty, "f00d99");
    }
}

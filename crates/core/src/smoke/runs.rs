//! Asking for a run: the one door, whoever asks — `ys test run`, the app, a workflow.
//!
//! Everything that can be checked before anything happens is checked here, and a refusal
//! writes nothing: the test must be in the worktree's own folder and ready, the workspace must
//! be there, every input must be given and make sense, every `{{ env.NAME }}` must be set in
//! the project's test environment. What is not checked here is whether the user has seen the
//! file: a run of a file at a hash nobody approved is queued as `awaiting_approval`, and nothing
//! but the app moves it on from there.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{find, Action, BrowserAction, InputKind, Test, WaitTarget};
use crate::error::{IpcError, IpcResult};
use crate::store::{NewTestRun, Store, TestQueued};
use crate::template;

/// A request for a run.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub test_id: &'a str,
    pub workspace_id: &'a str,
    /// As given, by input id. Checked against what the test asks for.
    pub inputs: &'a BTreeMap<String, String>,
    /// `app`, `cli` or `workflow`.
    pub requested_by: &'a str,
    /// The agent's conversation record, when the request came from an agent's terminal.
    pub session_record_id: Option<&'a str>,
}

/// A run that was queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub id: String,
    /// The file at this hash has not been approved: the run waits for the user in the app.
    pub awaiting_approval: bool,
}

/// SHA-256 of the file's text, hex: what an approval is recorded against.
pub fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Queue a run and say whether it waits for approval.
pub fn queue(store: &Store, request: &Request<'_>) -> IpcResult<Queued> {
    let workspace = store
        .workspace(request.workspace_id)?
        .filter(|w| !w.forgotten)
        .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
    if workspace.archived {
        return Err(IpcError::new(
            "workspace_archived",
            format!(
                "`{}` is archived. Restore it before running a smoke test in it.",
                workspace.name
            ),
        ));
    }
    let worktree = std::path::Path::new(&workspace.path);
    let entry = find(worktree, request.test_id).ok_or_else(|| {
        let known: Vec<String> = super::catalog(worktree).into_iter().map(|e| e.id).collect();
        IpcError::new(
            "test_not_found",
            if known.is_empty() {
                format!(
                    "There is no smoke test `{}`: {} has none.",
                    request.test_id,
                    super::dir(worktree).display()
                )
            } else {
                format!(
                    "There is no smoke test `{}`. There are: {}.",
                    request.test_id,
                    known.join(", ")
                )
            },
        )
    })?;
    let Some(test) = entry.test.as_ref().filter(|_| entry.runnable()) else {
        return Err(IpcError::new(
            "test_invalid",
            format!(
                "`{}` has problems and cannot run. `ys test show {}` lists them.",
                entry.id, entry.id
            ),
        ));
    };
    if let Some(step) = test
        .all_steps()
        .find(|(_, step)| matches!(step.action, Action::Browser { .. }))
    {
        return Err(IpcError::new(
            "test_browser_unsupported",
            format!(
                "`{}` has a `browser` step (`{}`), and this version of Yardsort cannot run \
                 browser steps yet.",
                entry.id, step.1.id
            ),
        ));
    }
    let inputs =
        check_inputs(test, request.inputs).map_err(|why| IpcError::new("test_inputs", why))?;
    let set: BTreeSet<String> = store
        .project_test_env(&workspace.project_id)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let missing: Vec<String> = env_names(test)
        .into_iter()
        .filter(|name| !set.contains(name))
        .collect();
    if !missing.is_empty() {
        return Err(IpcError::new(
            "test_env_missing",
            format!(
                "`{}` uses {}, which {} not set in the project's test environment. Set {} in \
                 Yardsort, under the project's smoke test settings.",
                entry.id,
                missing
                    .iter()
                    .map(|name| format!("`{{{{ env.{name} }}}}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if missing.len() == 1 { "is" } else { "are" },
                if missing.len() == 1 { "it" } else { "them" },
            ),
        ));
    }

    let hash = content_hash(&entry.text);
    let approved = store.test_approved(&workspace.project_id, &entry.id, &hash)?;
    let id = uuid::Uuid::new_v4().to_string();
    let inputs_json = serde_json::to_string(&inputs).unwrap_or_else(|_| "{}".to_owned());
    let steps: Vec<(&str, &str, &str)> = test
        .all_steps()
        .map(|(phase, step)| (step.id.as_str(), phase.name(), step.action.name()))
        .collect();
    let queued = store.queue_test_run(&NewTestRun {
        id: &id,
        project_id: &workspace.project_id,
        workspace_id: &workspace.id,
        workspace_name: &workspace.name,
        test_id: &entry.id,
        test_name: &test.name,
        definition: &entry.text,
        content_hash: &hash,
        inputs: &inputs_json,
        status: if approved {
            "queued"
        } else {
            "awaiting_approval"
        },
        requested_by: request.requested_by,
        session_record_id: request.session_record_id,
        steps: &steps,
    })?;
    match queued {
        TestQueued::Queued => Ok(Queued {
            id,
            awaiting_approval: !approved,
        }),
        TestQueued::AlreadyRunning(other) => Err(IpcError::new(
            "test_already_running",
            format!(
                "`{}` is already going in `{}` (run {}). Wait for it, or cancel it.",
                entry.id,
                workspace.name,
                crate::memory::short_id(&other)
            ),
        )),
    }
}

/// The inputs a run gets: what was given, defaults for the rest. Refuses unknown inputs,
/// missing required ones and a choice that is not an option.
pub fn check_inputs(
    test: &Test,
    given: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    if let Some(unknown) = given
        .keys()
        .find(|key| !test.inputs.iter().any(|i| &i.id == *key))
    {
        let asks = if test.inputs.is_empty() {
            "It asks for none.".to_owned()
        } else {
            format!(
                "It asks for {}.",
                test.inputs
                    .iter()
                    .map(|i| format!("`{}`", i.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(format!("`{}` has no input `{unknown}`. {asks}", test.id));
    }
    let mut inputs = BTreeMap::new();
    let mut missing = Vec::new();
    for input in &test.inputs {
        let value = given
            .get(&input.id)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .or_else(|| input.default.clone());
        let Some(value) = value else {
            if input.required {
                missing.push(format!("`{}` ({})", input.id, input.label));
            }
            continue;
        };
        if input.kind == InputKind::Choice && !input.options.contains(&value) {
            return Err(format!(
                "`{value}` is not one of the answers to `{}`: {}.",
                input.id,
                input.options.join(", ")
            ));
        }
        inputs.insert(input.id.clone(), value);
    }
    if !missing.is_empty() {
        return Err(format!("`{}` needs {}.", test.id, missing.join(", ")));
    }
    Ok(inputs)
}

/// Every `{{ env.NAME }}` the test uses.
pub fn env_names(test: &Test) -> BTreeSet<String> {
    test.all_steps()
        .flat_map(|(_, step)| texts(&step.action))
        .flat_map(template::placeholders)
        .filter_map(|var| match var.path.as_slice() {
            [namespace, name] if namespace == "env" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Every piece of template text a step has. A `shell` line is not one: it gets no variables.
pub fn texts(action: &Action) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    match action {
        Action::Run {
            program,
            args,
            env,
            stdin,
            expect,
            ..
        } => {
            out.push(program);
            out.extend(args.iter().map(String::as_str));
            out.extend(env.values().map(String::as_str));
            out.extend(stdin.as_deref());
            out.extend(expect.stdout_contains.as_deref());
        }
        Action::Shell { env, expect, .. } => {
            out.extend(env.values().map(String::as_str));
            out.extend(expect.stdout_contains.as_deref());
        }
        Action::Start {
            program, args, env, ..
        } => {
            out.push(program);
            out.extend(args.iter().map(String::as_str));
            out.extend(env.values().map(String::as_str));
        }
        Action::WaitFor { target, .. } => match target {
            WaitTarget::Port { port } => out.push(port),
            WaitTarget::Url { url } => out.push(url),
            WaitTarget::File { file } => out.push(file),
            WaitTarget::Command { program, args } => {
                out.push(program);
                out.extend(args.iter().map(String::as_str));
            }
        },
        Action::Http {
            url,
            headers,
            body,
            json,
            expect,
            ..
        } => {
            out.push(url);
            out.extend(headers.values().map(String::as_str));
            out.extend(body.as_deref());
            out.extend(json.as_deref());
            out.extend(expect.json.as_deref());
            out.extend(expect.body_contains.as_deref());
            if let Some(header) = &expect.header {
                out.push(&header.contains);
            }
        }
        Action::Browser { actions, .. } => {
            for action in actions {
                match action {
                    BrowserAction::Goto { url } => out.push(url),
                    BrowserAction::Fill { text, .. } => out.push(text),
                    BrowserAction::Select { value, .. } => out.push(value),
                    BrowserAction::WaitFor { text, .. } => out.extend(text.as_deref()),
                    BrowserAction::ExpectText { text } => out.push(text),
                    BrowserAction::ExpectUrl { pattern } => out.push(pattern),
                    BrowserAction::ExpectTitle { title } => out.push(title),
                    BrowserAction::Click { .. }
                    | BrowserAction::Press { .. }
                    | BrowserAction::ExpectVisible { .. }
                    | BrowserAction::ExpectHidden { .. }
                    | BrowserAction::Screenshot { .. } => {}
                }
            }
        }
        Action::Manual { prompt } => out.push(prompt),
    }
    out
}

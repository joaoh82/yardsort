//! The drafting IPC surface: who can write, the Anthropic key, and the two things asked for.

use tauri::AppHandle;
use yardsort_core::draft::{self, Want};

use super::anthropic::{self, AnthropicError};
use super::{ask, ask_agent, writer, DraftStatus};
use crate::assist::key::{self, KeyStore};
use crate::changes::commands::workspace;
use crate::changes::{Changes, Scope};
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::harness::HarnessDef;
use crate::program::Program;
use crate::state::{blocking, AppState};

fn failed(error: AnthropicError) -> IpcError {
    IpcError::new(error.code(), error.to_string())
}

fn store() -> key::Keychain {
    key::Keychain(key::ANTHROPIC.0)
}

/// The Anthropic key in force, from the credential store or `ANTHROPIC_API_KEY`.
fn anthropic_key(state: &AppState) -> (Option<String>, Option<String>) {
    let (found, _, problem) = key::resolve(&store(), state.env().get(key::ANTHROPIC.1));
    (found, problem)
}

/// Every harness as configured, so drafting sees the user's own overrides and custom ones.
fn harnesses(state: &AppState) -> Vec<HarnessDef> {
    crate::harness::resolve_all(&state.settings.get().harnesses)
        .into_iter()
        .map(|resolved| resolved.def)
        .collect()
}

fn status(state: &AppState, preferred: Option<&str>) -> DraftStatus {
    let settings = state.settings.get().draft;
    let all = harnesses(state);
    let harness = writer(&all, preferred).map(|def| def.label.clone());
    let (found, _) = anthropic_key(state);
    let key = found.is_some();
    let problem = match (&harness, key) {
        (None, false) => Some(
            "No agent here can write one, and there is no Anthropic API key. Give a harness its \
             non-interactive arguments in Settings → Harnesses, or add a key in Settings → Assist."
                .to_owned(),
        ),
        _ => None,
    };
    DraftStatus {
        enabled: settings.enabled,
        available: settings.enabled && (harness.is_some() || key),
        harness,
        key,
        model: settings.model,
        problem,
    }
}

/// Who would write, for this workspace. `harnessId` is the agent the workspace is using.
#[tauri::command]
#[specta::specta]
pub async fn draft_status(app: AppHandle, harness_id: Option<String>) -> IpcResult<DraftStatus> {
    blocking(app, move |state| Ok(status(state, harness_id.as_deref()))).await
}

/// Keep an Anthropic API key in the OS credential store. Never read back into the webview.
#[tauri::command]
#[specta::specta]
pub async fn draft_save_key(app: AppHandle, key: String) -> IpcResult<DraftStatus> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err(IpcError::new("draft_no_key", "Paste an API key first."));
    }
    blocking(app, move |state| {
        store()
            .set(&key)
            .map_err(|problem| IpcError::new("draft_key_store", problem))?;
        Ok(status(state, None))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn draft_forget_key(app: AppHandle) -> IpcResult<DraftStatus> {
    blocking(app, move |state| {
        store()
            .delete()
            .map_err(|problem| IpcError::new("draft_key_store", problem))?;
        Ok(status(state, None))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn draft_save_settings(
    app: AppHandle,
    enabled: bool,
    model: String,
) -> IpcResult<DraftStatus> {
    let model = model.trim().to_owned();
    if model.is_empty() {
        return Err(IpcError::new("draft_no_model", "Name a model to ask."));
    }
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.draft.enabled = enabled;
                settings.draft.model = model.clone();
            })
            .map_err(|error| {
                IpcError::new(
                    "settings_write_failed",
                    format!("Could not save settings: {error}"),
                )
            })?;
        Ok(status(state, None))
    })
    .await
}

/// What a draft needs before anything leaves the machine: who writes, what they are told, and
/// the diff itself.
struct Prepared {
    agent: Option<(Program, HarnessDef)>,
    key: Option<String>,
    root: std::path::PathBuf,
    system: &'static str,
    prompt: String,
}

/// Gather it all on the blocking side — settings, git, the credential store — so the async half
/// only makes the call.
fn prepare(
    state: &AppState,
    workspace_id: &str,
    harness_id: Option<&str>,
    want: Want,
) -> IpcResult<Prepared> {
    if !state.settings.get().draft.enabled {
        return Err(IpcError::new(
            "draft_off",
            "Writing with a model is switched off in Settings → Assist.",
        ));
    }
    let (row, root) = workspace(state, workspace_id)?;
    let git = Git::new(&state.env())?;
    let changes = Changes {
        git: &git,
        root: &root,
        base_branch: row.base_branch.as_deref(),
    };

    // A commit message describes what is not committed; a pull request, what the branch has done.
    let scope = match want {
        Want::CommitMessage => Scope::Uncommitted,
        Want::PullRequest => Scope::Committed,
    };
    let diff = changes.combined_patch(scope)?;
    if diff.trim().is_empty() {
        return Err(IpcError::new(
            "draft_nothing_to_describe",
            match want {
                Want::CommitMessage => "There are no uncommitted changes to describe.",
                Want::PullRequest => "This branch has no commits to describe yet.",
            },
        ));
    }

    // What the agent was asked to do, which is what turns "describe this diff" into
    // "explain this change". Same source Assist uses for its off-task question.
    let task = state.store.session_prompts(workspace_id)?.join("\n\n");
    let task = Some(task).filter(|task| !task.trim().is_empty());
    let (system, prompt) = ask(want, task.as_deref(), &diff);

    let all = harnesses(state);
    let env = state.env();
    let agent = writer(&all, harness_id)
        .and_then(|def| Program::find(&env, &def.command).map(|program| (program, def.clone())));
    let (key, _) = anthropic_key(state);

    if agent.is_none() && key.is_none() {
        return Err(IpcError::new(
            "draft_nobody_can_write",
            "No agent here can write one, and there is no Anthropic API key. Give a harness its \
             non-interactive arguments in Settings → Harnesses, or add a key in Settings → Assist.",
        ));
    }
    Ok(Prepared {
        agent,
        key,
        root,
        system,
        prompt,
    })
}

/// Run whichever backend can answer: the agent first, the key second.
///
/// A failing agent falls through to the key rather than ending there — an agent that is not
/// logged in is exactly when the other path earns its place — and if there is no key, the
/// agent's own complaint is what the user sees.
async fn run(app: AppHandle, prepared: Prepared) -> IpcResult<String> {
    let Prepared {
        agent,
        key,
        root,
        system,
        prompt,
    } = prepared;

    let mut first: Option<IpcError> = None;
    if let Some((program, def)) = agent {
        let prompt = prompt.clone();
        let written =
            tauri::async_runtime::spawn_blocking(move || ask_agent(&program, &def, &root, &prompt))
                .await
                .map_err(|error| IpcError::internal(error.to_string()))?;
        match written {
            Ok(written) => return Ok(written),
            Err(error) if key.is_none() => return Err(error),
            Err(error) => first = Some(error),
        }
    }

    let Some(key) = key else {
        return Err(first.unwrap_or_else(|| {
            IpcError::new("draft_nobody_can_write", "Nothing here can write one.")
        }));
    };
    let model = blocking(app, |state| Ok(state.settings.get().draft.model)).await?;
    match anthropic::write(&key, &model, system, &prompt).await {
        Ok(written) => Ok(draft::tidy(&written)),
        // The agent was tried first, so its failure is the one that explains the most.
        Err(error) => Err(first.unwrap_or_else(|| failed(error))),
    }
}

/// Write a commit message for what the workspace has not committed.
#[tauri::command]
#[specta::specta]
pub async fn draft_commit_message(
    app: AppHandle,
    workspace_id: String,
    harness_id: Option<String>,
) -> IpcResult<String> {
    let handle = app.clone();
    let prepared = blocking(app, move |state| {
        prepare(
            state,
            &workspace_id,
            harness_id.as_deref(),
            Want::CommitMessage,
        )
    })
    .await?;
    let written = run(handle, prepared).await?;
    Ok(draft::clamp(&written, draft::MAX_COMMIT_CHARS))
}

/// A pull request's title and description, from the commits the branch has made.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DraftedPullRequest {
    pub title: String,
    pub body: String,
}

#[tauri::command]
#[specta::specta]
pub async fn draft_pull_request(
    app: AppHandle,
    workspace_id: String,
    harness_id: Option<String>,
) -> IpcResult<DraftedPullRequest> {
    let handle = app.clone();
    let prepared = blocking(app, move |state| {
        prepare(
            state,
            &workspace_id,
            harness_id.as_deref(),
            Want::PullRequest,
        )
    })
    .await?;
    let written = run(handle, prepared).await?;
    let (title, body) = draft::split_pull_request(&written);
    Ok(DraftedPullRequest { title, body })
}

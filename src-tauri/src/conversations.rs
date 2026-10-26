//! What a workspace's agents are told about each other's conversations, for the window: the note
//! a new agent would be given, and the digest as it reads now. Both come from the core
//! (`yardsort_core::activity::digest`) and are read when asked for; nothing is stored. See
//! `docs/design/29-conversation-context.md`.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use yardsort_core::activity::digest;
use yardsort_core::usage::logs::Sources;

use crate::error::IpcResult;
use crate::state::{blocking, AppState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContext {
    /// What the note is about, as a tab says it: "claude's conversation".
    pub about: String,
    /// The note, as an agent started now would read it.
    pub note: String,
    /// What `ys workspace context` gives now.
    pub digest: String,
}

/// The note and the digest for an agent in `workspace_id` — `except` the conversation it is, for
/// one already running — or `None` when the project does not share its agents' conversations or
/// none of the others has said anything yet.
fn sources(state: &AppState) -> Sources {
    let env = state.env();
    Sources::from_env(&|name| env.vars.get(name).cloned())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) fn context(
    state: &AppState,
    workspace_id: &str,
    except: Option<&str>,
) -> IpcResult<Option<ConversationContext>> {
    let Some(workspace) = state.store.workspace(workspace_id)? else {
        return Ok(None);
    };
    if !state.store.conversations_shared(&workspace.project_id)? {
        return Ok(None);
    }
    let found = digest::gather(&state.store, &workspace, except, &sources(state), now())?;
    let Some(note) = digest::note(&found) else {
        return Ok(None);
    };
    Ok(Some(ConversationContext {
        about: note.about,
        note: note.text,
        digest: digest::render(&digest::select(&found, &digest::Options::default())),
    }))
}

/// Whether to ask, before an agent starts in `workspace_id`, if its project should share its
/// agents' conversations — asked once per project, beside a conversation that has said
/// something — and what the note would be about. The answer is `conversations_share`.
#[tauri::command]
#[specta::specta]
pub async fn conversations_unasked(
    app: AppHandle,
    workspace_id: String,
) -> IpcResult<Option<String>> {
    blocking(app, move |state| {
        let Some(workspace) = state.store.workspace(&workspace_id)? else {
            return Ok(None);
        };
        Ok(digest::unasked(
            &state.store,
            &workspace,
            &sources(state),
            now(),
        )?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn conversation_context(
    app: AppHandle,
    workspace_id: String,
    except: Option<String>,
) -> IpcResult<Option<ConversationContext>> {
    blocking(app, move |state| {
        context(state, &workspace_id, except.as_deref())
    })
    .await
}

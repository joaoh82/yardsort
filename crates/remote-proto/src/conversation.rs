//! A conversation as the phone reads it, and the permission questions it may answer. Since
//! minor 8.
//!
//! `conversation.read` gives a conversation's turns: what the user typed and what the agent wrote
//! back, read on the host from the harness's own transcript when asked and kept nowhere. It is
//! the text the terminal already shows a device that may watch it, so it needs `terminal:read`.
//! A tool's input and output are not in it, and nothing is read from the terminal.
//!
//! An agent that asks for a permission through its own hook — Claude Code's `PermissionRequest`
//! — waits for an answer while its terminal asks too. The questions still open come with the
//! turns ([`Ask`]), and `approval.answer` answers one. Whichever answer comes first, the phone's
//! or a key pressed in the terminal, is the one the agent takes. Answering is what a key in the
//! terminal would do, so it needs `terminal:control`.

use serde::{Deserialize, Serialize};

use crate::ids::{SessionRecordId, WorkspaceId};

/// Longest single text of a turn, in characters: one message, or one of the agent's texts. A
/// longer one is cut, and its turn says so.
pub const MAX_TURN_TEXT: usize = 16_000;
/// The most texts of the agent's one turn carries; the last ones, since the reply is last.
pub const MAX_SAID: usize = 24;
/// Longest command in an [`Ask`], in characters.
pub const MAX_COMMAND: usize = 2_000;
/// The most open questions a page lists.
pub const MAX_ASKS: usize = 8;
/// Longest ask id, in bytes.
pub const MAX_ASK_ID: usize = 64;

/// `conversation.read`. Without `from`, the newest turns that fit; with it, turns from that one
/// on, oldest first, as many as fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationReadParams {
    pub workspace: WorkspaceId,
    pub session: SessionRecordId,
    /// A turn's number, counted from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
}

/// One turn: the user's message, and every text the agent wrote back, commentary between its
/// steps first and its reply last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    /// Counted from 1.
    pub n: u32,
    /// When the message was written, by the host's clock, in milliseconds since the Unix epoch.
    pub at: i64,
    pub user: String,
    pub said: Vec<String>,
    /// Some of this turn's text was left out: a text cut to [`MAX_TURN_TEXT`], or earlier texts
    /// past [`MAX_SAID`]. The rest is on the computer.
    #[serde(default)]
    pub cut: bool,
}

/// A permission an agent asked for, still open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub id: String,
    pub asked_at: i64,
    /// The tool's name, as the agent gives it: `Bash`, `Edit`, `WebFetch`, …
    pub tool: String,
    /// What it would run, for a shell command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The file it would touch, relative to the workspace when it is in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The reply to `conversation.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPage {
    pub session: SessionRecordId,
    /// How many turns the conversation has.
    pub total: u32,
    /// Oldest first.
    pub turns: Vec<ConversationTurn>,
    /// Why the transcript could not be read, when it could not. The terminal still can be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// Permissions it asked for and nobody has answered yet, oldest first.
    #[serde(default)]
    pub asks: Vec<Ask>,
    /// Whether the agent answers its permissions through Yardsort: a question it asks can be
    /// answered with `approval.answer`.
    #[serde(default)]
    pub answers_asks: bool,
}

/// An answer to an [`Ask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

/// `approval.answer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswerParams {
    pub workspace: WorkspaceId,
    pub session: SessionRecordId,
    pub ask: String,
    pub decision: ApprovalDecision,
}

impl ApprovalAnswerParams {
    pub fn invalid(&self) -> Option<&'static str> {
        let id_ok = !self.ask.is_empty()
            && self.ask.len() <= MAX_ASK_ID
            && self
                .ask
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        (!id_ok).then_some("That question is not one Yardsort asked.")
    }
}

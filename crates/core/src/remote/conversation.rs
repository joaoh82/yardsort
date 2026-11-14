//! `conversation.read` and `approval.answer`: a conversation's turns, read from the harness's
//! own transcript as `ys workspace context` reads them ([`crate::activity::digest`]), and the
//! permissions its agent asked for ([`crate::activity::asks`]).
//!
//! Nothing is kept: each request reads the transcript again. The terminal is never read. The text
//! is what that conversation's terminal shows, so a grant that may watch the terminal may read
//! it; one that may type may answer.

use std::path::PathBuf;

use remote_proto::conversation::{
    ApprovalAnswerParams, ApprovalDecision, Ask, ConversationPage, ConversationReadParams,
    ConversationTurn, MAX_ASKS, MAX_COMMAND, MAX_SAID, MAX_TURN_TEXT,
};
use remote_proto::{clip, ErrorCode, Grant, RemoteError, MAX_TEXT};

use super::{encoded_len, unreadable, Answer, HostFacade, PAGE_BUDGET};
use crate::activity::{asks, digest, transcript::Turn};
use crate::usage::logs::Sources;

/// Where this host's agents keep their conversations and open questions. Without it, the
/// conversation operations are not answered.
#[derive(Debug, Clone)]
pub struct Conversations {
    /// From the environment agents are launched in.
    pub sources: Sources,
    /// This profile's data folder.
    pub data_dir: PathBuf,
}

/// The harnesses whose permission questions are answered through Yardsort.
const ANSWERING: &[&str] = &[crate::activity::claude::HARNESS_ID];

impl HostFacade<'_> {
    pub(super) fn conversation(
        &self,
        grant: &Grant,
        params: &ConversationReadParams,
    ) -> Answer<ConversationPage> {
        let conversations = self.conversations.ok_or_else(not_here)?;
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        let record = self.record(&workspace.id, params.session.as_str())?;
        let now = crate::store::now_ms();
        let read = digest::one(
            self.store,
            &workspace,
            &record.id,
            &conversations.sources,
            now,
        )
        .map_err(|_| unreadable())?;
        let (turns, unreadable_why) = match read {
            Some(found) => (found.turns, found.unreadable),
            None => (Vec::new(), None),
        };
        let answers_asks = ANSWERING.contains(&record.harness_id.as_str());
        let open = if answers_asks && record.running {
            asks::open(&asks::asks_dir(&conversations.data_dir), &record.id, now)
        } else {
            Vec::new()
        };
        let mut page = ConversationPage {
            session: params.session.clone(),
            total: u32::try_from(turns.len()).unwrap_or(u32::MAX),
            turns: Vec::new(),
            unreadable: unreadable_why.map(|why| clip(&why, MAX_TEXT)),
            asks: open
                .into_iter()
                .take(MAX_ASKS)
                .map(|ask| Ask {
                    id: ask.id,
                    asked_at: ask.at_ms,
                    tool: clip(&ask.tool, MAX_TEXT),
                    command: ask.command.map(|c| clip(&c, MAX_COMMAND)),
                    path: ask.path.map(|p| clip(&p, MAX_TEXT)),
                })
                .collect(),
            answers_asks,
        };
        page.turns = fit_turns(&page, &turns, params.from)?;
        Ok(page)
    }

    pub(super) fn answer_ask(&self, grant: &Grant, params: &ApprovalAnswerParams) -> Answer<()> {
        let conversations = self.conversations.ok_or_else(not_here)?;
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        let running = self
            .record(&workspace.id, params.session.as_str())
            .is_ok_and(|row| row.running);
        let decision = match params.decision {
            ApprovalDecision::Allow => asks::Decision::Allow,
            ApprovalDecision::Deny => asks::Decision::Deny,
        };
        let answered = running
            && asks::answer(
                &asks::asks_dir(&conversations.data_dir),
                params.session.as_str(),
                &params.ask,
                decision,
                crate::store::now_ms(),
            )
            .map_err(|_| unreadable())?;
        if answered {
            Ok(())
        } else {
            Err(RemoteError::new(
                ErrorCode::NotFound,
                "That question is not open any more: it was answered, or the agent moved on.",
            ))
        }
    }
}

impl HostFacade<'_> {
    /// One conversation's record, when it is `workspace`'s: any other is `not_found`.
    fn record(&self, workspace: &str, session: &str) -> Answer<crate::store::SessionRow> {
        self.store
            .session(session)
            .map_err(|_| unreadable())?
            .filter(|row| row.workspace_id == workspace)
            .ok_or_else(RemoteError::not_found)
    }
}

fn not_here() -> RemoteError {
    RemoteError::new(
        ErrorCode::Unsupported,
        "Conversations are not available from this computer yet.",
    )
}

/// One turn, its texts within the protocol's limits.
fn shown(n: usize, turn: &Turn, max: usize) -> Answer<ConversationTurn> {
    let mut cut = turn.said.len() > MAX_SAID;
    let mut text = |t: &str| {
        let clipped = clip(t, max);
        cut |= clipped.len() != t.len();
        clipped
    };
    let user = text(&turn.user);
    let said = turn.said[turn.said.len().saturating_sub(MAX_SAID)..]
        .iter()
        .map(|t| text(t))
        .collect();
    Ok(ConversationTurn {
        n: u32::try_from(n).map_err(|_| unreadable())?,
        at: turn.at,
        user,
        said,
        cut,
    })
}

/// A turn that fits a page on its own, its texts cut shorter until it does.
fn fitted(n: usize, turn: &Turn, room: usize) -> Answer<ConversationTurn> {
    let mut max = MAX_TURN_TEXT;
    loop {
        let shown = shown(n, turn, max)?;
        if encoded_len(&shown) <= room || max <= 64 {
            return Ok(shown);
        }
        max /= 2;
    }
}

/// The turns that fit beside the rest of `page`: from turn `from` on, or the newest without it.
/// The first turn taken always goes, cut to fit if it must.
fn fit_turns(
    page: &ConversationPage,
    turns: &[Turn],
    from: Option<u32>,
) -> Answer<Vec<ConversationTurn>> {
    let room = PAGE_BUDGET.saturating_sub(encoded_len(page) + 64);
    let numbered = turns.iter().enumerate().map(|(i, turn)| (i + 1, turn));
    let order: Vec<(usize, &Turn)> = match from {
        Some(from) => numbered
            .skip(
                usize::try_from(from)
                    .unwrap_or(usize::MAX)
                    .saturating_sub(1),
            )
            .collect(),
        None => numbered.rev().collect(),
    };
    let mut taken = Vec::new();
    let mut used = 0;
    for (n, turn) in order {
        let shown = if taken.is_empty() {
            fitted(n, turn, room)?
        } else {
            shown(n, turn, MAX_TURN_TEXT)?
        };
        let size = encoded_len(&shown) + 1;
        if !taken.is_empty() && used + size > room {
            break;
        }
        used += size;
        taken.push(shown);
    }
    if from.is_none() {
        taken.reverse();
    }
    Ok(taken)
}

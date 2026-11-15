//! `test.waiting` and `test.decide`: a smoke test run waiting for its person, as the approval
//! card shows it ([`crate::smoke::runs::review`]), and their answer.
//!
//! The gate a test file goes through is a person seeing what it does
//! (`docs/design/32-smoke-tests.md`). No agent can pass it: there is no `ys` command that
//! approves, and this is not one either. A request here comes from a device the host's person
//! paired, over that device's own channel, with a grant that may type into the host's terminals —
//! which is already the power to run anything there, as starting an agent is ([`super::start`]).
//!
//! A run is let go only on what its card showed. The card is built here from the file as it was
//! when the run was asked for, the same way for the reading and for the deciding, and a card that
//! does not fit a message whole — a file too long, or one that does not check out — can only be
//! refused from a device.

use std::collections::BTreeMap;

use remote_proto::smoke::{
    TestAskedBy, TestDecideParams, TestDecision, TestEnvUse, TestInput, TestStep, TestWaitingPage,
    TestWaitingParams, WaitingTest, MAX_DEFINITION, MAX_INPUT, MAX_NAMES, MAX_STEPS,
    MAX_STEP_WORDS,
};
use remote_proto::{clip, ErrorCode, Grant, RemoteError, SessionRecordId, MAX_LABEL, MAX_TEXT};

use super::{encoded_len, unreadable, Answer, HostFacade, PAGE_BUDGET};
use crate::smoke::runs;
use crate::store::TestRunRow;

/// What a host that carries smoke test runs out does once a device decided one. Without it the
/// operations are not answered: a headless host queues no runs.
pub trait TestGate: Send + Sync {
    /// A run was let go or refused: tell the window, and look at the runs now.
    fn decided(&self);
}

/// Said of a card that cannot be let go from a device.
const TOO_LONG: &str = "This test is too long to show whole here. Run it at the computer.";

impl HostFacade<'_> {
    pub(super) fn tests_waiting(
        &self,
        grant: &Grant,
        params: &TestWaitingParams,
    ) -> Answer<TestWaitingPage> {
        self.tests.ok_or_else(not_here)?;
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        let waiting: Vec<TestRunRow> = self
            .store
            .active_test_runs()
            .map_err(|_| unreadable())?
            .into_iter()
            .filter(|row| waits_in(row, &workspace.id))
            .collect();
        Ok(TestWaitingPage {
            first: waiting
                .first()
                .map(|row| self.card(row))
                .transpose()?
                .map(Box::new),
            behind: u32::try_from(waiting.len().saturating_sub(1)).unwrap_or(u32::MAX),
        })
    }

    pub(super) fn decide_test(&self, grant: &Grant, params: &TestDecideParams) -> Answer<()> {
        let gate = self.tests.ok_or_else(not_here)?;
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        let row = self
            .store
            .test_run(&params.run)
            .map_err(|_| unreadable())?
            .filter(|row| waits_in(row, &workspace.id))
            .ok_or_else(gone)?;
        if row.content_hash != params.content_hash {
            return Err(RemoteError::new(
                ErrorCode::Stale,
                "This is not the file that was shown. Read it again.",
            ));
        }
        let done = match params.decision {
            TestDecision::Refuse => self.store.refuse_test_run(&row.id),
            TestDecision::Run => {
                // The card as the device was given it: one that was not whole was not seen whole.
                let card = self.card(&row)?;
                if !card.whole {
                    return Err(RemoteError::new(
                        ErrorCode::InvalidRequest,
                        card.why_not.unwrap_or_else(|| TOO_LONG.to_owned()),
                    ));
                }
                self.store.approve_test_run(&row.id)
            }
        }
        .map_err(|_| unreadable())?;
        if !done {
            // Answered at the computer between the two looks.
            return Err(gone());
        }
        gate.decided();
        Ok(())
    }

    /// A waiting run as its card shows it, within one message.
    fn card(&self, row: &TestRunRow) -> Answer<WaitingTest> {
        let inputs: BTreeMap<String, String> =
            serde_json::from_str(&row.inputs).unwrap_or_default();
        let mut card = WaitingTest {
            run: row.id.clone(),
            name: clip(&row.test_name, MAX_TEXT),
            asked_by: match row.requested_by.as_str() {
                "cli" => TestAskedBy::Agent,
                "app" => TestAskedBy::App,
                "workflow" => TestAskedBy::Workflow,
                _ => TestAskedBy::Other,
            },
            session: row
                .session_record_id
                .as_ref()
                .and_then(|id| SessionRecordId::new(id.clone()).ok()),
            asked_at: row.created_at,
            inputs: Vec::new(),
            steps: Vec::new(),
            env: Vec::new(),
            runs_a_program: false,
            environment: Vec::new(),
            approved_before: false,
            previous: None,
            definition: None,
            content_hash: row.content_hash.clone(),
            whole: true,
            why_not: None,
        };
        // Whether anything the card says was left out.
        let cut = std::cell::Cell::new(inputs.len() > MAX_NAMES);
        // Text within its limit, noting when any of it was left out.
        let within = |text: &str, max: usize| {
            let clipped = clip(text, max);
            cut.set(cut.get() || clipped.len() != text.len());
            clipped
        };
        card.inputs = inputs
            .iter()
            .take(MAX_NAMES)
            .map(|(name, value)| TestInput {
                name: within(name, MAX_LABEL),
                value: within(value, MAX_INPUT),
            })
            .collect();
        let review = match runs::run_review(self.store, row) {
            Ok(review) => review,
            Err(error) if error.code == "test_invalid" => {
                card.whole = false;
                card.why_not = Some(clip(&error.message, MAX_TEXT));
                return Ok(card);
            }
            Err(_) => return Err(unreadable()),
        };
        if review.steps.len() > MAX_STEPS
            || review.env.len() > MAX_NAMES
            || review.environment.len() > MAX_NAMES
        {
            cut.set(true);
        }
        card.steps = review
            .steps
            .iter()
            .take(MAX_STEPS)
            .map(|step| TestStep {
                id: within(&step.id, MAX_LABEL),
                phase: within(&step.phase, MAX_LABEL),
                words: within(&step.words, MAX_STEP_WORDS),
            })
            .collect();
        card.env = review
            .env
            .iter()
            .take(MAX_NAMES)
            .map(|used| TestEnvUse {
                name: within(&used.name, MAX_LABEL),
                set: used.set,
            })
            .collect();
        card.environment = review
            .environment
            .iter()
            .take(MAX_NAMES)
            .map(|name| within(name, MAX_LABEL))
            .collect();
        card.runs_a_program = review.runs_a_program;
        card.approved_before = review.approved;
        if let Some(previous) = review.previous {
            // What changed is part of what is seen: both versions go, or the card is not whole.
            let fits = |text: &str| text.chars().count() <= MAX_DEFINITION;
            if fits(&previous) && fits(&row.definition) {
                card.previous = Some(previous);
                card.definition = Some(row.definition.clone());
            } else {
                cut.set(true);
            }
        }
        let room = PAGE_BUDGET.saturating_sub(64);
        if encoded_len(&card) > room && card.previous.is_some() {
            card.previous = None;
            card.definition = None;
            cut.set(true);
        }
        while encoded_len(&card) > room && !card.steps.is_empty() {
            card.steps.truncate(card.steps.len() / 2);
            cut.set(true);
        }
        if cut.get() {
            card.whole = false;
            card.why_not = Some(TOO_LONG.to_owned());
        }
        Ok(card)
    }
}

/// Waiting for a person, in this workspace.
fn waits_in(row: &TestRunRow, workspace: &str) -> bool {
    row.status == "awaiting_approval" && row.workspace_id.as_deref() == Some(workspace)
}

fn gone() -> RemoteError {
    RemoteError::new(
        ErrorCode::NotFound,
        "That run is not waiting any more: it was answered, or cancelled.",
    )
}

fn not_here() -> RemoteError {
    RemoteError::new(
        ErrorCode::Unsupported,
        "Smoke tests are not run from this computer.",
    )
}

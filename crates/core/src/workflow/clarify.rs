//! Clarification inside a `wait_session`: the agent asks, the person answers, the same
//! session continues.
//!
//! A question is `ys workflow ask`. Text the agent prints is not a question, and a session
//! going quiet is not one either. The workflow stores the rounds, waits, and types the
//! answer back into the session that asked. An approval is a different step.

use serde::{Deserialize, Serialize};

use crate::store::{Store, StoreResult, WorkflowRunRow};
use crate::workflow::artifact;
use crate::workflow::waits::{self, AnswerWrite};
use crate::workflow::{parse, Action, SessionRef};

/// How many rounds a clarifying step allows when the file leaves `max_rounds` out.
pub const DEFAULT_ROUNDS: u32 = 3;
/// The most a file may set, and the most the engine will wait through.
pub const MAX_ROUNDS: u32 = 10;
/// Questions in one round. One reply answers the whole round.
pub const MAX_QUESTIONS: usize = 8;
/// Characters in one question, after controls are stripped.
pub const MAX_QUESTION_CHARS: usize = 2_000;

/// Appended to the first message of a session a later step watches with `clarify: true`.
/// The file's own prompt does not have to mention the command.
pub const ASK_NOTE: &str = "\
If you need the person to decide something before you can finish, ask with \
`ys workflow ask`, one `--question` per question. A question you only print is not \
asked. When you are done, record the outcome with `ys workflow summary --text`. \
Going quiet does not finish this step. This is not an approval.";

/// Put [`ASK_NOTE`] after `prompt`. An empty prompt becomes the note on its own.
pub fn with_note(prompt: Option<String>) -> Option<String> {
    Some(match prompt {
        Some(text) if text.trim().is_empty() => ASK_NOTE.to_owned(),
        Some(mut text) => {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
            text.push_str(ASK_NOTE);
            text
        }
        None => ASK_NOTE.to_owned(),
    })
}

/// What the engine needs to know about one step's rounds. The driver fills this from the
/// tables. A world with no clarifications leaves it empty. A clarifying step does not
/// finish on a quiet session: it finishes on [`Self::summary`], or on `until: exited`
/// when no question is still open. A session that has ended with no summary fails.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClarificationState {
    /// Rounds stored, including one the limit already refused.
    pub rounds: u32,
    /// An answer recorded and not yet claimed for the session.
    pub pending: Option<PendingAnswer>,
    /// A round is open and nobody has answered it. The step does not finish while this is set.
    pub waiting: bool,
    /// `ys workflow summary` was recorded. The step finishes on this.
    pub summary: bool,
    /// Claimed (`applied_at`) and not confirmed typed. Do not type it again.
    pub undelivered: bool,
}

/// One answer to type into the session that asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAnswer {
    pub round: u32,
    /// The message the session receives: the questions and the person's reply.
    pub text: String,
}

/// One stored round, as the brief and the engine read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub round: u32,
    pub questions: String,
    pub answer: Option<String>,
    pub actor: Option<String>,
    pub asked_at: i64,
    pub answered_at: Option<i64>,
    pub applied_at: Option<i64>,
    /// Set only after the keystrokes returned. Null means the claim was not confirmed.
    pub typed_at: Option<i64>,
}

/// Why `ys workflow ask` did not store a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskWrite {
    Recorded {
        round: u32,
    },
    Empty,
    TooManyQuestions,
    QuestionTooLong,
    AlreadyWaiting,
    /// The previous answer is stored and has not been typed into the session yet.
    NotSentBack,
    TooMany {
        asked: u32,
        limit: u32,
    },
    MissingRun,
    MissingStep,
    NotClarifying,
    NotWaiting {
        status: String,
    },
    RunEnded {
        status: String,
    },
}

/// Why `ys workflow summary` did not store the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryWrite {
    Recorded,
    Empty,
    /// A question on this step has no answer yet. The summary waits until that answer.
    AlreadyWaiting,
    MissingRun,
    MissingStep,
    NotClarifying,
    NotWaiting {
        status: String,
    },
    RunEnded {
        status: String,
    },
}

/// The step a question belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub run_id: String,
    pub step_id: String,
}

/// Why a session could not be matched to one clarifying step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateMiss {
    /// No running clarify step is watching the session this activity run started.
    None,
    /// More than one is. The command has to name `--step`.
    Many(usize),
}

/// The clarifying step whose session was started as activity run `activity_run`.
///
/// `YARDSORT_RUN_ID` is that activity run. The start step stores it as `run`. No new
/// launch variable is required, and nothing is read from the terminal.
pub fn locate(
    store: &Store,
    activity_run: &str,
    step: Option<&str>,
) -> StoreResult<Result<Located, LocateMiss>> {
    let activity_run = activity_run.trim();
    if activity_run.is_empty() {
        return Ok(Err(LocateMiss::None));
    }
    let mut found = Vec::new();
    for run in store.running_workflow_runs()? {
        let Ok(workflow) = parse(&run.definition) else {
            continue;
        };
        let rows = store.workflow_steps(&run.id)?;
        for candidate in &workflow.steps {
            if step.is_some_and(|wanted| wanted != candidate.id) {
                continue;
            }
            let Action::WaitSession {
                session: SessionRef::Step(source),
                clarify: true,
                ..
            } = &candidate.action
            else {
                continue;
            };
            let started = rows.iter().find(|row| &row.step_id == source);
            let recorded = started
                .map(|row| super::engine::parse_outputs(&row.outputs))
                .and_then(|outputs| outputs.get("run").cloned())
                .unwrap_or_default();
            if recorded == activity_run {
                found.push(Located {
                    run_id: run.id.clone(),
                    step_id: candidate.id.clone(),
                });
            }
        }
    }
    Ok(match found.len() {
        0 => Err(LocateMiss::None),
        1 => Ok(found.remove(0)),
        n => Err(LocateMiss::Many(n)),
    })
}

/// Store one round of questions on a clarifying step, and open the human wait when the
/// round is still inside the limit. The step does not move.
pub fn ask(
    store: &Store,
    run_id: &str,
    step_id: &str,
    questions: &[String],
) -> StoreResult<AskWrite> {
    let questions = match normalize_questions(questions) {
        Ok(questions) => questions,
        Err(write) => return Ok(write),
    };
    let Some(prepared) = prepare(store, run_id, step_id)? else {
        return missing(store, run_id, step_id);
    };
    let Prepared {
        limit, refs, phase, ..
    } = prepared;
    store.record_clarification(
        run_id,
        step_id,
        &encode_questions(&questions),
        &prompt_text(&questions),
        limit,
        &refs,
        &phase,
    )
}

/// Store the agent's outcome for a clarifying step. A later call replaces it. The step
/// does not move, and the text is not taken from the terminal. A question that is still
/// open is refused: the summary would otherwise finish a dead session as if none existed.
pub fn summary(
    store: &Store,
    run_id: &str,
    step_id: &str,
    text: &str,
) -> StoreResult<SummaryWrite> {
    let (body, _) = artifact::cap(text);
    if body.trim().is_empty() {
        return Ok(SummaryWrite::Empty);
    }
    let Some(_prepared) = prepare(store, run_id, step_id)? else {
        return Ok(match missing(store, run_id, step_id)? {
            AskWrite::MissingRun => SummaryWrite::MissingRun,
            AskWrite::MissingStep => SummaryWrite::MissingStep,
            AskWrite::NotClarifying => SummaryWrite::NotClarifying,
            AskWrite::NotWaiting { status } => SummaryWrite::NotWaiting { status },
            AskWrite::RunEnded { status } => SummaryWrite::RunEnded { status },
            _ => SummaryWrite::NotClarifying,
        });
    };
    let open = store
        .clarification_rounds(run_id, step_id)?
        .iter()
        .any(|round| round.answer.is_none() && round.applied_at.is_none());
    if open {
        return Ok(SummaryWrite::AlreadyWaiting);
    }
    store.record_clarification_summary(run_id, step_id, body.trim())?;
    Ok(SummaryWrite::Recorded)
}

/// What to tell the person when an ask was not stored. `None` when it was.
pub fn ask_refusal(result: &AskWrite, run: &str, step: &str) -> Option<String> {
    let message = match result {
        AskWrite::Recorded { .. } => return None,
        AskWrite::Empty => "Say at least one question.".to_owned(),
        AskWrite::TooManyQuestions => {
            format!("Ask at most {MAX_QUESTIONS} questions at once.")
        }
        AskWrite::QuestionTooLong => {
            format!("A question is longer than {MAX_QUESTION_CHARS} characters.")
        }
        AskWrite::AlreadyWaiting => format!("Step `{step}` is already waiting for an answer."),
        AskWrite::NotSentBack => {
            "The last answer has not been sent back to the session yet.".to_owned()
        }
        AskWrite::TooMany { asked, limit } => {
            format!("Clarification was asked {asked} times, which is past the limit ({limit}).")
        }
        AskWrite::MissingRun => format!("No run {run:?}. `ys workflow runs` lists them."),
        AskWrite::MissingStep => format!("Run {run} has no step `{step}`."),
        AskWrite::NotClarifying => {
            format!("Step `{step}` does not ask clarification questions.")
        }
        AskWrite::NotWaiting { status } => {
            format!("Step `{step}` is not waiting. It is {status}.")
        }
        AskWrite::RunEnded { status } => format!("Run {run} has already ended ({status})."),
    };
    Some(message)
}

/// What to tell the person when a summary was not stored. `None` when it was.
pub fn summary_refusal(result: &SummaryWrite, run: &str, step: &str) -> Option<String> {
    let message = match result {
        SummaryWrite::Recorded => return None,
        SummaryWrite::Empty => "Say the outcome in --text.".to_owned(),
        SummaryWrite::AlreadyWaiting => format!(
            "Step `{step}` is still waiting for an answer. Record the summary after that answer."
        ),
        SummaryWrite::MissingRun => format!("No run {run:?}. `ys workflow runs` lists them."),
        SummaryWrite::MissingStep => format!("Run {run} has no step `{step}`."),
        SummaryWrite::NotClarifying => {
            format!("Step `{step}` does not ask clarification questions.")
        }
        SummaryWrite::NotWaiting { status } => {
            format!("Step `{step}` is not waiting. It is {status}.")
        }
        SummaryWrite::RunEnded { status } => format!("Run {run} has already ended ({status})."),
    };
    Some(message)
}

/// What to tell the person when an answer was not stored. `None` when it was.
pub fn answer_refusal(result: &AnswerWrite, run: &str, step: &str) -> Option<String> {
    let message = match result {
        AnswerWrite::Recorded => return None,
        AnswerWrite::Missing => {
            format!("Step `{step}` is not waiting for an answer, or the answer was empty.")
        }
        AnswerWrite::NotAQuestion => format!("Step `{step}` is not a clarification."),
        AnswerWrite::AlreadyAnswered => format!("Step `{step}` already has an answer."),
        AnswerWrite::NotWaiting => format!("Step `{step}` is not waiting for an answer."),
        AnswerWrite::RunEnded { status } => format!("Run {run} has already ended ({status})."),
        AnswerWrite::WrongCode => "That code is not the one Yardsort showed you.".to_owned(),
        AnswerWrite::CodeLocked => {
            "Too many wrong codes. Open Yardsort to be shown a new one.".to_owned()
        }
        AnswerWrite::NoCode => {
            "There is no code for this step. Open Yardsort to be shown one.".to_owned()
        }
        AnswerWrite::CodeUnusable => {
            "That code cannot be checked. Open Yardsort to be shown a new one.".to_owned()
        }
        AnswerWrite::Conflict => "That answer was already sent with different contents.".to_owned(),
        AnswerWrite::Stale => remote_proto::workflow::STALE_MESSAGE.to_owned(),
    };
    Some(message)
}

/// Build the engine's view of the stored rounds.
pub fn state_from(rounds: &[Round]) -> ClarificationState {
    let answered = |round: &&Round| {
        round
            .answer
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty())
    };
    let undelivered = rounds
        .iter()
        .any(|round| answered(&round) && round.applied_at.is_some() && round.typed_at.is_none());
    // A claim that was not confirmed is not typed again. A later round waits behind that failure.
    let pending = if undelivered {
        None
    } else {
        rounds
            .iter()
            .rev()
            .find(|round| answered(round) && round.applied_at.is_none())
    };
    ClarificationState {
        rounds: u32::try_from(rounds.len()).unwrap_or(u32::MAX),
        pending: pending.map(|round| PendingAnswer {
            round: round.round,
            text: answer_for_session(
                &decode_questions(&round.questions),
                round.answer.as_deref().unwrap_or(""),
            ),
        }),
        waiting: rounds
            .iter()
            .any(|round| round.answer.is_none() && round.applied_at.is_none()),
        summary: false,
        undelivered,
    }
}

/// The questions and the reply, as the same session should read them.
pub fn answer_for_session(questions: &[String], answer: &str) -> String {
    let mut out = String::from("The person answered.\n");
    if !questions.is_empty() {
        out.push_str("\nYou asked:\n");
        for (index, question) in questions.iter().enumerate() {
            out.push_str(&format!("{}. {question}\n", index + 1));
        }
    }
    out.push_str("\nAnswer:\n");
    out.push_str(answer.trim());
    out.push('\n');
    out
}

/// Numbered questions, as the wait and the window show them.
pub fn prompt_text(questions: &[String]) -> String {
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| format!("{}. {question}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The outcome brief a later step consumes. Never empty: a step with nothing asked, and
/// an agent that recorded no summary, still leaves a document.
pub fn render_brief(rounds: &[Round], summary: Option<&str>) -> String {
    let mut out = String::from("# Outcome brief\n");
    if rounds.is_empty() {
        out.push_str("\nNo questions were asked.\n");
    } else {
        out.push_str("\n## Questions and answers\n");
        for round in rounds {
            out.push_str(&format!("\n### Round {}\n\n", round.round));
            for (index, question) in decode_questions(&round.questions).iter().enumerate() {
                out.push_str(&format!("{}. {question}\n", index + 1));
            }
            out.push_str("\nAnswer: ");
            match round.answer.as_deref() {
                Some(answer) => out.push_str(answer),
                None => out.push_str("(no answer)"),
            }
            out.push('\n');
        }
    }
    out.push_str("\n## Summary\n\n");
    match summary.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => out.push_str(text),
        None => out.push_str("The agent finished without recording a summary."),
    }
    out.push('\n');
    out
}

/// Questions as stored. A row that is not JSON is kept as one question, so an old or
/// damaged row still appears in the brief.
pub fn decode_questions(raw: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Asked {
        prompt: String,
    }
    serde_json::from_str::<Vec<Asked>>(raw)
        .map(|items| {
            items
                .into_iter()
                .map(|item| item.prompt)
                .filter(|prompt| !prompt.trim().is_empty())
                .collect()
        })
        .unwrap_or_else(|_| vec![raw.to_owned()])
}

fn encode_questions(questions: &[String]) -> String {
    #[derive(Serialize)]
    struct Asked<'a> {
        id: String,
        prompt: &'a str,
    }
    let items: Vec<Asked<'_>> = questions
        .iter()
        .enumerate()
        .map(|(index, prompt)| Asked {
            id: (index + 1).to_string(),
            prompt,
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

fn normalize_questions(questions: &[String]) -> Result<Vec<String>, AskWrite> {
    if questions.len() > MAX_QUESTIONS {
        return Err(AskWrite::TooManyQuestions);
    }
    let mut cleaned = Vec::new();
    for question in questions {
        let text = artifact::strip_controls(question);
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        if text.chars().count() > MAX_QUESTION_CHARS {
            return Err(AskWrite::QuestionTooLong);
        }
        cleaned.push(text);
    }
    if cleaned.is_empty() {
        Err(AskWrite::Empty)
    } else {
        Ok(cleaned)
    }
}

struct Prepared {
    limit: u32,
    refs: String,
    phase: String,
}

/// The run is going and the step is a clarifying wait that has not finished.
/// `None` when it is not, and the caller maps that to a refusal.
fn prepare(store: &Store, run_id: &str, step_id: &str) -> StoreResult<Option<Prepared>> {
    let Some(run) = store.workflow_run(run_id)? else {
        return Ok(None);
    };
    if run.status != "running" {
        return Ok(None);
    }
    let Ok(workflow) = parse(&run.definition) else {
        return Ok(None);
    };
    let Some(step) = workflow.steps.iter().find(|step| step.id == step_id) else {
        return Ok(None);
    };
    let Action::WaitSession {
        clarify: true,
        max_rounds,
        ..
    } = &step.action
    else {
        return Ok(None);
    };
    let rows = store.workflow_steps(run_id)?;
    let Some(row) = rows.iter().find(|row| row.step_id == step_id) else {
        return Ok(None);
    };
    if !matches!(row.status.as_str(), "pending" | "running" | "waiting") {
        return Ok(None);
    }
    let phase = waits::Phase {
        workflow_id: run.workflow_id.clone(),
        workflow_name: run.workflow_name.clone(),
        workspace_name: run.workspace_name.clone(),
        step_id: step.id.clone(),
        action: "wait_session".to_owned(),
    };
    Ok(Some(Prepared {
        limit: (*max_rounds).clamp(1, MAX_ROUNDS),
        refs: serde_json::to_string(&step.consumes).unwrap_or_else(|_| "[]".to_owned()),
        phase: phase.to_json(),
    }))
}

fn missing(store: &Store, run_id: &str, step_id: &str) -> StoreResult<AskWrite> {
    let Some(run) = store.workflow_run(run_id)? else {
        return Ok(AskWrite::MissingRun);
    };
    if run.status != "running" {
        return Ok(AskWrite::RunEnded { status: run.status });
    }
    let rows = store.workflow_steps(run_id)?;
    let Some(row) = rows.iter().find(|row| row.step_id == step_id) else {
        return Ok(AskWrite::MissingStep);
    };
    if !matches!(row.status.as_str(), "pending" | "running" | "waiting") {
        return Ok(AskWrite::NotWaiting {
            status: row.status.clone(),
        });
    }
    Ok(classify_definition(&run, step_id))
}

fn classify_definition(run: &WorkflowRunRow, step_id: &str) -> AskWrite {
    let Ok(workflow) = parse(&run.definition) else {
        return AskWrite::NotClarifying;
    };
    if workflow.steps.iter().any(|step| step.id == step_id) {
        AskWrite::NotClarifying
    } else {
        AskWrite::MissingStep
    }
}

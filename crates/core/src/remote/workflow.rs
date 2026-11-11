//! Workflow runs for a signed-in phone.
//!
//! The desktop owns the engine. This answers the relay while that engine is running.
//! A decision uses the same code-checked write as a terminal. The desktop window and a
//! phone cannot both apply, because the row is updated only while it is still undecided.
//! A host token, a computer, and any session Yardsort started are refused before that write.
//!
//! The one-time code is returned only by `workflow.challenge`, and only to a phone. It is
//! not in a run, an artifact, or a notification. The phone holds it and sends it back with
//! the decision. It is not shown.
//!
//! The computer being offline is the relay's `host_offline`. Nothing here queues a decision.
//! A retry of the same decision id, after the reply was lost, returns the outcome already
//! stored and does not apply again.

use std::path::Path;

use remote_proto::account::DeviceKind;
use remote_proto::workflow::{
    redact_preview, safe_error, WorkflowAction, WorkflowArtifactView, WorkflowChallenge,
    WorkflowDecision, WorkflowRunDetail, WorkflowRunPage, WorkflowRunSummary, WorkflowStepView,
    MAX_ARTIFACT_PREVIEW,
};
use remote_proto::{
    clip, ErrorCode, Grant, Operation, ProjectGrant, ProjectId, RemoteError, Reply, Request,
    MAX_NOTE, MAX_TEXT,
};
use sha2::{Digest, Sha256};

use crate::store::{self, ApprovalWrite, PhoneStamp, RemoteTaken, Store, StoreResult};
use crate::workflow::artifact::is_secret;
use crate::workflow::{self, waits};

use super::unreadable;

/// What the desktop uses to hand a phone the current code and to wake the driver.
pub trait WorkflowGate: Send + Sync {
    /// The one-time code for this waiting step, or nothing when there is not one.
    fn challenge(&self, store: &Store, run_id: &str, step_id: &str) -> StoreResult<Option<String>>;
    /// Look at the runs now, so a recorded decision or a queued run moves.
    fn wake(&self);
}

/// Who is asking. `launched_session` is true when this process was started by Yardsort.
#[derive(Debug, Clone)]
pub struct WorkflowCaller {
    pub kind: DeviceKind,
    pub device: String,
    pub launched_session: bool,
}

pub fn is_workflow(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::WorkflowRunList(_)
            | Operation::WorkflowRunGet(_)
            | Operation::WorkflowChallenge(_)
            | Operation::WorkflowDecide(_)
    )
}

/// Whether any of the markers Yardsort puts on a session it started is set and non-empty.
pub fn launched_markers(present: impl Fn(&str) -> bool) -> bool {
    [
        crate::coordinator::ENV,
        crate::activity::RUN_ENV,
        crate::activity::WORKSPACE_ENV,
        crate::activity::RECORD_ENV,
    ]
    .into_iter()
    .any(present)
}

/// This process was started by Yardsort. Tested through [`launched_markers`], not by
/// changing the process environment, which other tests share.
pub fn launched_here() -> bool {
    launched_markers(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// Answer one workflow request. `gate` is what the desktop drives with. Without it the
/// caller should not ask: the operation is unsupported.
pub fn answer(
    store: &Store,
    gate: Option<&dyn WorkflowGate>,
    caller: &WorkflowCaller,
    grant: &Grant,
    request: &Request,
) -> Result<Reply, RemoteError> {
    let Some(gate) = gate else {
        return Err(unsupported());
    };
    match &request.operation {
        Operation::WorkflowRunList(params) => {
            refuses_host(caller)?;
            list(store, grant, params).map(Reply::WorkflowRuns)
        }
        Operation::WorkflowRunGet(params) => {
            refuses_host(caller)?;
            detail(store, grant, &params.run_id).map(Reply::WorkflowRun)
        }
        Operation::WorkflowChallenge(params) => {
            refuses_decision(caller)?;
            let run = reachable_run(store, grant, &params.run_id)?;
            let _ = run;
            let code = gate
                .challenge(store, &params.run_id, &params.step_id)
                .map_err(|_| unreadable())?
                .filter(|code| !code.is_empty());
            let Some(code) = code else {
                return Err(RemoteError::new(
                    ErrorCode::NotFound,
                    "There is no code for this step. Open Yardsort on the computer to be shown one.",
                ));
            };
            Ok(Reply::WorkflowChallenge(WorkflowChallenge { code }))
        }
        Operation::WorkflowDecide(params) => {
            refuses_decision(caller)?;
            let _ = reachable_run(store, grant, &params.run_id)?;
            decide(store, gate, caller, params)
        }
        _ => Err(unsupported()),
    }
}

fn unsupported() -> RemoteError {
    RemoteError::new(
        ErrorCode::Unsupported,
        "Workflows are not available from this computer yet.",
    )
}

/// A host token and a session Yardsort started cannot even list runs.
fn refuses_host(caller: &WorkflowCaller) -> Result<(), RemoteError> {
    if caller.launched_session {
        return Err(RemoteError::new(
            ErrorCode::Forbidden,
            "A session Yardsort started cannot decide or answer a workflow.",
        ));
    }
    if caller.kind == DeviceKind::Host {
        return Err(RemoteError::new(
            ErrorCode::Forbidden,
            "A host token cannot decide a workflow.",
        ));
    }
    Ok(())
}

/// Only a signed-in phone decides. A host token, a computer, and a launched session are refused
/// before any row is written.
fn refuses_decision(caller: &WorkflowCaller) -> Result<(), RemoteError> {
    refuses_host(caller)?;
    if caller.kind != DeviceKind::Phone {
        return Err(RemoteError::new(
            ErrorCode::Forbidden,
            "Only a signed-in phone can decide a workflow.",
        ));
    }
    Ok(())
}

fn list(
    store: &Store,
    grant: &Grant,
    params: &remote_proto::workflow::WorkflowRunListParams,
) -> Result<WorkflowRunPage, RemoteError> {
    let open = waits::list_open(store).map_err(|_| unreadable())?;
    let rows = store.workflow_runs(None, 1_000).map_err(|_| unreadable())?;
    let mut runs = Vec::new();
    for row in rows {
        if !reaches(grant, row.project_id.as_deref()) {
            continue;
        }
        let waiting = open.iter().find(|wait| wait.run_id == row.id);
        runs.push(WorkflowRunSummary {
            run_id: row.id,
            name: clip(&row.workflow_name, MAX_TEXT),
            status: row.status,
            workspace: clip(&row.workspace_name, MAX_TEXT),
            waiting_on: waiting.map(|wait| clip(&wait.step_id, MAX_TEXT)),
            blocking: waiting.map(|wait| clip(&wait.action_needed, MAX_TEXT)),
            created_at: row.created_at,
        });
    }
    let (runs, next) = super::page(
        runs,
        |run| &run.run_id,
        params.cursor.as_ref(),
        params.limit,
        |page| {
            serde_json::to_vec(page)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX)
        },
    )?;
    Ok(WorkflowRunPage { runs, next })
}

fn detail(store: &Store, grant: &Grant, run_id: &str) -> Result<WorkflowRunDetail, RemoteError> {
    let row = reachable_run(store, grant, run_id)?;
    let steps = store.workflow_steps(run_id).map_err(|_| unreadable())?;
    let waiting = steps.iter().find(|step| step.status == "waiting");
    let wait = waiting.and_then(|step| store.workflow_wait(run_id, &step.step_id).ok().flatten());
    let (attempts, locked) = waiting
        .map(|step| {
            store
                .workflow_code_attempts(run_id, &step.step_id)
                .unwrap_or((0, false))
        })
        .unwrap_or((0, false));
    let decided = waiting
        .and_then(|step| {
            store
                .workflow_approval(run_id, &step.step_id)
                .ok()
                .flatten()
        })
        .is_some();
    let needs_person = wait.as_ref().is_some_and(|wait| {
        (wait.kind == "approval" && !decided) || (wait.kind == "question" && wait.answer.is_none())
    });
    let refs = wait
        .as_ref()
        .map(|wait| waits::artifact_refs(&wait.artifact_refs))
        .unwrap_or_default();
    let artifacts = store
        .workflow_artifacts(run_id)
        .map_err(|_| unreadable())?
        .into_iter()
        .filter_map(|artifact| artifact_view(&artifact, &refs))
        .collect();
    let resume_eligible = store
        .workflow_resume_eligible(run_id)
        .map_err(|_| unreadable())?
        .unwrap_or(false);
    Ok(WorkflowRunDetail {
        run_id: row.id,
        name: clip(&row.workflow_name, MAX_TEXT),
        status: row.status,
        workspace: clip(&row.workspace_name, MAX_TEXT),
        error: row.error.as_deref().map(safe_error),
        waiting_on: waiting
            .filter(|_| needs_person)
            .map(|step| clip(&step.step_id, MAX_TEXT)),
        blocking: wait
            .as_ref()
            .filter(|_| needs_person)
            .map(|wait| clip(&wait.action_needed, MAX_TEXT)),
        waiting_kind: wait
            .as_ref()
            .filter(|_| needs_person)
            .map(|wait| wait.kind.clone()),
        attempts,
        locked,
        resume_eligible,
        steps: steps
            .into_iter()
            .map(|step| WorkflowStepView {
                step_id: clip(&step.step_id, MAX_TEXT),
                action: clip(&step.action, MAX_TEXT),
                status: step.status,
                attempts: step.attempts,
                note: step.note.as_deref().map(|note| clip(note, MAX_NOTE)),
            })
            .collect(),
        artifacts,
    })
}

fn artifact_view(
    artifact: &store::WorkflowArtifactRow,
    refs: &[String],
) -> Option<WorkflowArtifactView> {
    if is_secret(Path::new(&artifact.name))
        || artifact
            .detail
            .as_deref()
            .is_some_and(|detail| is_secret(Path::new(detail)))
    {
        return None;
    }
    let relevant = refs.iter().any(|name| name == &artifact.name)
        || matches!(artifact.source.as_str(), "decision" | "verdict");
    let body = relevant.then(|| redact_preview(&artifact.body));
    Some(WorkflowArtifactView {
        name: clip(&artifact.name, MAX_TEXT),
        source: clip(&artifact.source, 32),
        truncated: artifact.truncated || artifact.body.chars().count() > MAX_ARTIFACT_PREVIEW,
        body,
    })
}

fn reachable_run(
    store: &Store,
    grant: &Grant,
    run_id: &str,
) -> Result<store::WorkflowRunRow, RemoteError> {
    let Some(row) = store.workflow_run(run_id).map_err(|_| unreadable())? else {
        return Err(RemoteError::not_found());
    };
    if !reaches(grant, row.project_id.as_deref()) {
        return Err(RemoteError::not_found());
    }
    Ok(row)
}

fn reaches(grant: &Grant, project: Option<&str>) -> bool {
    match project {
        None => matches!(grant.projects, ProjectGrant::All),
        Some(id) => ProjectId::new(id.to_owned()).is_ok_and(|id| grant.reaches(&id)),
    }
}

fn decide(
    store: &Store,
    gate: &dyn WorkflowGate,
    caller: &WorkflowCaller,
    params: &remote_proto::workflow::WorkflowDecideParams,
) -> Result<Reply, RemoteError> {
    let note = params.note.clone().unwrap_or_default();
    let step = params
        .step_id
        .clone()
        .unwrap_or_else(|| params.run_id.clone());
    let action = action_name(params.action);
    let stamp = PhoneStamp {
        key: params.decision_id.clone(),
        device_id: caller.device.clone(),
        fingerprint: fingerprint(action, &step, &note),
    };
    if params.action == WorkflowAction::Resume {
        return resume(store, gate, &params.run_id, &step, &stamp);
    }
    let code = params.code.as_deref().unwrap_or("");
    let result = match params.action {
        WorkflowAction::Approve => store.record_phone_approval(
            &params.run_id,
            &step,
            "approved",
            None,
            "phone",
            code,
            &stamp,
        ),
        WorkflowAction::Reject => store.record_phone_approval(
            &params.run_id,
            &step,
            "rejected",
            empty_note(&note),
            "phone",
            code,
            &stamp,
        ),
        WorkflowAction::Changes => store.record_phone_approval(
            &params.run_id,
            &step,
            "changes_requested",
            Some(note.trim()),
            "phone",
            code,
            &stamp,
        ),
        WorkflowAction::Answer => {
            return answer_question(
                store,
                gate,
                &params.run_id,
                &step,
                note.trim(),
                code,
                &stamp,
            );
        }
        WorkflowAction::Resume => unreachable!("resume is handled above"),
    }
    .map_err(|_| unreadable())?;
    if result == ApprovalWrite::Recorded {
        gate.wake();
    }
    Ok(approval_reply(&result, &params.run_id, &step))
}

fn answer_question(
    store: &Store,
    gate: &dyn WorkflowGate,
    run_id: &str,
    step_id: &str,
    text: &str,
    code: &str,
    stamp: &PhoneStamp,
) -> Result<Reply, RemoteError> {
    let result = store
        .answer_phone_wait(run_id, step_id, text, "phone", code, stamp)
        .map_err(|_| unreadable())?;
    if result == waits::AnswerWrite::Recorded {
        gate.wake();
    }
    let outcome = store::answer_stored(&result);
    let message = workflow::clarify::answer_refusal(&result, run_id, step_id)
        .unwrap_or_else(|| "Recorded.".to_owned());
    Ok(Reply::WorkflowDecision(WorkflowDecision {
        outcome,
        message: clip(&message, MAX_TEXT),
    }))
}

fn resume(
    store: &Store,
    gate: &dyn WorkflowGate,
    run_id: &str,
    step_id: &str,
    stamp: &PhoneStamp,
) -> Result<Reply, RemoteError> {
    let eligible = store
        .workflow_resume_eligible(run_id)
        .map_err(|_| unreadable())?;
    let Some(eligible) = eligible else {
        return Err(RemoteError::not_found());
    };
    let wanted = if eligible { "accepted" } else { "not_eligible" };
    let taken = store
        .take_remote_action(stamp, run_id, step_id, wanted)
        .map_err(|_| unreadable())?;
    let outcome = match taken {
        RemoteTaken::Conflict => "conflict".to_owned(),
        RemoteTaken::Fresh(outcome) | RemoteTaken::Same(outcome) => outcome,
    };
    if outcome == "accepted" {
        gate.wake();
    }
    let message = match outcome.as_str() {
        "accepted" => "The computer will continue that workflow.",
        "not_eligible" => "That workflow is not waiting to be continued.",
        "conflict" => "That decision was already sent with different contents.",
        _ => "That workflow is not waiting to be continued.",
    };
    Ok(Reply::WorkflowDecision(WorkflowDecision {
        outcome,
        message: message.to_owned(),
    }))
}

fn approval_reply(result: &ApprovalWrite, run_id: &str, step_id: &str) -> Reply {
    let outcome = store::approval_stored(result);
    let message = workflow::approval_refusal(result, run_id, step_id)
        .unwrap_or_else(|| "Recorded.".to_owned());
    Reply::WorkflowDecision(WorkflowDecision {
        outcome,
        message: clip(&message, MAX_TEXT),
    })
}

fn action_name(action: WorkflowAction) -> &'static str {
    match action {
        WorkflowAction::Approve => "approve",
        WorkflowAction::Reject => "reject",
        WorkflowAction::Changes => "changes",
        WorkflowAction::Answer => "answer",
        WorkflowAction::Resume => "resume",
    }
}

fn empty_note(note: &str) -> Option<&str> {
    let trimmed = note.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn fingerprint(action: &str, step: &str, note: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(action.as_bytes());
    hasher.update([0]);
    hasher.update(step.as_bytes());
    hasher.update([0]);
    hasher.update(note.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use remote_proto::account::DeviceKind;
    use remote_proto::workflow::{WorkflowAction, WorkflowDecideParams};
    use remote_proto::{
        DeviceId, ErrorCode, Grant, Operation, ProjectGrant, ProtocolVersion, Reply, Request,
        RequestId, Scope,
    };

    use super::*;
    use crate::store::Store;

    struct Gate {
        code: Mutex<Option<String>>,
        wakes: AtomicU32,
    }

    impl WorkflowGate for Gate {
        fn challenge(
            &self,
            _store: &Store,
            _run_id: &str,
            _step_id: &str,
        ) -> StoreResult<Option<String>> {
            Ok(self.code.lock().expect("code").clone())
        }

        fn wake(&self) {
            self.wakes.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Boom;

    impl WorkflowGate for Boom {
        fn challenge(
            &self,
            _store: &Store,
            _run_id: &str,
            _step_id: &str,
        ) -> StoreResult<Option<String>> {
            panic!("a refused caller must not be shown a code");
        }

        fn wake(&self) {
            panic!("a refused caller must not wake the driver");
        }
    }

    fn grant() -> Grant {
        Grant {
            device: DeviceId::new("phone-1").unwrap(),
            scopes: [Scope::WorkflowRead, Scope::WorkflowDecide].into(),
            projects: ProjectGrant::All,
        }
    }

    fn caller(kind: DeviceKind) -> WorkflowCaller {
        WorkflowCaller {
            kind,
            device: "phone-1".into(),
            launched_session: false,
        }
    }

    fn request(operation: Operation) -> Request {
        Request {
            request_id: RequestId::new("r-1").unwrap(),
            protocol: ProtocolVersion::CURRENT,
            operation,
        }
    }

    fn decide(action: WorkflowAction, note: Option<&str>, code: &str, id: &str) -> Request {
        request(Operation::WorkflowDecide(WorkflowDecideParams {
            run_id: "run-1".into(),
            step_id: Some("sign-off".into()),
            action,
            note: note.map(str::to_owned),
            code: Some(code.to_owned()),
            decision_id: id.into(),
        }))
    }

    fn waiting(store: &Store) -> String {
        store
            .conn()
            .execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-1', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'running', 'cli', 1)",
                [],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_step_runs
                 (run_id, step_id, position, action, status)
                 VALUES ('run-1', 'sign-off', 0, 'approve', 'waiting')",
                [],
            )
            .unwrap();
        store
            .begin_approval_code("run-1", "sign-off")
            .unwrap()
            .unwrap()
    }

    fn outcome(reply: Reply) -> String {
        match reply {
            Reply::WorkflowDecision(decision) => decision.outcome,
            other => panic!("not a decision: {other:?}"),
        }
    }

    #[test]
    fn launched_markers_see_any_of_the_four() {
        assert!(!launched_markers(|_| false));
        assert!(launched_markers(|name| name == crate::coordinator::ENV));
        assert!(launched_markers(|name| name == crate::activity::RUN_ENV));
        assert!(launched_markers(
            |name| name == crate::activity::WORKSPACE_ENV
        ));
        assert!(launched_markers(|name| name == crate::activity::RECORD_ENV));
    }

    #[test]
    fn a_host_a_computer_and_a_launched_session_cannot_decide() {
        let store = Store::in_memory();
        let code = waiting(&store);
        let boom = Boom;
        let asked = decide(WorkflowAction::Approve, None, &code, "d-1");
        for caller in [
            caller(DeviceKind::Host),
            caller(DeviceKind::Computer),
            WorkflowCaller {
                kind: DeviceKind::Phone,
                device: "phone-1".into(),
                launched_session: true,
            },
        ] {
            let error = answer(&store, Some(&boom), &caller, &grant(), &asked).unwrap_err();
            assert_eq!(error.code, ErrorCode::Forbidden, "{caller:?}");
        }
        assert!(store
            .workflow_approval("run-1", "sign-off")
            .unwrap()
            .is_none());
        let actions: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM workflow_remote_actions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(actions, 0);
    }

    #[test]
    fn a_phone_retry_is_one_decision_and_a_different_note_conflicts() {
        let store = Store::in_memory();
        let code = waiting(&store);
        let gate = Gate {
            code: Mutex::new(Some(code.clone())),
            wakes: AtomicU32::new(0),
        };
        let phone = caller(DeviceKind::Phone);
        let first = answer(
            &store,
            Some(&gate),
            &phone,
            &grant(),
            &decide(WorkflowAction::Approve, None, &code, "d-1"),
        )
        .unwrap();
        assert_eq!(outcome(first), "recorded");
        let again = answer(
            &store,
            Some(&gate),
            &phone,
            &grant(),
            &decide(WorkflowAction::Approve, None, "WRONG123", "d-1"),
        )
        .unwrap();
        assert_eq!(outcome(again), "recorded");
        let changed = answer(
            &store,
            Some(&gate),
            &phone,
            &grant(),
            &decide(WorkflowAction::Reject, Some("no"), &code, "d-1"),
        )
        .unwrap();
        assert_eq!(outcome(changed), "conflict");
        assert!(gate.wakes.load(Ordering::SeqCst) >= 1);
        let device: String = store
            .conn()
            .query_row(
                "SELECT device_id FROM workflow_approval_decisions WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(device, "phone-1");
    }

    #[test]
    fn a_run_detail_redacts_secrets_and_never_includes_the_code() {
        let store = Store::in_memory();
        let code = waiting(&store);
        store
            .conn()
            .execute(
                "INSERT INTO workflow_artifacts
                 (run_id, name, step_id, source, body, truncated, updated_at, attempt)
                 VALUES ('run-1', '.env', 'sign-off', 'step', 'TOKEN=1', 0, 1, 0),
                        ('run-1', 'notes', 'sign-off', 'decision', 'api_key: sk-live', 0, 1, 0)",
                [],
            )
            .unwrap();
        let gate = Gate {
            code: Mutex::new(None),
            wakes: AtomicU32::new(0),
        };
        let reply = answer(
            &store,
            Some(&gate),
            &caller(DeviceKind::Phone),
            &grant(),
            &request(Operation::WorkflowRunGet(
                remote_proto::workflow::WorkflowRunRef {
                    run_id: "run-1".into(),
                },
            )),
        )
        .unwrap();
        let Reply::WorkflowRun(detail) = reply else {
            panic!("not a run");
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains(&code));
        assert!(!json.contains("codeHash"));
        assert!(!json.contains(".env"));
        assert!(json.contains("[redacted]"));
        assert_eq!(detail.artifacts.len(), 1);
        assert!(!detail.locked);
    }

    #[test]
    fn resume_wakes_a_queued_run_and_a_retry_keeps_that_outcome() {
        let store = Store::in_memory();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-1', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'queued', 'cli', 1)",
                [],
            )
            .unwrap();
        let gate = Gate {
            code: Mutex::new(None),
            wakes: AtomicU32::new(0),
        };
        let asked = request(Operation::WorkflowDecide(WorkflowDecideParams {
            run_id: "run-1".into(),
            step_id: None,
            action: WorkflowAction::Resume,
            note: None,
            code: None,
            decision_id: "d-resume".into(),
        }));
        let phone = caller(DeviceKind::Phone);
        assert_eq!(
            outcome(answer(&store, Some(&gate), &phone, &grant(), &asked).unwrap()),
            "accepted"
        );
        store
            .conn()
            .execute(
                "UPDATE workflow_runs SET status = 'succeeded' WHERE id = 'run-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            outcome(answer(&store, Some(&gate), &phone, &grant(), &asked).unwrap()),
            "accepted"
        );
        assert_eq!(gate.wakes.load(Ordering::SeqCst), 2);
    }
}

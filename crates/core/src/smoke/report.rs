//! A run in words: what `ys test wait` prints, and what a workflow's `run_test` step leaves
//! for the agent. Written for an agent to act on: the verdict first, then every step that did
//! not pass with why, then the last lines of each server the test started.

use super::exec::{duration_words, indent, last_lines, read_capped, MAX_VARIABLE_BYTES};
use crate::store::{now_ms, TestRunRow, TestStepRow};

/// The whole report.
pub fn summary(run: &TestRunRow, steps: &[TestStepRow]) -> String {
    let mut out = vec![headline(run, steps)];
    match run.status.as_str() {
        "awaiting_approval" => {
            out.push("  waiting for you to approve it in Yardsort".to_owned());
            return out.join("\n");
        }
        "queued" => {
            out.push("  queued; Yardsort starts it in a moment".to_owned());
            return out.join("\n");
        }
        "refused" => {
            out.push("  you chose not to run this file in Yardsort".to_owned());
            return out.join("\n");
        }
        _ => {}
    }
    if let Some(error) = &run.error {
        out.push(format!("  {error}"));
    }
    for step in steps {
        if matches!(step.status.as_str(), "passed" | "pending" | "skipped") {
            continue;
        }
        let took = match (step.started_at, step.ended_at) {
            (Some(from), Some(to)) => format!(
                " after {}",
                duration_words(((to - from) / 1000).max(0) as u64)
            ),
            (Some(_), None) => " and is still going".to_owned(),
            _ => String::new(),
        };
        let phase = if step.phase == "steps" {
            String::new()
        } else {
            format!(" ({})", step.phase)
        };
        out.push(format!(
            "  {}  {}{phase}  {}{took}",
            step.step_id, step.action, step.status
        ));
        if let Some(note) = &step.note {
            out.push(indent(note, "    "));
        }
    }
    if matches!(run.status.as_str(), "failed" | "errored" | "cancelled") {
        for step in steps.iter().filter(|s| s.action == "start") {
            let outputs = step.outputs();
            let tail = ["stderr", "stdout"]
                .iter()
                .filter_map(|name| outputs.get(*name))
                .map(|path| read_capped(std::path::Path::new(path), MAX_VARIABLE_BYTES))
                .map(|text| last_lines(&text, 8))
                .find(|text| !text.trim().is_empty());
            if let Some(tail) = tail {
                out.push(format!(
                    "    last lines of `{}` ({} step):",
                    step.step_id, step.phase
                ));
                out.push(indent(&tail, "      "));
            }
        }
    }
    out.join("\n")
}

/// `failed  login  (3 of 6 steps passed, 1m 12s)`.
pub fn headline(run: &TestRunRow, steps: &[TestStepRow]) -> String {
    let counted: Vec<&TestStepRow> = steps.iter().filter(|s| s.phase != "teardown").collect();
    let passed = counted.iter().filter(|s| s.status == "passed").count();
    let took = match (run.started_at, run.ended_at) {
        (Some(from), Some(to)) => duration_words(((to - from) / 1000).max(0) as u64),
        (Some(from), None) => format!(
            "{} so far",
            duration_words(((now_ms() - from) / 1000).max(0) as u64)
        ),
        _ => "not started".to_owned(),
    };
    format!(
        "{}  {}  ({passed} of {} steps passed, {took})",
        run.status,
        run.test_id,
        counted.len()
    )
}

/// What `ys test wait` exits with: 0 passed, 1 failed, 2 anything else.
pub fn exit_code(status: &str) -> i32 {
    match status {
        "passed" => 0,
        "failed" => 1,
        _ => 2,
    }
}

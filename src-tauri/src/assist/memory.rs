//! Reading a project's memory proposals with Jev: does this one repeat an approved entry, and
//! does it contradict one?
//!
//! Design 09 gives Jev typed signals for the memory review queue and no more: it never writes
//! an entry and never approves one. Two questions, one criterion each — the lesson of stage 3's
//! unaccounted question, where a compound ask made Jev hedge. What is sent is the proposal and
//! the project's approved entries, and only with the switch on.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use specta::Type;

use super::jev::{Answer, Answers, Jev, JevError, Question};
use super::{Cache, Thresholds};

/// Bumped whenever the questions change, so answers to older wording are not reused.
const QUESTIONS_VERSION: u32 = 1;
/// Approved entries beyond these are not sent: the newest are the ones a proposal most often
/// repeats, and the state Jev reads has a size.
const MAX_APPROVED: usize = 60;

/// What Jev said about one proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCheck {
    pub id: String,
    /// It says what an approved entry already says.
    pub repeats: bool,
    /// It says the opposite of an approved entry, or something that cannot also be true.
    pub contradicts: bool,
}

fn questions() -> BTreeMap<String, Question> {
    let mut questions = BTreeMap::new();
    questions.insert(
        "repeats".to_owned(),
        Question::Noul {
            instructions: "Does `candidate` say the same thing as one of the entries in \
                           `approved`, so that keeping it would add nothing new?"
                .into(),
        },
    );
    questions.insert(
        "contradicts".to_owned(),
        Question::Noul {
            instructions: "Does `candidate` contradict one of the entries in `approved` — say \
                           the opposite of it, or something that cannot also be true?"
                .into(),
        },
    );
    questions
}

fn state_for(candidate: &str, approved: &[String]) -> serde_json::Value {
    let listed: Vec<String> = approved
        .iter()
        .take(MAX_APPROVED)
        .enumerate()
        .map(|(n, text)| format!("{}. {text}", n + 1))
        .collect();
    serde_json::json!({ "candidate": candidate, "approved": listed.join("\n") })
}

fn cache_key(candidate: &str, approved: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (QUESTIONS_VERSION, super::jev::MODEL, candidate).hash(&mut hasher);
    approved
        .iter()
        .take(MAX_APPROVED)
        .for_each(|a| a.hash(&mut hasher));
    hasher.finish()
}

fn verdict(id: &str, answers: &Answers, thresholds: Thresholds) -> MemoryCheck {
    let yes = |q: &str| {
        answers
            .get(q)
            .and_then(Answer::yes)
            .is_some_and(|p| p >= thresholds.flag_at())
    };
    MemoryCheck {
        id: id.to_owned(),
        repeats: yes("repeats"),
        contradicts: yes("contradicts"),
    }
}

/// Check each `(id, text)` candidate against the approved entries, newest first. With nothing
/// approved there is nothing to repeat or contradict, and nothing is sent.
pub async fn check(
    jev: Arc<Jev>,
    cache: &Cache<Answers>,
    candidates: Vec<(String, String)>,
    approved: Vec<String>,
    thresholds: Thresholds,
) -> Result<Vec<MemoryCheck>, JevError> {
    let mut checks = Vec::with_capacity(candidates.len());
    if approved.is_empty() {
        return Ok(candidates
            .into_iter()
            .map(|(id, _)| MemoryCheck {
                id,
                repeats: false,
                contradicts: false,
            })
            .collect());
    }
    for (id, text) in candidates {
        let key = cache_key(&text, &approved);
        let answers = match cache.get(key) {
            Some(answers) => answers,
            None => {
                let answers = jev.ask(&state_for(&text, &approved), &questions()).await?;
                cache.put(key, answers.clone());
                answers
            }
        };
        checks.push(verdict(&id, &answers, thresholds));
    }
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use super::super::jev::fake;
    use super::*;

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    /// Each proposal is asked the two questions against the approved entries; the answers are
    /// read at the flag threshold; an unchanged proposal is not asked twice; and with nothing
    /// approved nothing is sent at all.
    #[test]
    fn proposals_are_checked_against_the_approved_entries_and_cached() {
        // Answers depend on the proposal, so the stand-in reads the state it was sent.
        let server = fake::serve(|request| {
            let candidate = request.body["state"]["candidate"].as_str().unwrap_or("");
            let (repeats, contradicts) = match candidate {
                "The tests need TZ=UTC set." => (0.92, 0.05),
                "Tests run in local time." => (0.05, 0.88),
                _ => (0.05, 0.05),
            };
            (
                200,
                serde_json::json!({
                    "model": super::super::jev::MODEL,
                    "answers": { "repeats": fake::noul(repeats), "contradicts": fake::noul(contradicts) },
                    "usage": {"input_tokens": 100, "output_tokens": 10},
                }),
            )
        });
        let cache = Cache::default();
        let approved = vec!["The tests need TZ=UTC.".to_owned()];
        let ask = |candidates: Vec<(&str, &str)>, approved: Vec<String>| {
            block(check(
                Arc::new(Jev::at(&server.url, "k")),
                &cache,
                candidates
                    .into_iter()
                    .map(|(id, text)| (id.to_owned(), text.to_owned()))
                    .collect(),
                approved,
                Thresholds::default(),
            ))
            .unwrap()
        };
        let checks = ask(
            vec![
                ("a", "The tests need TZ=UTC set."),
                ("b", "Tests run in local time."),
                ("c", "Use bun, not npm."),
            ],
            approved.clone(),
        );
        assert_eq!(
            checks
                .iter()
                .map(|c| (c.id.as_str(), c.repeats, c.contradicts))
                .collect::<Vec<_>>(),
            [("a", true, false), ("b", false, true), ("c", false, false)]
        );
        let sent = server.received.lock().unwrap().clone();
        assert_eq!(sent.len(), 3);
        assert_eq!(
            sent[0].body["state"]["approved"],
            "1. The tests need TZ=UTC."
        );
        assert!(sent[0].body["questions"]["repeats"].is_object());

        ask(vec![("a", "The tests need TZ=UTC set.")], approved);
        assert_eq!(server.received.lock().unwrap().len(), 3, "cached");
        let none = ask(vec![("d", "Anything.")], vec![]);
        assert_eq!((none[0].repeats, none[0].contradicts), (false, false));
        assert_eq!(
            server.received.lock().unwrap().len(),
            3,
            "nothing approved, nothing sent"
        );
    }
}

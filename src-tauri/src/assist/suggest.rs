//! Composer suggestions: which harness suits the message being typed, and how much effort it
//! deserves.
//!
//! Only two things are sent: the message, and the "Good at" descriptions the user wrote for their
//! harnesses. A harness without one is never suggested — which agent suits which work is the
//! user's judgment to record, not ours to invent.

use std::collections::BTreeMap;

use serde::Serialize;
use specta::Type;

use super::jev::{Answer, Answers, Jev, JevError, Question};
use super::Thresholds;

/// Shorter than this and there is nothing to judge.
const MIN_MESSAGE_CHARS: usize = 15;
/// Picked when a harness offers these; otherwise the levels are spread over what it does offer.
const LOW_MEDIUM_HIGH: [&str; 3] = ["low", "medium", "high"];
/// The option that means "none of these harnesses stands out".
const NO_PREFERENCE: &str = "No description fits this task better than the others";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// The harness whose "Good at" fits best, when one clearly does.
    pub harness_id: Option<String>,
    /// The effort to offer, per harness id — the level depends on what each harness offers.
    pub effort_by_harness: BTreeMap<String, String>,
}

impl Suggestion {
    fn is_empty(&self) -> bool {
        self.harness_id.is_none() && self.effort_by_harness.is_empty()
    }
}

/// A harness as the suggestion needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    /// What the user says it is good at. Empty means it takes no part in the harness question.
    pub strengths: String,
    pub efforts: Vec<String>,
}

fn questions(named: &BTreeMap<String, &Candidate>) -> BTreeMap<String, Question> {
    let mut questions = BTreeMap::from([(
        "difficulty".to_owned(),
        Question::Score {
            instructions: "How much careful work does the request in `task` need from a coding \
                           agent?"
                .into(),
            criteria: vec![
                "A small, contained job: a typo, a rename, a one-line fix, a question about the \
                 code, or a change in a single obvious place."
                    .into(),
                "An ordinary job: a feature or a bug fix that touches a few files and needs some \
                 care, but no deep investigation."
                    .into(),
                "A demanding job: a design decision, a change across many parts of a codebase, a \
                 subtle bug, a migration, or work where a mistake is expensive."
                    .into(),
            ],
        },
    )]);
    if named.len() >= 2 {
        let mut criteria: BTreeMap<String, String> = named
            .iter()
            .map(|(name, candidate)| (name.clone(), candidate.strengths.clone()))
            .collect();
        criteria.insert(
            NO_PREFERENCE.to_owned(),
            "The work in `task` matches none of the descriptions above more than the others."
                .to_owned(),
        );
        questions.insert(
            "harness".to_owned(),
            Question::Choice {
                instructions: "Which coding agent is described as best suited to the work in \
                               `task`?"
                    .into(),
                criteria,
            },
        );
    }
    questions
}

/// The effort level to offer for a difficulty (0 easiest), given what a harness offers.
fn effort_for(level: usize, efforts: &[String]) -> Option<String> {
    if efforts.is_empty() {
        return None;
    }
    let known: Vec<&String> = LOW_MEDIUM_HIGH
        .iter()
        .filter_map(|name| {
            efforts
                .iter()
                .find(|effort| effort.eq_ignore_ascii_case(name))
        })
        .collect();
    if known.len() == LOW_MEDIUM_HIGH.len() {
        return Some(known[level.min(2)].clone());
    }
    // An unfamiliar set of levels: spread the three difficulties over it, ends included.
    let last = efforts.len() - 1;
    Some(efforts[(level.min(2) * last).div_ceil(2)].clone())
}

fn read(
    answers: &Answers,
    candidates: &[Candidate],
    named: &BTreeMap<String, &Candidate>,
    thresholds: Thresholds,
) -> Suggestion {
    let harness_id = match answers.get("harness") {
        Some(Answer::Choice {
            choice, confidence, ..
        }) if *confidence >= thresholds.suggest_at() && choice != NO_PREFERENCE => {
            named.get(choice.as_str()).map(|c| c.id.clone())
        }
        _ => None,
    };
    let effort_by_harness = answers
        .get("difficulty")
        .and_then(Answer::likeliest_level)
        .filter(|(_, probability)| *probability >= thresholds.suggest_at())
        .map(|(level, _)| {
            candidates
                .iter()
                .filter_map(|c| Some((c.id.clone(), effort_for(level, &c.efforts)?)))
                .collect()
        })
        .unwrap_or_default();
    Suggestion {
        harness_id,
        effort_by_harness,
    }
}

/// What to offer for this message, or `None` when there is nothing worth offering.
pub async fn suggest(
    jev: &Jev,
    message: &str,
    candidates: &[Candidate],
    thresholds: Thresholds,
) -> Result<Option<Suggestion>, JevError> {
    let message = message.trim();
    if message.chars().count() < MIN_MESSAGE_CHARS || candidates.is_empty() {
        return Ok(None);
    }
    // Choice options are what the model reads, so they are the labels, not our internal ids.
    let named: BTreeMap<String, &Candidate> = candidates
        .iter()
        .filter(|candidate| !candidate.strengths.trim().is_empty())
        .map(|candidate| (candidate.label.clone(), candidate))
        .collect();
    let questions = questions(&named);
    let state = serde_json::json!({ "task": message });
    let answers = jev.ask(&state, &questions).await?;
    let suggestion = read(&answers, candidates, &named, thresholds);
    Ok(Some(suggestion).filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::super::jev::fake;
    use super::*;

    fn candidate(id: &str, label: &str, strengths: &str, efforts: &[&str]) -> Candidate {
        Candidate {
            id: id.to_owned(),
            label: label.to_owned(),
            strengths: strengths.to_owned(),
            efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
        }
    }

    fn harnesses() -> Vec<Candidate> {
        vec![
            candidate(
                "claude",
                "Claude Code",
                "Long refactors and tricky debugging.",
                &["low", "medium", "high", "xhigh", "max"],
            ),
            candidate(
                "codex",
                "Codex",
                "Quick, well-specified edits.",
                &["low", "medium", "high"],
            ),
            candidate("opencode", "OpenCode", "", &[]),
        ]
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn a_harness_and_an_effort_are_offered_from_the_message_alone() {
        let server = fake::answering(|id, _| match id {
            "harness" => fake::choice("Claude Code", 0.82),
            _ => fake::score(&[0.05, 0.15, 0.8]),
        });
        let suggestion = block(suggest(
            &Jev::at(&server.url, "k"),
            "  Track down why the worktree reconciler drops adopted branches  ",
            &harnesses(),
            Thresholds::default(),
        ))
        .unwrap()
        .unwrap();

        assert_eq!(suggestion.harness_id.as_deref(), Some("claude"));
        assert_eq!(suggestion.effort_by_harness["claude"], "high");
        assert_eq!(suggestion.effort_by_harness["codex"], "high");
        assert!(
            !suggestion.effort_by_harness.contains_key("opencode"),
            "a harness with no effort levels gets no suggestion"
        );

        let sent = server.received.lock().unwrap()[0].clone();
        assert_eq!(
            sent.body["state"]["task"],
            "Track down why the worktree reconciler drops adopted branches"
        );
        let options = &sent.body["questions"]["harness"]["criteria"];
        assert_eq!(
            options["Claude Code"],
            "Long refactors and tricky debugging."
        );
        assert!(options["OpenCode"].is_null(), "no description, no option");
        assert!(!options[NO_PREFERENCE].is_null());
    }

    #[test]
    fn an_easy_task_gets_the_lowest_effort_and_an_unsure_answer_gets_none() {
        let easy = fake::answering(|id, _| match id {
            "harness" => fake::choice(NO_PREFERENCE, 0.9),
            _ => fake::score(&[0.8, 0.15, 0.05]),
        });
        let suggestion = block(suggest(
            &Jev::at(&easy.url, "k"),
            "Fix the typo in the README title",
            &harnesses(),
            Thresholds::default(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            suggestion.harness_id, None,
            "\"none of these\" is not a suggestion"
        );
        assert_eq!(suggestion.effort_by_harness["claude"], "low");

        let muddled = fake::answering(|id, _| match id {
            "harness" => fake::choice("Codex", 0.3),
            _ => fake::score(&[0.4, 0.35, 0.25]),
        });
        assert_eq!(
            block(suggest(
                &Jev::at(&muddled.url, "k"),
                "Have a look at the parser",
                &harnesses(),
                Thresholds::default(),
            ))
            .unwrap(),
            None,
            "nothing clear to say means nothing is shown"
        );
    }

    #[test]
    fn a_short_message_is_never_sent() {
        let server = fake::answering(|_, _| fake::noul(0.5));
        assert_eq!(
            block(suggest(
                &Jev::at(&server.url, "k"),
                "fix tests",
                &harnesses(),
                Thresholds::default(),
            ))
            .unwrap(),
            None
        );
        assert!(server.received.lock().unwrap().is_empty());
    }

    #[test]
    fn with_fewer_than_two_described_harnesses_only_the_effort_is_asked() {
        let server = fake::answering(|_, _| fake::score(&[0.05, 0.8, 0.15]));
        let only_one = vec![
            harnesses()[0].clone(),
            candidate("codex", "Codex", "", &["low"]),
        ];
        let suggestion = block(suggest(
            &Jev::at(&server.url, "k"),
            "Add a --json flag to the status command",
            &only_one,
            Thresholds::default(),
        ))
        .unwrap()
        .unwrap();

        assert_eq!(suggestion.harness_id, None);
        assert_eq!(suggestion.effort_by_harness["claude"], "medium");
        assert!(server.received.lock().unwrap()[0].body["questions"]["harness"].is_null());
    }

    #[test]
    fn effort_levels_are_mapped_onto_whatever_a_harness_offers() {
        let named: Vec<String> = ["low", "medium", "high", "xhigh"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(effort_for(0, &named).unwrap(), "low");
        assert_eq!(effort_for(2, &named).unwrap(), "high");

        let unusual: Vec<String> = ["fast", "balanced", "thorough", "exhaustive"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(effort_for(0, &unusual).unwrap(), "fast");
        assert_eq!(effort_for(1, &unusual).unwrap(), "thorough");
        assert_eq!(effort_for(2, &unusual).unwrap(), "exhaustive");

        let pair = vec!["quick".to_owned(), "deep".to_owned()];
        assert_eq!(effort_for(0, &pair).unwrap(), "quick");
        assert_eq!(effort_for(2, &pair).unwrap(), "deep");
        assert_eq!(effort_for(1, &[]), None);
    }
}

//! Reading a workspace's changes with Jev: is this file part of what was asked, and does it do
//! something risky?
//!
//! The judgments are deliberately narrow — one question per property, one request per file — and
//! everything else is ordinary code: git produces the diffs, the thresholds below turn
//! probabilities into flags, and a file whose *name* says "credentials" is flagged without its
//! contents ever leaving the machine.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use specta::Type;

use super::jev::{Answer, Answers, Jev, JevError, Question};
use super::{Cache, Thresholds};
use crate::changes::{ChangeKind, FileChange, Scope};

/// Beyond this a diff is cut short; Jev reads 32k tokens of state at most, and a judgment about a
/// change does not improve with the tail of a huge one.
const MAX_PATCH_CHARS: usize = 40_000;
/// More changed files than this and we stop asking: the rest are listed as not checked.
const MAX_FILES: usize = 80;
/// How many files are asked about at once. Well inside TypeSafe's rate limits.
const CONCURRENCY: usize = 6;
/// Below this the model is not saying anything useful about relevance.
const UNSURE_BELOW: f64 = 0.5;
/// Bumped whenever the questions change, so cached verdicts from older wording are not reused.
const QUESTIONS_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Relevance {
    /// Does what the task asks for.
    Direct,
    /// Not the task itself, but plausibly needed for it.
    Supporting,
    /// Nothing to do with the task.
    Unrelated,
    /// The model had no clear read.
    Unsure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReviewFlag {
    /// A file whose name says it holds credentials. Decided here; its contents are never sent.
    CredentialsFile,
    /// The change adds a literal secret.
    Secret,
    /// Tests were removed, skipped, or made weaker.
    WeakensTests,
    /// A lint, type check or CI step was switched off.
    DisablesChecks,
    /// A substantive change no agent reported making, in a workspace whose agents were
    /// reporting what they wrote. Only asked when the provenance facts are sent.
    Unaccounted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileReview {
    pub path: String,
    pub scope: Scope,
    /// `None` when the workspace's task is not known, or the file was not checked.
    pub relevance: Option<Relevance>,
    pub flags: Vec<ReviewFlag>,
    /// Why this file was not looked at, if it was not.
    pub not_checked: Option<String>,
    /// What Jev was told about who wrote the file, word for word, when that was sent.
    pub told: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub files: Vec<FileReview>,
    /// What the workspace was asked to do, as far as Yardsort knows: the first message of each
    /// of its conversations. Without it, only the risk checks run.
    pub task: Option<String>,
    pub model: String,
    /// A failure that left part of the review undone.
    pub problem: Option<String>,
}

/// One file to ask about: what git says changed, and the diff to judge.
#[derive(Debug, Clone, PartialEq)]
pub struct FileInput {
    pub path: String,
    pub scope: Scope,
    pub kind: ChangeKind,
    pub patch: Option<String>,
    /// The name alone is enough to flag it; the diff is not sent.
    pub credentials_file: bool,
    /// What the agents reported about this file, in the Changes list's own words, when the
    /// user chose to send that; `None` sends nothing new.
    pub written: Option<Written>,
}

/// Who wrote a changed file, as far as the agents' reports say. The words sent to Jev are
/// exactly the ones the Changes list shows; nothing about tools, times or commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// One or more agents reported writing it.
    Reported(Vec<String>),
    /// No report, but its last write fell inside an agent's command.
    Observed(String),
    /// No report and no observation, while agents in the workspace were reporting.
    Unaccounted,
}

impl Written {
    /// The sentence Jev is given as `written`.
    pub fn sentence(&self) -> String {
        match self {
            Written::Reported(agents) => format!("reported written by {}", agents.join(" and ")),
            Written::Observed(agent) => {
                format!("last written while {agent} ran a command; no agent reported writing it")
            }
            Written::Unaccounted => "not reported written by any agent, though the agents in \
                                     this workspace were reporting what they wrote"
                .to_owned(),
        }
    }
}

impl FileInput {
    pub fn new(change: &FileChange, scope: Scope, patch: Option<String>) -> Self {
        let credentials_file = looks_like_credentials(&change.path);
        Self {
            path: change.path.clone(),
            scope,
            kind: change.kind,
            patch: patch.filter(|_| !credentials_file).map(truncate),
            credentials_file,
            written: None,
        }
    }
}

/// Files that hold secrets by convention. Example and template files are not among them.
fn looks_like_credentials(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name.starts_with(".env") {
        return !["example", "sample", "template", "dist", "defaults"]
            .iter()
            .any(|suffix| name.ends_with(suffix));
    }
    let extension = name.rsplit('.').next().unwrap_or_default();
    ["pem", "key", "p12", "pfx", "keystore", "jks"].contains(&extension)
        || [".netrc", "_netrc", ".npmrc", ".pypirc", "credentials"].contains(&name.as_str())
        || (name.starts_with("id_") && !name.ends_with(".pub"))
}

fn truncate(patch: String) -> String {
    if patch.chars().count() <= MAX_PATCH_CHARS {
        return patch;
    }
    let kept: String = patch.chars().take(MAX_PATCH_CHARS).collect();
    format!("{kept}\n… the rest of this diff was left out because it is very long.")
}

/// What the whole file is judged by. Cached under a hash of exactly these inputs.
fn state_for(input: &FileInput, task: Option<&str>) -> serde_json::Value {
    let mut state = serde_json::Map::new();
    if let Some(task) = task {
        state.insert("task".into(), task.into());
    }
    state.insert("file".into(), input.path.clone().into());
    state.insert("change".into(), kind_word(input.kind).into());
    state.insert(
        "diff".into(),
        input.patch.clone().unwrap_or_default().into(),
    );
    if let Some(written) = &input.written {
        state.insert("written".into(), written.sentence().into());
    }
    serde_json::Value::Object(state)
}

fn kind_word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Conflicted => "conflicted",
    }
}

fn questions(task_known: bool, unaccounted: bool) -> BTreeMap<String, Question> {
    let mut questions = BTreeMap::new();
    if unaccounted {
        questions.insert(
            "unaccounted".to_owned(),
            Question::Noul {
                instructions: "Does `diff` add or change something with meaning — code, text, \
                               configuration, tests, dependencies or documentation? Answer no \
                               only if the change is nothing but formatting, whitespace, \
                               generated output, or a lock file. (`written` says no agent \
                               reported writing this file, although the agents in this \
                               workspace were reporting what they wrote; that is already known \
                               and is not the question.)"
                    .into(),
            },
        );
    }
    if task_known {
        questions.insert(
            "relevance".to_owned(),
            Question::Score {
                instructions: "How does the change shown in `diff` to the file named by `file` \
                               relate to the work described in `task`?"
                    .into(),
                criteria: vec![
                    "The change does something that `task` does not ask for and does not need."
                        .into(),
                    "The change is not what `task` asks for, but work described in `task` \
                     plausibly needs it: tests, configuration, types, imports, dependencies, \
                     error handling or documentation for that work."
                        .into(),
                    "The change does part of what `task` asks for.".into(),
                ],
            },
        );
    }
    questions.insert(
        "secret".to_owned(),
        Question::Noul {
            instructions: "Does `diff` add or change a line containing a literal secret value — \
                           an API key, access token, password, connection string with a password, \
                           or private key? A name of an environment variable, a placeholder, or \
                           an obvious example value is not a secret."
                .into(),
        },
    );
    questions.insert(
        "tests".to_owned(),
        Question::Noul {
            instructions: "Does `diff` delete a test, skip or disable a test, or weaken what an \
                           existing test asserts?"
                .into(),
        },
    );
    questions.insert(
        "checks".to_owned(),
        Question::Noul {
            instructions: "Does `diff` switch off a check that would otherwise report problems? \
                           For example: adding a lint suppression such as eslint-disable, \
                           #[allow(...)] or noqa; silencing a type error with @ts-ignore, any or \
                           unwrap-style casts; lowering a compiler or linter setting; or making \
                           a build or CI step optional or skipped."
                .into(),
        },
    );
    questions
}

/// Turn the answers into the flags and relevance the panel shows. All policy, no model: the
/// same answers read against different thresholds give different badges, and cost nothing.
fn verdict(answers: &Answers, thresholds: Thresholds) -> (Option<Relevance>, Vec<ReviewFlag>) {
    let mut flags = Vec::new();
    let mut flag_if = |id: &str, flag: ReviewFlag| {
        if answers
            .get(id)
            .and_then(Answer::yes)
            .is_some_and(|p| p >= thresholds.flag_at())
        {
            flags.push(flag);
        }
    };
    flag_if("secret", ReviewFlag::Secret);
    flag_if("tests", ReviewFlag::WeakensTests);
    flag_if("checks", ReviewFlag::DisablesChecks);
    flag_if("unaccounted", ReviewFlag::Unaccounted);

    let relevance = answers.get("relevance").map(|answer| {
        let unrelated = answer.level(0).unwrap_or(0.0);
        let (level, probability) = answer.likeliest_level().unwrap_or((0, 0.0));
        if unrelated >= thresholds.off_task_at() {
            Relevance::Unrelated
        } else if probability < UNSURE_BELOW {
            Relevance::Unsure
        } else {
            match level {
                0 => Relevance::Unrelated,
                1 => Relevance::Supporting,
                _ => Relevance::Direct,
            }
        }
    });
    (relevance, flags)
}

fn cache_key(input: &FileInput, task: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (QUESTIONS_VERSION, super::jev::MODEL).hash(&mut hasher);
    (task, &input.path, &input.patch).hash(&mut hasher);
    (input.kind as u8, input.scope as u8).hash(&mut hasher);
    // What Jev was told about the writer is part of what it answered about.
    input
        .written
        .as_ref()
        .map(Written::sentence)
        .hash(&mut hasher);
    hasher.finish()
}

/// Ask about every file that has something to ask about, a few at a time.
pub async fn review(
    jev: Arc<Jev>,
    cache: &Cache<Answers>,
    task: Option<String>,
    inputs: Vec<FileInput>,
    thresholds: Thresholds,
) -> Result<Review, JevError> {
    let mut files: Vec<FileReview> = Vec::with_capacity(inputs.len());
    let mut asking = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let not_checked = if input.credentials_file {
            None
        } else if input.patch.is_none() {
            Some("There is nothing to read in this change.".to_owned())
        } else if asking.len() >= MAX_FILES {
            Some(format!(
                "Only the first {MAX_FILES} changed files are checked."
            ))
        } else {
            None
        };
        let cached = cache.get(cache_key(input, task.as_deref()));
        if not_checked.is_none() && !input.credentials_file && cached.is_none() {
            asking.push((index, input.clone()));
        }
        let (relevance, mut flags) = cached
            .map(|answers| verdict(&answers, thresholds))
            .unwrap_or_default();
        if input.credentials_file {
            flags.push(ReviewFlag::CredentialsFile);
        }
        // `told` is what left the machine about this file. A credentials file and a file with
        // nothing to read are never sent, sentence included, so nothing was told of them.
        let sent = !input.credentials_file && not_checked.is_none();
        files.push(FileReview {
            path: input.path.clone(),
            scope: input.scope,
            relevance,
            flags,
            not_checked,
            told: input
                .written
                .as_ref()
                .filter(|_| sent)
                .map(Written::sentence),
        });
    }

    let mut problem = None;
    for batch in asking.chunks(CONCURRENCY) {
        let mut running = tokio::task::JoinSet::new();
        for (index, input) in batch.iter().cloned() {
            let (jev, task) = (Arc::clone(&jev), task.clone());
            running.spawn(async move {
                let questions =
                    questions(task.is_some(), input.written == Some(Written::Unaccounted));
                let state = state_for(&input, task.as_deref());
                (index, input, jev.ask(&state, &questions).await)
            });
        }
        while let Some(finished) = running.join_next().await {
            let Ok((index, input, answered)) = finished else {
                continue;
            };
            match answered {
                Ok(answers) => {
                    // A dev build says what Jev answered, per question, so a badge that does
                    // or does not appear can be traced to a number: the path and the
                    // probabilities, never the diff.
                    if cfg!(debug_assertions) {
                        let said: Vec<String> = answers
                            .answers
                            .iter()
                            .map(|(id, answer)| match answer.yes() {
                                Some(p) => format!("{id}={p:.2}"),
                                None => format!(
                                    "{id}=[{}]",
                                    (0..3)
                                        .map(|i| format!("{:.2}", answer.level(i).unwrap_or(0.0)))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                ),
                            })
                            .collect();
                        eprintln!("assist review {}: {}", input.path, said.join(" "));
                    }
                    let (relevance, flags) = verdict(&answers, thresholds);
                    cache.put(cache_key(&input, task.as_deref()), answers);
                    files[index].relevance = relevance;
                    files[index].flags = flags;
                }
                // A key that stopped working stops the whole review; anything else leaves the
                // files we did read in place and says what went wrong.
                Err(JevError::BadKey) => return Err(JevError::BadKey),
                Err(error) => {
                    files[index].not_checked = Some(error.to_string());
                    problem.get_or_insert_with(|| error.to_string());
                }
            }
        }
    }

    Ok(Review {
        files,
        task,
        model: super::jev::MODEL.to_owned(),
        problem,
    })
}

#[cfg(test)]
mod tests {
    use super::super::jev::fake;
    use super::*;

    fn input(path: &str, patch: &str) -> FileInput {
        FileInput::new(
            &FileChange {
                path: path.to_owned(),
                old_path: None,
                kind: ChangeKind::Modified,
                additions: None,
                deletions: None,
            },
            Scope::Uncommitted,
            Some(patch.to_owned()),
        )
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn files_are_judged_against_the_task_and_flagged_for_risky_edits() {
        // The stand-in answers by file: the state carries the path in `file`.
        let server = fake::answering(|id, _| match id {
            "relevance" => fake::score(&[0.05, 0.15, 0.8]),
            "tests" => fake::noul(0.9),
            _ => fake::noul(0.02),
        });
        let cache = Cache::default();
        let checked = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &cache,
            Some("Fix the login redirect".to_owned()),
            vec![input("src/login.rs", "-old\n+new\n")],
            Thresholds::default(),
        ))
        .unwrap();

        assert_eq!(checked.files[0].relevance, Some(Relevance::Direct));
        assert_eq!(checked.files[0].flags, [ReviewFlag::WeakensTests]);
        assert_eq!(checked.problem, None);
        assert_eq!(checked.model, super::super::jev::MODEL);

        let sent = server.received.lock().unwrap()[0].clone();
        assert_eq!(sent.body["state"]["task"], "Fix the login redirect");
        assert_eq!(sent.body["state"]["file"], "src/login.rs");
        assert_eq!(sent.body["state"]["change"], "modified");
        assert_eq!(sent.body["state"]["diff"], "-old\n+new\n");
        assert_eq!(
            sent.body["questions"]["relevance"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    /// With the switch on, Jev is told who wrote the file in the Changes list's words and no
    /// more; a file no agent accounted for gets the one extra question, and its answer the one
    /// extra badge. With nothing to tell, nothing new is sent and nothing new is asked.
    #[test]
    fn who_wrote_the_file_is_told_in_the_lists_words_and_an_unaccounted_change_is_asked_about() {
        let server = fake::answering(|id, _| match id {
            "unaccounted" => fake::noul(0.9),
            "relevance" => fake::score(&[0.1, 0.2, 0.7]),
            _ => fake::noul(0.0),
        });
        let mut reported = input("src/login.rs", "-old\n+new\n");
        reported.written = Some(Written::Reported(vec!["claude".into()]));
        let mut observed = input("hello.txt", "+hello\n");
        observed.written = Some(Written::Observed("claude".into()));
        let mut stray = input("ci.yml", "-on: push\n");
        stray.written = Some(Written::Unaccounted);
        let silent = input("README.md", "+# hi\n");
        let checked = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &Cache::default(),
            Some("Fix the login redirect".to_owned()),
            vec![reported, observed, stray, silent],
            Thresholds::default(),
        ))
        .unwrap();

        assert_eq!(
            checked.files[0].told.as_deref(),
            Some("reported written by claude")
        );
        assert_eq!(
            checked.files[1].told.as_deref(),
            Some("last written while claude ran a command; no agent reported writing it")
        );
        assert!(checked.files[2]
            .told
            .as_deref()
            .unwrap()
            .starts_with("not reported written by any agent"));
        assert_eq!(checked.files[3].told, None);
        assert_eq!(checked.files[2].flags, [ReviewFlag::Unaccounted]);
        // A credentials file is never sent, its sentence included: nothing was told of it.
        let mut env = input(".env", "+KEY=hunter2\n");
        env.written = Some(Written::Reported(vec!["claude".into()]));
        let before = server.received.lock().unwrap().len();
        let excluded = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &Cache::default(),
            None,
            vec![env],
            Thresholds::default(),
        ))
        .unwrap();
        assert_eq!(excluded.files[0].flags, [ReviewFlag::CredentialsFile]);
        assert_eq!(excluded.files[0].told, None, "not sent, so not told");
        assert_eq!(server.received.lock().unwrap().len(), before, "no request");
        assert_eq!(checked.files[0].flags, [], "a reported file is not asked");
        assert_eq!(checked.files[1].flags, []);

        let sent = server.received.lock().unwrap();
        let by = |file: &str| {
            sent.iter()
                .find(|r| r.body["state"]["file"] == file)
                .unwrap()
                .body
                .clone()
        };
        assert_eq!(
            by("src/login.rs")["state"]["written"],
            "reported written by claude"
        );
        assert!(by("src/login.rs")["questions"]["unaccounted"].is_null());
        assert!(by("hello.txt")["questions"]["unaccounted"].is_null());
        assert!(!by("ci.yml")["questions"]["unaccounted"].is_null());
        assert!(
            by("README.md")["state"]["written"].is_null(),
            "nothing to tell, nothing sent"
        );
        assert!(by("README.md")["questions"]["unaccounted"].is_null());
        for request in sent.iter() {
            let text = request.body.to_string();
            for never in ["Bash", "toolUseId", "07:", "echo"] {
                assert!(!text.contains(never), "sent {never}: {text}");
            }
        }
    }

    #[test]
    fn a_file_the_task_does_not_call_for_is_marked_unrelated_and_an_unsure_model_says_so() {
        let unrelated = fake::answering(|id, _| match id {
            "relevance" => fake::score(&[0.7, 0.2, 0.1]),
            _ => fake::noul(0.0),
        });
        let muddled = fake::answering(|id, _| match id {
            "relevance" => fake::score(&[0.4, 0.35, 0.25]),
            _ => fake::noul(0.0),
        });
        let ask = |url: &str| {
            block(review(
                Arc::new(Jev::at(url, "k")),
                &Cache::default(),
                Some("Fix the login redirect".to_owned()),
                vec![input("ci.yml", "-on: push\n")],
                Thresholds::default(),
            ))
            .unwrap()
            .files[0]
                .relevance
        };
        assert_eq!(ask(&unrelated.url), Some(Relevance::Unrelated));
        assert_eq!(ask(&muddled.url), Some(Relevance::Unsure));
    }

    #[test]
    fn without_a_task_only_the_risk_checks_are_asked() {
        let server = fake::answering(|_, _| fake::noul(0.95));
        let checked = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &Cache::default(),
            None,
            vec![input("src/api.rs", "+const KEY = \"sk-live-123\";\n")],
            Thresholds::default(),
        ))
        .unwrap();

        assert_eq!(checked.files[0].relevance, None);
        assert_eq!(
            checked.files[0].flags,
            [
                ReviewFlag::Secret,
                ReviewFlag::WeakensTests,
                ReviewFlag::DisablesChecks
            ]
        );
        let sent = server.received.lock().unwrap()[0].clone();
        assert!(sent.body["questions"]["relevance"].is_null());
        assert!(sent.body["state"]["task"].is_null());
    }

    #[test]
    fn a_credentials_file_is_flagged_without_its_contents_being_sent() {
        let server = fake::answering(|_, _| fake::noul(0.0));
        let checked = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &Cache::default(),
            Some("Add staging config".to_owned()),
            vec![
                input(".env", "+DATABASE_URL=postgres://user:hunter2@host/db\n"),
                input(".env.example", "+DATABASE_URL=\n"),
                input("keys/server.pem", "+-----BEGIN PRIVATE KEY-----\n"),
            ],
            Thresholds::default(),
        ))
        .unwrap();

        assert_eq!(checked.files[0].flags, [ReviewFlag::CredentialsFile]);
        assert_eq!(checked.files[2].flags, [ReviewFlag::CredentialsFile]);
        assert_eq!(
            checked.files[1].flags,
            [],
            "an example file is not a secret"
        );
        let sent = server.received.lock().unwrap();
        assert_eq!(sent.len(), 1, "only .env.example was sent");
        assert_eq!(sent[0].body["state"]["file"], ".env.example");
        assert!(
            !format!("{:?}", sent[0].body).contains("hunter2"),
            "no credentials left the machine"
        );
    }

    #[test]
    fn a_file_that_has_not_changed_since_last_time_is_not_asked_about_again() {
        let server = fake::answering(|id, _| match id {
            "relevance" => fake::score(&[0.05, 0.15, 0.8]),
            _ => fake::noul(0.0),
        });
        let cache = Cache::default();
        let jev = Arc::new(Jev::at(&server.url, "k"));
        let task = Some("Fix the login redirect".to_owned());
        let first = vec![input("a.rs", "-old\n+new\n"), input("b.rs", "+one\n")];
        block(review(
            Arc::clone(&jev),
            &cache,
            task.clone(),
            first.clone(),
            Thresholds::default(),
        ))
        .unwrap();
        assert_eq!(server.received.lock().unwrap().len(), 2);

        let second = vec![first[0].clone(), input("b.rs", "+one\n+two\n")];
        let checked = block(review(jev, &cache, task, second, Thresholds::default())).unwrap();
        assert_eq!(
            server.received.lock().unwrap().len(),
            3,
            "only the file whose diff changed is asked about again"
        );
        assert_eq!(checked.files[0].relevance, Some(Relevance::Direct));
        assert_eq!(checked.files[1].relevance, Some(Relevance::Direct));
    }

    #[test]
    fn moving_a_threshold_re_reads_the_answers_instead_of_asking_again() {
        // Jev is half-sure the tests were weakened: a badge at 40%, none at 70%.
        let server = fake::answering(|id, _| match id {
            "tests" => fake::noul(0.55),
            _ => fake::noul(0.0),
        });
        let cache = Cache::default();
        let jev = Arc::new(Jev::at(&server.url, "k"));
        let files = vec![input("a.rs", "-assert!(x);\n")];
        let check = |thresholds| {
            block(review(
                Arc::clone(&jev),
                &cache,
                None,
                files.clone(),
                thresholds,
            ))
            .unwrap()
            .files[0]
                .flags
                .clone()
        };

        assert_eq!(check(Thresholds::default()), []);
        assert_eq!(
            check(Thresholds {
                flag_at_percent: 40,
                ..Thresholds::default()
            }),
            [ReviewFlag::WeakensTests]
        );
        assert_eq!(
            server.received.lock().unwrap().len(),
            1,
            "the model was asked once; the rest is policy in code"
        );
    }

    #[test]
    fn thresholds_are_percentages_and_refuse_silly_values() {
        let defaults = Thresholds::default();
        assert_eq!(defaults.flag_at(), 0.7);
        assert_eq!(defaults.off_task_at(), 0.6);
        assert_eq!(defaults.suggest_at(), 0.5);
        assert!(defaults.validate().is_ok());

        for bad in [0, 4, 96, 100, 200] {
            let thresholds = Thresholds {
                flag_at_percent: bad,
                ..defaults
            };
            assert!(thresholds.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_failure_leaves_the_rest_of_the_review_standing_but_a_bad_key_stops_it() {
        let rate_limited = fake::serve(|_| (429, serde_json::json!({})));
        let checked = block(review(
            Arc::new(Jev::at(&rate_limited.url, "k")),
            &Cache::default(),
            None,
            vec![input("a.rs", "+x\n")],
            Thresholds::default(),
        ))
        .unwrap();
        assert!(checked.problem.unwrap().contains("rate limiting"));
        assert!(checked.files[0].not_checked.is_some());

        let refused = fake::serve(|_| (401, serde_json::json!({})));
        assert_eq!(
            block(review(
                Arc::new(Jev::at(&refused.url, "k")),
                &Cache::default(),
                None,
                vec![input("a.rs", "+x\n")],
                Thresholds::default(),
            )),
            Err(JevError::BadKey)
        );
    }

    #[test]
    fn nothing_readable_means_nothing_asked() {
        let server = fake::answering(|_, _| fake::noul(0.0));
        let binary = FileInput::new(
            &FileChange {
                path: "logo.png".into(),
                old_path: None,
                kind: ChangeKind::Added,
                additions: None,
                deletions: None,
            },
            Scope::Committed,
            None,
        );
        let checked = block(review(
            Arc::new(Jev::at(&server.url, "k")),
            &Cache::default(),
            None,
            vec![binary],
            Thresholds::default(),
        ))
        .unwrap();
        assert!(checked.files[0].not_checked.is_some());
        assert!(server.received.lock().unwrap().is_empty());
    }

    #[test]
    fn long_diffs_are_cut_short_before_they_are_sent() {
        let long = format!("+{}\n", "x".repeat(MAX_PATCH_CHARS * 2));
        let input = input("big.rs", &long);
        let patch = input.patch.unwrap();
        assert!(patch.chars().count() < MAX_PATCH_CHARS + 100);
        assert!(patch.ends_with("because it is very long."));
    }

    #[test]
    fn credential_file_names_are_recognised_but_examples_are_not() {
        for yes in [
            ".env",
            "app/.env.production",
            "keys/server.pem",
            "deploy/id_ed25519",
            ".npmrc",
            "secrets/private.key",
        ] {
            assert!(looks_like_credentials(yes), "{yes}");
        }
        for no in [
            ".env.example",
            ".env.sample",
            "src/keyboard.rs",
            "identity.ts",
            "deploy/id_ed25519.pub",
            "README.md",
        ] {
            assert!(!looks_like_credentials(no), "{no}");
        }
    }
}

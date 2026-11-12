//! A workflow written by a model, from a description in the person's own words.
//!
//! This goes through Drafting (`crate::draft`): the agent the user already has, in its
//! non-interactive mode, or their Anthropic key. Never Assist — Jev answers typed questions and
//! does not write. The model is told the file format, the rules the validator enforces, and the
//! built-in review as an example; its reply is checked exactly as a file typed by hand is, and
//! when it has problems it is sent back once, with them, for one more try. Whatever comes back
//! opens in the editor unsaved: nothing a model wrote is a workflow until the person saves it.
//!
//! The same writer also changes a workflow that already exists — a built-in just customized, or
//! a file of the person's — from a description of the change. It is shown the whole file, told to
//! keep its `id` (a copy that replaces a built-in does so by that id), and its answer is checked
//! the same way, with a changed id counted as one more problem.
//!
//! The loop is [`next_prompt`] and [`finish`]: the app asks whichever writer it has and calls
//! `finish` on the answer, until `next_prompt` has nothing more to ask.

use std::path::{Path, PathBuf};

use super::{parse, Problem};
use crate::error::{IpcError, IpcResult};
use crate::git::Git;

/// How much of a description is sent. A workflow is described in a paragraph, not a document.
pub const MAX_DESCRIPTION_CHARS: usize = 4_000;

/// A first answer, and one more with the problems pointed out. Not a third: a model that got it
/// wrong twice is not going to get it right by being told a third time, and the person can fix
/// the rest in the editor, where every problem is marked.
pub const MAX_TRIES: u32 = 2;

/// The standing instruction: the format, the rules, and the built-in as an example. `concat!`
/// so it is one static string, as Drafting's system prompts are.
pub const SYSTEM: &str = concat!(
    "You write Yardsort workflow files. Reply with the YAML file and nothing else: no preamble, \
     no explanation, no code fences.\n\n\
     A workflow is a YAML file with these keys, and no others:\n\
     - `id`: lowercase letters, digits and `-`, like `fix-ci`. Short, from the name.\n\
     - `name`: what it is called.\n\
     - `description`: one or two sentences. Optional.\n\
     - `version`: always `1`.\n\
     - `trigger`: always `kind: manual`.\n\
     - `inputs`: optional, a list. Each has `id` (a lowercase name), `kind` (`harness` for \
     which agent, `test` for one of the workspace's smoke tests, `text`, or `choice` with \
     `options`), `label`, `required` (default false), `default`.\n\
     - `steps`: a list of at least one. Each has `id` (a lowercase name), `action`, optional \
     `needs` (the step ids that must succeed first; steps whose needs are met run together), \
     optional `produces` and `consumes` (named documents handed from one step to a later one), \
     and the action's own keys, and no others:\n\
       - `start_session`: `harness` (an agent id like `claude`, or exactly \
     `\"{{ inputs.<id> }}\"` for an input of kind harness), `prompt` (the agent's first \
     message), optional `model`, `effort`, `skip_memory`.\n\
       - `wait_session`: `session`, optional `until` (`settled`, the default, or `exited`), \
     optional `timeout`. Optional `clarify: true` lets the agent ask the person questions \
     with `ys workflow ask` before the session counts as finished. Optional `max_rounds` \
     (1 to 10, default 3) bounds how many rounds. A question only printed in the session is \
     not asked. `clarify` is not an approval. With `clarify: true` the step may \
     `produces` a `from: brief` artifact: the questions, the answers and the summary the \
     agent records with `ys workflow summary`.\n\
       - `send_to_session`: `session`, `prompt` (typed into the agent once it is quiet), \
     optional `timeout`.\n\
       - `wait_pr_activity`: waits for a review or comment on the workspace's pull request \
     after the run started. Optional `kind` (`review`, `comment`, `any`), optional `timeout`.\n\
       - `run_test`: runs one of the workspace's smoke tests and waits for its verdict. The \
     step succeeds whether the test passed or failed, leaving `verdict` for a later step to act \
     on either way; it fails only if the run reached no verdict or did not end in time. \
     Optional `gate: true` makes a failed verdict fail the step, skip the steps still going, \
     and stop the run. `test` \
     (a test id, or exactly `\"{{ inputs.<id> }}\"` for an \
     input of kind test), optional `inputs` (a map of the test's input ids to values), optional \
     `timeout`.\n\
       - `notify`: `title`, optional `body`. A system notification.\n\
       - `approve`: waits until a person in Yardsort, or `ys` from their own terminal, \
     approves, rejects, or requests changes. Optional `prompt`, optional `timeout`. It is \
     always a gate: a rejection or a request for changes stops the run. Do not set \
     `gate: false`. Waiting for a GitHub review is `wait_pr_activity`, not this. Leaves \
     `decision` (`approved`, `rejected` or `changes_requested`) and `feedback`.\n\
     `session` is `origin` (the agent already working in the workspace) or exactly \
     `\"{{ steps.<id>.session }}\"` for an earlier `start_session` step that is among this \
     step's needs. `timeout` is a number and a unit: `30s`, `45m`, `2h`; at most a week.\n\n\
     Text in steps may use these variables, written `{{ name }}`, and no others: \
     `project.name`, `project.root`, `workspace.name`, `workspace.branch`, \
     `workspace.base_branch`, `workspace.path`, `workspace.task` (its first message), \
     `pr.number`, `pr.url`, `pr.title` (the workspace's open pull request; `pr.repo` and \
     `pr.clone` are only for a pull request a workspace tracks elsewhere), `inputs.<id>`, \
     `steps.<id>.session` and `steps.<id>.run` from a start_session step, \
     `steps.<id>.outcome` from wait_session, `steps.<id>.count` and `steps.<id>.latest_url` \
     from wait_pr_activity, `steps.<id>.run`, `steps.<id>.verdict` and `steps.<id>.summary` \
     from run_test, `steps.<id>.decision` and `steps.<id>.feedback` from approve, \
     `memory` (the project's notes for agents), `handoff` (what \
     happened in the workspace so far), `artifacts.<name>` (a named document an earlier step \
     produced; this step must list that name under `consumes`). No filters, no expressions. A \
     prompt that uses the pull request or `session: origin` needs the workspace to have one \
     when the run starts.\n\n\
     `produces` is a list. Each item has `name` and where the text comes from when the step \
     succeeds: `path` for a file in the workspace, `text` for text written here, `field` for \
     one field this step leaves (such as `summary`), `from: transcript` with `session` and \
     optional `raw: true`, or `from: brief` on a `wait_session` with `clarify: true`. \
     `session` or `raw` alone does not copy a transcript: the step must \
     say `from: transcript`. The step that produces a name must be among the consumer's \
     `needs`, directly or through other steps. A `start_session` or `send_to_session` that \
     consumes an artifact and does not place `{{ artifacts.<name> }}` in its own prompt is \
     given the text under a heading. Those two steps cannot hand on a file or a transcript, \
     because they succeed before the agent has written or replied. Put that on the \
     `wait_session`. A `notify` step may hand on a file or a transcript that is already \
     there. Use `from: transcript` only when the person asked for the conversation.\n\n\
     Keep it small: the fewest steps that do what was asked, a `wait_session` after each \
     `start_session` whose finish matters, and a `notify` at the end so the person knows. \
     Write prompts to an agent as clear instructions, in the second person, saying what to do \
     and when to stop.\n\n\
     For example, the built-in code review:\n\n",
    include_str!("builtin/code-review.yaml"),
);

/// What is being asked for: a new workflow, or a change to the one in `current`.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
    /// In the person's own words: the whole workflow, or the change to make.
    pub description: &'a str,
    /// The workflow's text as it stands, when the description is a change to it.
    pub current: Option<&'a str>,
}

impl<'a> Ask<'a> {
    /// A new workflow, from nothing.
    pub fn new(description: &'a str) -> Self {
        Self {
            description,
            current: None,
        }
    }

    /// A change to `current`.
    pub fn change(description: &'a str, current: &'a str) -> Self {
        Self {
            description,
            current: Some(current),
        }
    }

    /// The id the answer must keep: the current workflow's, when it has one.
    fn keep_id(&self) -> Option<String> {
        self.current.and_then(id_of)
    }
}

/// The id a workflow's text gives itself, whether or not the rest of it checks out.
fn id_of(text: &str) -> Option<String> {
    match parse(text) {
        Ok(workflow) => Some(workflow.id),
        Err(invalid) => invalid.id,
    }
}

/// What the model is asked first.
pub fn first_prompt(ask: Ask<'_>) -> String {
    let description = crate::draft::clamp(ask.description.trim(), MAX_DESCRIPTION_CHARS);
    let Some(current) = ask.current else {
        return format!("Write a workflow that does this:\n\n{description}");
    };
    let keep = match ask.keep_id() {
        Some(id) => format!("Keep its `id`, `{id}`. "),
        None => String::new(),
    };
    format!(
        "Here is a workflow as it stands:\n\n{}\n\nChange it like this:\n\n{description}\n\n\
         {keep}Change only what this asks for and keep the rest as it is. Reply with the whole \
         changed file.",
        current.trim_end()
    )
}

/// What the model is asked again: what was asked the first time, since a second answer is a
/// fresh conversation with no memory of it, then what it wrote and what is wrong with that.
pub fn retry_prompt(ask: Ask<'_>, text: &str, problems: &[Problem]) -> String {
    let listed: Vec<String> = problems.iter().map(|p| format!("- {p}")).collect();
    format!(
        "{}\n\nYour first answer has these problems, each at its line and column:\n\n{}\n\n\
         Here is what you wrote:\n\n{}\n\nReply with the whole corrected file, still doing \
         what was asked.",
        first_prompt(ask),
        listed.join("\n"),
        text.trim_end()
    )
}

/// Where an agent stands while it writes: an empty git repository of the profile's own, made
/// once. Empty, because a description is not a change to describe and no project of the
/// user's should be read for it; a repository, because some agents will not run outside one —
/// Codex refuses a folder that is not a trusted git checkout.
pub fn scratch_dir(data_dir: &Path, git: &Git) -> IpcResult<PathBuf> {
    let dir = data_dir.join("drafting");
    std::fs::create_dir_all(&dir)
        .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", dir.display())))?;
    // Git spells a root as it really is — a symlink followed, a short name expanded — so the
    // two are compared normalised, or a folder that is already the repository would be taken
    // for one inside another and set up again every time.
    if !is_own_repository(&dir, git)? {
        git.init(&dir)?;
    }
    Ok(dir)
}

/// Whether `dir` is a repository's own root, rather than a folder inside some other one.
///
/// Git spells a root as it really is — a symlink followed, a short name expanded — so the two
/// are compared normalised, or a folder that is already the repository would be taken for one
/// inside another and set up again every time.
fn is_own_repository(dir: &Path, git: &Git) -> IpcResult<bool> {
    Ok(git
        .repo_root(dir)?
        .is_some_and(|root| crate::git::normalize(&root) == crate::git::normalize(dir)))
}

/// What a model wrote, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    /// The file, tidied: fences off, one trailing newline.
    pub text: String,
    /// What is wrong with it; empty when it checks out.
    pub problems: Vec<Problem>,
    /// How many answers this is the result of.
    pub tries: u32,
}

/// The next thing to ask, or nothing: the answer so far is good, or has been tried enough.
pub fn next_prompt(ask: Ask<'_>, so_far: Option<&Described>) -> Option<String> {
    match so_far {
        None => Some(first_prompt(ask)),
        Some(done) if done.problems.is_empty() || done.tries >= MAX_TRIES => None,
        Some(again) => Some(retry_prompt(ask, &again.text, &again.problems)),
    }
}

/// An answer to `ask`, tidied and checked, as the `tries`th. A change that gave the workflow
/// another id has that as a problem too: saved, it would stop replacing the built-in it is a copy
/// of, or leave the person's other workflows pointing at an id that is gone.
pub fn finish(ask: Ask<'_>, answer: &str, tries: u32) -> Described {
    let mut text = crate::draft::tidy(answer);
    text.push('\n');
    let mut problems = match parse(&text) {
        Ok(_) => Vec::new(),
        Err(invalid) => invalid.problems,
    };
    if let Some(keep) = ask.keep_id() {
        match id_of(&text) {
            Some(id) if id == keep => {}
            Some(id) => problems.push(Problem::whole(format!(
                "The `id` is `{id}`; keep it `{keep}`, as it was."
            ))),
            // A file with no id at all already has that as a problem.
            None => {}
        }
    }
    Described {
        text,
        problems,
        tries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::BUILT_IN;

    #[test]
    fn the_model_is_told_the_format_and_shown_the_built_in() {
        for word in [
            "start_session",
            "wait_session",
            "send_to_session",
            "wait_pr_activity",
            "notify",
            "gate: true",
            "artifacts.<name>",
            "consumes",
            "{{ steps.<id>.session }}",
            "workspace.base_branch",
            "no code fences",
        ] {
            assert!(SYSTEM.contains(word), "{word}");
        }
        assert!(
            SYSTEM.ends_with(BUILT_IN[0].1),
            "the example is the built-in, whole"
        );
    }

    #[test]
    fn the_description_is_asked_trimmed_and_a_long_one_is_cut() {
        assert_eq!(
            first_prompt(Ask::new("  Review my PR.  ")),
            "Write a workflow that does this:\n\nReview my PR."
        );
        let long = "words ".repeat(2_000);
        assert!(first_prompt(Ask::new(&long)).contains("left out because it is very long"));
    }

    #[test]
    fn a_retry_repeats_what_was_asked_then_names_each_problem_and_shows_the_answer() {
        let asked = Ask::new("Run the tests with Codex. Never post a review.");
        // A first answer that lost the whole task: a second writer starts from nothing.
        let bad = finish(asked, "id: draft\nname: Draft\n", 1);
        assert!(!bad.problems.is_empty());
        let again = retry_prompt(asked, &bad.text, &bad.problems);
        assert!(again.starts_with(&first_prompt(asked)), "{again}");
        assert!(again.contains("Never post a review."));
        for problem in &bad.problems {
            assert!(again.contains(&format!("- {problem}")), "{again}");
        }
        assert!(again.contains("id: draft\nname: Draft"));
        assert!(again.ends_with("still doing what was asked."));
        assert_eq!(
            next_prompt(asked, Some(&bad)).as_deref(),
            Some(again.as_str()),
            "the loop asks exactly this"
        );
    }

    #[test]
    fn a_change_shows_the_whole_workflow_asks_for_the_change_and_to_keep_its_id() {
        let current = BUILT_IN[0].1;
        let asked = first_prompt(Ask::change("  Also run the tests first.  ", current));
        assert!(
            asked.starts_with("Here is a workflow as it stands:"),
            "{asked}"
        );
        assert!(asked.contains(current.trim_end()), "the whole file");
        assert!(asked.contains("Change it like this:\n\nAlso run the tests first.\n"));
        assert!(asked.contains("Keep its `id`, `code-review`."), "{asked}");
        assert!(asked.contains("keep the rest as it is"));
    }

    #[test]
    fn a_change_that_renames_the_workflow_has_that_as_a_problem_and_is_sent_back() {
        let current = BUILT_IN[0].1;
        let ask = Ask::change("Call it careful review.", current);
        let renamed = current.replacen("id: code-review", "id: careful-review", 1);
        let done = finish(ask, &renamed, 1);
        assert_eq!(done.problems.len(), 1, "{:?}", done.problems);
        assert!(
            done.problems[0].message.contains("keep it `code-review`"),
            "{:?}",
            done.problems
        );
        let again = next_prompt(ask, Some(&done)).expect("sent back once");
        assert!(again.starts_with(&first_prompt(ask)));
        assert!(again.contains("keep it `code-review`"));

        let kept = finish(ask, current, 1);
        assert!(kept.problems.is_empty(), "{:?}", kept.problems);
        assert_eq!(next_prompt(ask, Some(&kept)), None);
        // A new workflow may be called anything.
        assert!(finish(Ask::new("Review it."), &renamed, 1)
            .problems
            .is_empty());
    }

    #[test]
    fn the_scratch_folder_is_an_empty_repository_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let git = crate::git::testing::git();
        let scratch = scratch_dir(dir.path(), &git).unwrap();
        assert_eq!(scratch, dir.path().join("drafting"));
        let root = git.repo_root(&scratch).unwrap().expect("a repository");
        assert_eq!(
            crate::git::normalize(&root),
            crate::git::normalize(&scratch)
        );
        let files: Vec<_> = std::fs::read_dir(&scratch)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            files,
            vec![std::ffi::OsString::from(".git")],
            "nothing else in it"
        );
        let again = scratch_dir(dir.path(), &git).unwrap();
        assert_eq!(again, scratch, "the same folder, not another init");
    }

    /// macOS keeps its temporary folders behind a symlink and Windows spells them short, so
    /// git names the repository differently from the path it was asked about.
    #[cfg(unix)]
    #[test]
    fn a_data_dir_reached_through_a_symlink_is_still_its_own_repository() {
        let real = tempfile::tempdir().unwrap();
        let link_holder = tempfile::tempdir().unwrap();
        let link = link_holder.path().join("data");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        let git = crate::git::testing::git();
        let scratch = scratch_dir(&link, &git).unwrap();
        assert_ne!(
            git.repo_root(&scratch).unwrap().unwrap(),
            scratch,
            "git spells the root differently from the path it was asked about"
        );
        assert!(
            is_own_repository(&scratch, &git).unwrap(),
            "and still it is recognised as its own repository, not set up again"
        );
    }

    #[test]
    fn a_fenced_answer_is_unfenced_and_checked_like_any_file() {
        let good = finish(
            Ask::new("Review it."),
            &format!("```yaml\n{}\n```", BUILT_IN[0].1),
            1,
        );
        assert!(good.problems.is_empty(), "{:?}", good.problems);
        assert_eq!(good.text, BUILT_IN[0].1);
        let bad = finish(Ask::new("Review it."), "```\nid: nope\n```", 1);
        assert!(!bad.problems.is_empty());
        assert_eq!(bad.text, "id: nope\n");
    }

    /// The loop as the app runs it, with a writer that gets it wrong once.
    #[test]
    fn a_wrong_answer_is_sent_back_once_with_its_problems_and_a_right_one_ends_it() {
        let mut asked: Vec<String> = Vec::new();
        let mut writer = |prompt: &str| -> String {
            asked.push(prompt.to_owned());
            if asked.len() == 1 {
                "id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: go\n    action: notfy\n    title: Hi\n".into()
            } else {
                "id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: go\n    action: notify\n    title: Hi\n".into()
            }
        };
        let mut so_far: Option<Described> = None;
        while let Some(prompt) = next_prompt(Ask::new("Say hi."), so_far.as_ref()) {
            let answer = writer(&prompt);
            let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
            so_far = Some(finish(Ask::new("Say hi."), &answer, tries));
        }
        let done = so_far.unwrap();
        assert_eq!(asked.len(), 2);
        assert!(asked[0].starts_with("Write a workflow that does this:"));
        assert!(
            asked[1].contains("`notfy` is not an action"),
            "{}",
            asked[1]
        );
        assert!(done.problems.is_empty());
        assert_eq!(done.tries, 2);
    }

    #[test]
    fn two_wrong_answers_end_it_with_the_problems_still_there() {
        let mut asked = 0;
        let mut so_far: Option<Described> = None;
        while let Some(_prompt) = next_prompt(Ask::new("Say hi."), so_far.as_ref()) {
            asked += 1;
            let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
            so_far = Some(finish(Ask::new("Say hi."), "id: still-wrong\n", tries));
        }
        let done = so_far.unwrap();
        assert_eq!(asked, MAX_TRIES);
        assert!(
            !done.problems.is_empty(),
            "left for the person to fix in the editor"
        );
    }
}

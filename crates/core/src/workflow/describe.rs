//! A workflow written by a model, from a description in the person's own words.
//!
//! This goes through Drafting (`crate::draft`): the agent the user already has, in its
//! non-interactive mode, or their Anthropic key. Never Assist — Jev answers typed questions and
//! does not write. The model is told the file format, the rules the validator enforces, and the
//! built-in review as an example; its reply is checked exactly as a file typed by hand is, and
//! when it has problems it is sent back once, with them, for one more try. Whatever comes back
//! opens in the editor unsaved: nothing a model wrote is a workflow until the person saves it.
//!
//! The loop is [`next_prompt`] and [`finish`]: the app asks whichever writer it has and calls
//! `finish` on the answer, until `next_prompt` has nothing more to ask.

use super::{parse, Problem};

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
     which agent, `text`, or `choice` with `options`), `label`, `required` (default false), \
     `default`.\n\
     - `steps`: a list of at least one. Each has `id` (a lowercase name), `action`, optional \
     `needs` (the step ids that must succeed first; steps whose needs are met run together), \
     and the action's own keys, and no others:\n\
       - `start_session`: `harness` (an agent id like `claude`, or exactly \
     `\"{{ inputs.<id> }}\"` for an input of kind harness), `prompt` (the agent's first \
     message), optional `model`, `effort`, `skip_memory`.\n\
       - `wait_session`: `session`, optional `until` (`settled`, the default, or `exited`), \
     optional `timeout`.\n\
       - `send_to_session`: `session`, `prompt` (typed into the agent once it is quiet), \
     optional `timeout`.\n\
       - `wait_pr_activity`: waits for a review or comment on the workspace's pull request \
     after the run started. Optional `kind` (`review`, `comment`, `any`), optional `timeout`.\n\
       - `notify`: `title`, optional `body`. A system notification.\n\
     `session` is `origin` (the agent already working in the workspace) or exactly \
     `\"{{ steps.<id>.session }}\"` for an earlier `start_session` step that is among this \
     step's needs. `timeout` is a number and a unit: `30s`, `45m`, `2h`; at most a week.\n\n\
     Text in steps may use these variables, written `{{ name }}`, and no others: \
     `project.name`, `project.root`, `workspace.name`, `workspace.branch`, \
     `workspace.base_branch`, `workspace.path`, `workspace.task` (its first message), \
     `pr.number`, `pr.url`, `pr.title` (the workspace's open pull request), `inputs.<id>`, \
     `steps.<id>.session` and `steps.<id>.run` from a start_session step, \
     `steps.<id>.outcome` from wait_session, `steps.<id>.count` and `steps.<id>.latest_url` \
     from wait_pr_activity, `memory` (the project's notes for agents), `handoff` (what \
     happened in the workspace so far). No filters, no expressions. A prompt that uses the \
     pull request or `session: origin` needs the workspace to have one when the run starts.\n\n\
     Keep it small: the fewest steps that do what was asked, a `wait_session` after each \
     `start_session` whose finish matters, and a `notify` at the end so the person knows. \
     Write prompts to an agent as clear instructions, in the second person, saying what to do \
     and when to stop.\n\n\
     For example, the built-in code review:\n\n",
    include_str!("builtin/code-review.yaml"),
);

/// What the model is asked first.
pub fn first_prompt(description: &str) -> String {
    let description = crate::draft::clamp(description.trim(), MAX_DESCRIPTION_CHARS);
    format!("Write a workflow that does this:\n\n{description}")
}

/// What the model is asked again, with what was wrong with its answer.
pub fn retry_prompt(text: &str, problems: &[Problem]) -> String {
    let listed: Vec<String> = problems.iter().map(|p| format!("- {p}")).collect();
    format!(
        "The workflow you wrote has these problems, each at its line and column:\n\n{}\n\n\
         Here is what you wrote:\n\n{}\n\nReply with the whole corrected file.",
        listed.join("\n"),
        text.trim_end()
    )
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
pub fn next_prompt(description: &str, so_far: Option<&Described>) -> Option<String> {
    match so_far {
        None => Some(first_prompt(description)),
        Some(done) if done.problems.is_empty() || done.tries >= MAX_TRIES => None,
        Some(again) => Some(retry_prompt(&again.text, &again.problems)),
    }
}

/// An answer, tidied and checked, as the `tries`th.
pub fn finish(answer: &str, tries: u32) -> Described {
    let mut text = crate::draft::tidy(answer);
    text.push('\n');
    let problems = match parse(&text) {
        Ok(_) => Vec::new(),
        Err(invalid) => invalid.problems,
    };
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
            first_prompt("  Review my PR.  "),
            "Write a workflow that does this:\n\nReview my PR."
        );
        let long = "words ".repeat(2_000);
        assert!(first_prompt(&long).contains("left out because it is very long"));
    }

    #[test]
    fn a_retry_names_each_problem_where_it_is_and_shows_the_answer() {
        let bad = finish("id: x\nname: X\n", 1);
        assert!(!bad.problems.is_empty());
        let again = retry_prompt(&bad.text, &bad.problems);
        for problem in &bad.problems {
            assert!(again.contains(&format!("- {problem}")), "{again}");
        }
        assert!(again.contains("id: x\nname: X"));
        assert!(again.ends_with("Reply with the whole corrected file."));
    }

    #[test]
    fn a_fenced_answer_is_unfenced_and_checked_like_any_file() {
        let good = finish(&format!("```yaml\n{}\n```", BUILT_IN[0].1), 1);
        assert!(good.problems.is_empty(), "{:?}", good.problems);
        assert_eq!(good.text, BUILT_IN[0].1);
        let bad = finish("```\nid: nope\n```", 1);
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
        while let Some(prompt) = next_prompt("Say hi.", so_far.as_ref()) {
            let answer = writer(&prompt);
            let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
            so_far = Some(finish(&answer, tries));
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
        while let Some(_prompt) = next_prompt("Say hi.", so_far.as_ref()) {
            asked += 1;
            let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
            so_far = Some(finish("id: still-wrong\n", tries));
        }
        let done = so_far.unwrap();
        assert_eq!(asked, MAX_TRIES);
        assert!(
            !done.problems.is_empty(),
            "left for the person to fix in the editor"
        );
    }
}

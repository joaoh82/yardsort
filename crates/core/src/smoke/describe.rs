//! A smoke test written by a model, from a description in the person's own words.
//!
//! The same shape as workflow drafting (`crate::workflow::describe`): the agent the user already
//! has, in its non-interactive mode, or their Anthropic key — never Assist, which answers typed
//! questions and does not write. The model is told the format with [`GUIDE`](crate::smoke::GUIDE)
//! (the very text `ys test guide` prints, so what an agent is told and what the validator accepts
//! cannot drift) and shown [`EXAMPLE`](crate::smoke::EXAMPLE) as a whole file. Its reply is
//! checked exactly as a file typed by hand is, and when it has problems it is sent back once,
//! with them, for one more try. Whatever comes back opens in the editor unsaved: nothing a model
//! wrote is a test until the person saves it, into the chosen worktree's `.yardsort/tests`.
//!
//! The loop is [`next_prompt`] and [`finish`], as the workflow one is; the app asks whichever
//! writer it has and calls `finish` on the answer until `next_prompt` has nothing more to ask.

use super::{parse, Problem};

/// How much of a description is sent. A test is described in a paragraph, not a document.
pub const MAX_DESCRIPTION_CHARS: usize = 4_000;

/// A first answer, and one more with the problems pointed out. Not a third: a model that got it
/// wrong twice is not going to get it right by being told a third time, and the person can fix
/// the rest in the editor, where every problem is marked.
pub const MAX_TRIES: u32 = 2;

/// The standing instruction: a short preface, then the guide itself, then the example file.
/// `concat!` of the same `include_str!`s the `GUIDE` and `EXAMPLE` consts use, so it is one
/// static string and cannot drift from what `ys test guide`/`ys test example` print.
pub const SYSTEM: &str = concat!(
    "You write Yardsort smoke test files. A smoke test is a short, structured list of steps \
     that checks a piece of software from the outside. Reply with the YAML file and nothing \
     else: no preamble, no explanation, no code fences.\n\n\
     Here is the format, every action, key and variable:\n\n",
    include_str!("guide.md"),
    "\n\nFor example, a test that starts a server and checks it answers:\n\n",
    include_str!("example.yaml"),
);

/// What the model is asked first.
pub fn first_prompt(description: &str) -> String {
    let description = crate::draft::clamp(description.trim(), MAX_DESCRIPTION_CHARS);
    format!("Write a smoke test that does this:\n\n{description}")
}

/// What the model is asked again: what was asked the first time, since a second answer is a
/// fresh conversation with no memory of it, then what it wrote and what is wrong with that.
pub fn retry_prompt(description: &str, text: &str, problems: &[Problem]) -> String {
    let listed: Vec<String> = problems.iter().map(|p| format!("- {p}")).collect();
    format!(
        "{}\n\nYour first answer has these problems, each at its line and column:\n\n{}\n\n\
         Here is what you wrote:\n\n{}\n\nReply with the whole corrected file, still doing \
         what was asked.",
        first_prompt(description),
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
        Some(again) => Some(retry_prompt(description, &again.text, &again.problems)),
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
    use crate::smoke::{EXAMPLE, GUIDE};

    #[test]
    fn the_model_is_told_the_format_and_shown_the_example() {
        assert!(SYSTEM.contains("no code fences"));
        assert!(SYSTEM.contains(GUIDE), "the guide is in it, whole");
        assert!(SYSTEM.ends_with(EXAMPLE), "the example is the end of it");
    }

    #[test]
    fn the_description_is_asked_trimmed_and_a_long_one_is_cut() {
        assert_eq!(
            first_prompt("  Check the login.  "),
            "Write a smoke test that does this:\n\nCheck the login."
        );
        let long = "words ".repeat(2_000);
        assert!(first_prompt(&long).contains("left out because it is very long"));
    }

    #[test]
    fn a_fenced_answer_is_unfenced_and_checked_like_any_file() {
        let good = finish(&format!("```yaml\n{EXAMPLE}\n```"), 1);
        assert!(good.problems.is_empty(), "{:?}", good.problems);
        assert_eq!(good.text, EXAMPLE);
        let bad = finish("```\nid: nope\n```", 1);
        assert!(!bad.problems.is_empty());
        assert_eq!(bad.text, "id: nope\n");
    }

    #[test]
    fn a_retry_repeats_what_was_asked_then_names_each_problem_and_shows_the_answer() {
        let asked = "Check the health endpoint answers.";
        let bad = finish("id: draft\nname: Draft\n", 1);
        assert!(!bad.problems.is_empty());
        let again = retry_prompt(asked, &bad.text, &bad.problems);
        assert!(again.starts_with(&first_prompt(asked)), "{again}");
        assert!(again.contains("Check the health endpoint answers."));
        for problem in &bad.problems {
            assert!(again.contains(&format!("- {problem}")), "{again}");
        }
        assert!(again.ends_with("still doing what was asked."));
        assert_eq!(
            next_prompt(asked, Some(&bad)).as_deref(),
            Some(again.as_str()),
            "the loop asks exactly this"
        );
    }

    #[test]
    fn a_wrong_answer_is_sent_back_once_and_a_right_one_ends_it() {
        let mut asked: Vec<String> = Vec::new();
        let mut writer = |prompt: &str| -> String {
            asked.push(prompt.to_owned());
            if asked.len() == 1 {
                "id: demo\nname: Demo\n".into()
            } else {
                EXAMPLE.to_owned()
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
        assert!(asked[0].starts_with("Write a smoke test that does this:"));
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.tries, 2);
    }

    #[test]
    fn two_wrong_answers_end_it_with_the_problems_still_there() {
        let mut asked = 0;
        let mut so_far: Option<Described> = None;
        while next_prompt("Say hi.", so_far.as_ref()).is_some() {
            asked += 1;
            let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
            so_far = Some(finish("id: still-wrong\n", tries));
        }
        let done = so_far.unwrap();
        assert_eq!(asked, MAX_TRIES);
        assert!(!done.problems.is_empty(), "left for the person to fix");
    }
}

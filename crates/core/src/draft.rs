//! Having a model write the words: a commit message, or a pull request's title and description.
//!
//! This is the one place Yardsort asks a model to *produce* text rather than judge it. Assist
//! cannot do it — Jev answers typed questions and never writes (see `assist::jev`) — so drafting
//! is its own thing, with its own switch and its own two ways of getting an answer:
//!
//! 1. **The agent the user already has.** Every built-in harness has a non-interactive mode
//!    (`claude --print`, `codex exec`, `opencode run`, `grok --single`), so the diff can go to
//!    agent already installed, already logged in and already paid for. No new credential.
//! 2. **The user's own Anthropic API key**, for when no configured harness can write — a custom
//!    harness with no write arguments, or none installed.
//!
//! The agent is tried first, for the reason above. Neither runs unless the button is pressed.
//!
//! This module holds only what every client needs: the prompts, the limits, and how an answer is
//! cleaned up. Running a program and calling an API live in the app, which has the runtime for it.

/// The model the API-key path asks for when the user has not chosen another.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

/// How much diff to send. A commit message comes from the shape of a change, not from every
/// line of it, and a 50,000-line refactor would cost a fortune to describe no better.
pub const MAX_DIFF_CHARS: usize = 60_000;

/// A commit subject is one line; this is the ceiling on the whole message.
pub const MAX_COMMIT_CHARS: usize = 2_000;

/// What is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// One commit message: a subject line, and a body when the change warrants one.
    CommitMessage,
    /// A pull request: a title on the first line, then the description.
    PullRequest,
}

impl Want {
    /// The standing instruction. Kept short and concrete: a long style lecture makes for worse
    /// writing, not better, and the repository's own history is the real style guide.
    pub fn system(self) -> &'static str {
        match self {
            Self::CommitMessage => {
                "You write git commit messages. Reply with the message and nothing else: no \
                 preamble, no code fences, no quotation marks.\n\n\
                 First line: imperative mood, under 72 characters, no trailing full stop. Say \
                 what the change does, not which files it touches.\n\n\
                 Then, only if the change is not self-explanatory: a blank line, then a short \
                 paragraph on why it was made. Skip it for small or obvious changes. Never pad."
            }
            Self::PullRequest => {
                "You write pull request descriptions. Reply with the title on the first line, \
                 then a blank line, then the description in Markdown. Nothing else: no preamble, \
                 no code fences around the whole reply, no \"Title:\" label.\n\n\
                 The title is one line, under 72 characters, imperative mood, no trailing full \
                 stop. The description says what changed and why a reviewer should care; use \
                 short paragraphs or a list. Do not invent testing you cannot see evidence of, \
                 and do not list every file."
            }
        }
    }
}

/// What the model is given, in the order that reads best: the task first when there is one, then
/// the change itself.
///
/// The task is the opening messages of this workspace's conversations — what the agent was
/// *asked* to do. Without it a diff can only be described; with it, it can be explained.
pub fn prompt(want: Want, task: Option<&str>, diff: &str) -> String {
    let mut prompt = String::new();
    if let Some(task) = task.map(str::trim).filter(|task| !task.is_empty()) {
        prompt.push_str("The work was asked for like this:\n\n");
        prompt.push_str(task);
        prompt.push_str("\n\n");
    }
    prompt.push_str(match want {
        Want::CommitMessage => "Write a commit message for this diff.\n\n",
        Want::PullRequest => "Write a pull request title and description for this diff.\n\n",
    });
    prompt.push_str(&clamp(diff, MAX_DIFF_CHARS));
    prompt
}

/// Cut a diff down, saying so where it was cut rather than trailing off mid-line.
pub fn clamp(diff: &str, limit: usize) -> String {
    if diff.chars().count() <= limit {
        return diff.to_owned();
    }
    let kept: String = diff.chars().take(limit).collect();
    let kept = kept
        .rsplit_once('\n')
        .map_or(kept.as_str(), |(head, _)| head);
    format!("{kept}\n… the rest of this diff was left out because it is very long.")
}

/// What a model wrote, made fit to put in a text box.
///
/// Models wrap a reply in a code fence often enough that stripping one is worth the few lines —
/// and a fenced commit message pasted verbatim would be a commit message containing backticks.
pub fn tidy(answer: &str) -> String {
    let answer = answer.trim();
    let unfenced = strip_fence(answer).unwrap_or(answer);
    unfenced.trim().to_owned()
}

fn strip_fence(answer: &str) -> Option<&str> {
    let rest = answer.strip_prefix("```")?;
    // ```text, ```markdown, or just ``` — the language tag is the rest of that line.
    let (_, body) = rest.split_once('\n')?;
    let (body, _) = body.rsplit_once("```")?;
    Some(body)
}

/// Split a pull request answer into its title and its description.
pub fn split_pull_request(answer: &str) -> (String, String) {
    let answer = tidy(answer);
    let (title, body) = answer.split_once('\n').unwrap_or((&answer, ""));
    // A model that ignored the instruction and wrote "# Title" still meant the title.
    let title = title.trim().trim_start_matches('#').trim();
    (title.to_owned(), body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_comes_before_the_diff_when_there_is_one() {
        let with = prompt(Want::CommitMessage, Some("Fix the login bug"), "-a\n+b\n");
        assert!(with.starts_with("The work was asked for like this:"));
        assert!(with.find("Fix the login bug") < with.find("-a\n+b"));

        let without = prompt(Want::CommitMessage, None, "-a\n+b\n");
        assert!(without.starts_with("Write a commit message"));
        let blank = prompt(Want::CommitMessage, Some("   "), "-a\n+b\n");
        assert_eq!(without, blank, "a blank task is no task");
    }

    #[test]
    fn a_very_long_diff_is_cut_at_a_line_and_says_so() {
        let diff = "0123456789\n".repeat(100);
        let cut = clamp(&diff, 100);
        assert!(cut.ends_with("left out because it is very long."));
        assert!(cut.len() < diff.len());
        // Cut at a newline, so the last kept line is whole.
        let body = cut.lines().next().unwrap();
        assert_eq!(body, "0123456789");
    }

    #[test]
    fn a_short_diff_is_left_exactly_as_it_is() {
        assert_eq!(clamp("-a\n+b\n", 100), "-a\n+b\n");
    }

    #[test]
    fn a_fenced_answer_comes_back_unfenced() {
        assert_eq!(
            tidy("```\nFix the login redirect\n```"),
            "Fix the login redirect"
        );
        assert_eq!(
            tidy("```markdown\nFix the login redirect\n\nBecause.\n```"),
            "Fix the login redirect\n\nBecause."
        );
        assert_eq!(
            tidy("  Fix the login redirect \n"),
            "Fix the login redirect"
        );
    }

    #[test]
    fn a_fence_inside_the_message_is_left_alone() {
        let answer = "Add the example\n\nIt reads:\n\n```sh\nys workspace new\n```";
        assert_eq!(
            tidy(answer),
            answer,
            "the reply does not start with a fence"
        );
    }

    #[test]
    fn a_pull_request_splits_into_a_title_and_the_rest() {
        let (title, body) = split_pull_request("Fix the login redirect\n\nThe cookie was wrong.\n");
        assert_eq!(title, "Fix the login redirect");
        assert_eq!(body, "The cookie was wrong.");

        let (title, body) = split_pull_request("Fix the login redirect");
        assert_eq!(title, "Fix the login redirect");
        assert_eq!(body, "");

        let (title, _) = split_pull_request("# Fix the login redirect\n\nBody.");
        assert_eq!(
            title, "Fix the login redirect",
            "a heading is still a title"
        );
    }
}

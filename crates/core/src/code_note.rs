//! A note about particular lines of a pull request's diff, written for an agent.
//!
//! The user selects lines in the Pull requests view, writes what they want, and sends it. What
//! the agent gets is the pull request, the file, which side of the diff and which lines, the
//! lines themselves, and the note — nothing it has to go and look up. Where it goes is the
//! app's business (`publish::pull_requests`); the words are decided here, with tests.

use serde::Deserialize;
use specta::Type;

use crate::forge::{DiffSide, PullRequest};

/// The lines the note is about.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Excerpt {
    pub path: String,
    pub side: DiffSide,
    /// The first and last line, 1-based, of the file on that side.
    pub from: u32,
    pub to: u32,
    /// The lines themselves, as the viewer had them.
    pub text: String,
}

/// The message. `task` is what the workspace was first asked to do, for an agent that has not
/// seen it — a new conversation, or a new workspace.
pub fn prompt(pr: &PullRequest, excerpt: &Excerpt, note: &str, task: Option<&str>) -> String {
    let base = pr
        .details
        .as_ref()
        .map(|details| details.base.as_str())
        .unwrap_or("its base");
    let where_ = match excerpt.side {
        DiffSide::Right => "as the pull request has it",
        DiffSide::Left => "as it was before the pull request",
    };
    let lines = if excerpt.from == excerpt.to {
        format!("line {}", excerpt.from)
    } else {
        format!("lines {}–{}", excerpt.from, excerpt.to)
    };
    let mut text = format!(
        "Pull request #{number} ({url}), \"{title}\": the branch `{branch}` into `{base}`.\n\n",
        number = pr.number,
        url = pr.url,
        title = pr.title,
        branch = pr.branch,
    );
    if let Some(task) = task.map(str::trim).filter(|task| !task.is_empty()) {
        text.push_str(&format!("The task it was opened for:\n\n{task}\n\n"));
    }
    text.push_str(&format!(
        "A note from me about `{path}`, {lines}, {where_}:\n\n```\n{excerpt}\n```\n\n{note}\n\n\
         Act on it in this worktree, run the project's checks, and commit. Do not rebase or \
         force-push: the pull request's history stays as it is. If the note needs a decision \
         the code cannot settle, stop and ask me.",
        path = excerpt.path,
        excerpt = excerpt.text.trim_end_matches('\n'),
        note = note.trim(),
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr() -> PullRequest {
        let row = serde_json::json!({
            "number": 9, "url": "https://github.com/o/r/pull/9", "title": "Fix login",
            "headRefName": "ys/fix", "state": "OPEN", "baseRefName": "main",
        });
        crate::forge::pull_request(&row).unwrap()
    }

    #[test]
    fn the_agent_is_told_the_pull_request_the_lines_and_the_note() {
        let excerpt = Excerpt {
            path: "src/login.rs".into(),
            side: DiffSide::Right,
            from: 10,
            to: 12,
            text: "let a = 1;\nlet b = 2;\nlet c = 3;\n".into(),
        };
        let text = prompt(&pr(), &excerpt, "  Fold these into one.  ", None);
        assert!(text.starts_with(
            "Pull request #9 (https://github.com/o/r/pull/9), \"Fix login\": the branch `ys/fix` \
             into `main`.\n\n"
        ));
        assert!(text.contains(
            "A note from me about `src/login.rs`, lines 10–12, as the pull request has it:\n\n\
             ```\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```\n\nFold these into one.\n\n"
        ));
        assert!(text.contains("Do not rebase or force-push"));
        assert!(!text.contains("task it was opened for"));
    }

    #[test]
    fn one_line_of_the_old_side_and_the_task_for_an_agent_that_has_not_seen_it() {
        let excerpt = Excerpt {
            path: "a.rs".into(),
            side: DiffSide::Left,
            from: 4,
            to: 4,
            text: "gone()".into(),
        };
        let text = prompt(
            &pr(),
            &excerpt,
            "Why was this removed?",
            Some("  Make login work  "),
        );
        assert!(text.contains("The task it was opened for:\n\nMake login work\n\n"));
        assert!(text.contains("`a.rs`, line 4, as it was before the pull request:"));
        assert!(text.contains("```\ngone()\n```"));
    }
}

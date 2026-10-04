//! Tasks: work that exists before any workspace does, read from where a project keeps it.
//!
//! A task has an identity of its own — a source and a key, `github` and `#91` — and is what the
//! Tasks view lists and `ys task` prints. To begin with there is one source, a GitHub
//! repository's issues ([`github`]), read through `gh` like everything else on the forge;
//! [`TaskSource`] is the seam a second one goes behind. Nothing here says anything only GitHub
//! could: a key is a string, a state is open or closed with a reason.
//!
//! The rules live here rather than in the app so that the window and the command line cannot
//! come to disagree: what [needs an answer](needs_answer), and what a [`Filter`] lets through.
//! See `docs/design/23-tasks.md`.

pub mod delegate;
pub mod github;

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::forge::ForgeResult;

/// How many open tasks are read per project, most recently updated first. Past this the view
/// says how many more there are and sends you to the source for them.
pub const OPEN_CAP: usize = 200;

/// How many closed tasks are read, most recently updated first: one page.
pub const CLOSED_CAP: usize = 50;

/// How long one page may take. GitHub gives up on a query after ten seconds itself; this is
/// for a network that went away instead.
pub const PAGE_LIMIT: Duration = Duration::from_secs(20);

/// Where a task is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum TaskSourceKind {
    GitHub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Open,
    Closed,
}

/// Why a closed task was closed, when its source says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ClosedAs {
    Completed,
    NotPlanned,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskLabel {
    pub name: String,
    /// Six hexadecimal digits, without the `#`.
    pub color: String,
}

/// A task, as much of it as a row in the list needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub source: TaskSourceKind,
    /// What a person types and reads: `#91`. Unique within a project's source.
    pub key: String,
    pub url: String,
    pub title: String,
    pub state: TaskState,
    pub closed_as: Option<ClosedAs>,
    /// Who opened it, by login. `None` for an account that no longer exists.
    pub author: Option<String>,
    pub labels: Vec<TaskLabel>,
    pub assignees: Vec<String>,
    /// How many comments it has, all told.
    pub comments: u32,
    /// As the source wrote them: `2026-10-02T17:43:03Z`.
    pub created_at: String,
    pub updated_at: String,
    /// The last word on it is not a maintainer's: see [`needs_answer`].
    pub needs_answer: bool,
    /// Open pull requests that will close it when they merge, by number.
    pub linked_pull_requests: Vec<u32>,
}

/// One comment in a task's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskComment {
    pub author: Option<String>,
    pub created_at: String,
    /// Empty for one its source hid: what was hidden there is not shown by another door.
    pub body: String,
    pub url: Option<String>,
    /// Why the source hid it, when it did: `spam`, `off-topic`, …
    pub hidden: Option<String>,
    /// Written by a program, not a person.
    pub bot: bool,
    /// Written by someone who maintains the project.
    pub maintainer: bool,
}

/// One task in full: the row, its description and its conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: Task,
    pub body: String,
    /// Oldest first. The latest ones only, when there are more than the source hands over in
    /// one answer: [`Task::comments`] is how many there are.
    pub comments: Vec<TaskComment>,
}

/// What a source managed to read of a project's tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    /// How many the source says there are in this state, which may be more than were read.
    pub total: Option<u32>,
    /// Who is asking, as the source knows them.
    pub viewer: Option<String>,
    /// The project keeps no tasks there: a repository with issues switched off.
    pub disabled: bool,
    /// At least one page arrived. When none did, whatever was known before is still the best
    /// there is.
    pub answered: bool,
    /// Why the reading stopped short, when it did.
    pub problem: Option<String>,
    pub logged_out: bool,
}

/// A task to be made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    pub title: String,
    /// The description, as Markdown. May be empty.
    pub body: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

/// A task that was just made: what its source now calls it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreatedTask {
    pub key: String,
    pub url: String,
}

/// Why a task is being closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CloseReason {
    /// It is done.
    Completed,
    /// It will not be done.
    NotPlanned,
}

/// What to change about a task. Everything is optional; one that changes nothing is refused
/// rather than sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskEdit {
    pub title: Option<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub add_assignees: Vec<String>,
    pub remove_assignees: Vec<String>,
}

impl TaskEdit {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Somewhere tasks are kept. `root` is the project's checkout, which is how a source knows
/// which project is meant.
pub trait TaskSource {
    /// The project's tasks in `state`, most recently updated first, up to `cap`.
    ///
    /// Never an `Err`: a page that fails ends the reading and says why, and the pages before
    /// it are kept.
    fn list(&self, root: &Path, state: TaskState, cap: usize) -> TaskList;

    /// The task `key` in full, asked of the source now.
    fn show(&self, root: &Path, key: &str) -> ForgeResult<TaskDetail>;

    // Everything below writes to the source, where other people see it and are told of it.
    // Nothing here confirms anything: whoever calls these has already been asked.

    fn create(&self, root: &Path, new: &NewTask) -> ForgeResult<CreatedTask>;

    /// Add `text` to the task's conversation.
    fn comment(&self, root: &Path, key: &str, text: &str) -> ForgeResult<()>;

    fn close(&self, root: &Path, key: &str, reason: CloseReason) -> ForgeResult<()>;

    fn reopen(&self, root: &Path, key: &str) -> ForgeResult<()>;

    fn edit(&self, root: &Path, key: &str, change: &TaskEdit) -> ForgeResult<()>;

    /// The labels a task of this project can be given.
    fn labels(&self, root: &Path) -> ForgeResult<Vec<TaskLabel>>;

    /// Who a task of this project can be assigned to, by login.
    fn assignees(&self, root: &Path) -> ForgeResult<Vec<String>>;
}

/// Someone who said something on a task: its opener, or the author of a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    /// A person with an account. Not a bot, and not an account that has since been deleted:
    /// neither is waiting for anything.
    pub person: bool,
    /// One of the project's own: for GitHub, an owner, a member or a collaborator.
    pub maintainer: bool,
}

/// Whether a task is waiting on the project: it is open, and the last word on it is not a
/// maintainer's.
///
/// `latest` is the end of its conversation, oldest first. The last *person* in it decides; a
/// bot's comment is nobody's word, so a stale-bot's nudge neither answers a reporter nor asks
/// anything. With no person among them the opener decides — which is also the answer for a task
/// nobody has commented on: one a maintainer opened is not waiting on a maintainer.
pub fn needs_answer(open: bool, opener: Voice, latest: &[Voice]) -> bool {
    if !open {
        return false;
    }
    let last = latest
        .iter()
        .rev()
        .find(|voice| voice.person)
        .unwrap_or(&opener);
    last.person && !last.maintainer
}

/// What narrows a list of tasks. Everything is optional; an empty one lets every task through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub needs_answer: bool,
    /// Every one of these labels, by name, ignoring case.
    pub labels: Vec<String>,
    /// A login, or [`ME`].
    pub assignee: Option<String>,
    /// A login, or [`ME`].
    pub author: Option<String>,
    /// Words in the title, or the key: `91` and `#91` both find `#91`.
    pub search: Option<String>,
}

/// "Whoever is asking", in a filter: the source says who that is.
pub const ME: &str = "@me";

impl Filter {
    pub fn matches(&self, task: &Task, viewer: Option<&str>) -> bool {
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        // `@me` with nobody known to be asking matches nothing, rather than everything.
        let is = |login: &str, wanted: &str| match wanted {
            ME => viewer.is_some_and(|viewer| same(login, viewer)),
            wanted => same(login, wanted),
        };
        if self.needs_answer && !task.needs_answer {
            return false;
        }
        if !self
            .labels
            .iter()
            .all(|wanted| task.labels.iter().any(|label| same(&label.name, wanted)))
        {
            return false;
        }
        if let Some(wanted) = &self.assignee {
            if !task.assignees.iter().any(|login| is(login, wanted)) {
                return false;
            }
        }
        if let Some(wanted) = &self.author {
            if !task
                .author
                .as_deref()
                .is_some_and(|login| is(login, wanted))
            {
                return false;
            }
        }
        match self.search.as_deref().map(str::trim) {
            None | Some("") => true,
            Some(query) => {
                let digits = query.strip_prefix('#').unwrap_or(query);
                let by_key = !digits.is_empty()
                    && digits.bytes().all(|b| b.is_ascii_digit())
                    && task.key.trim_start_matches('#').starts_with(digits);
                by_key || task.title.to_lowercase().contains(&query.to_lowercase())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTSIDER: Voice = Voice {
        person: true,
        maintainer: false,
    };
    const MAINTAINER: Voice = Voice {
        person: true,
        maintainer: true,
    };
    const BOT: Voice = Voice {
        person: false,
        maintainer: false,
    };

    #[test]
    fn the_last_person_to_speak_decides_whether_an_answer_is_owed() {
        // Nobody has commented: the opener's is the last word.
        assert!(needs_answer(true, OUTSIDER, &[]));
        assert!(!needs_answer(true, MAINTAINER, &[]));
        // Answered, and then asked again.
        assert!(!needs_answer(true, OUTSIDER, &[OUTSIDER, MAINTAINER]));
        assert!(needs_answer(true, OUTSIDER, &[MAINTAINER, OUTSIDER]));
        // A maintainer's own issue that someone else has since asked about.
        assert!(needs_answer(true, MAINTAINER, &[OUTSIDER]));
        // Closed is closed, whoever spoke last.
        assert!(!needs_answer(false, OUTSIDER, &[OUTSIDER]));
    }

    #[test]
    fn a_bot_is_nobodys_word() {
        // A triage bot after the reporter does not answer them…
        assert!(needs_answer(true, OUTSIDER, &[BOT]));
        assert!(needs_answer(true, OUTSIDER, &[OUTSIDER, BOT, BOT]));
        // …and one after a maintainer does not ask anything.
        assert!(!needs_answer(true, OUTSIDER, &[MAINTAINER, BOT]));
        assert!(!needs_answer(true, MAINTAINER, &[BOT, BOT]));
        // An issue a bot opened and only bots commented on is waiting for no one. A bot can be
        // a "maintainer" as far as the forge's association goes; it still is not a person.
        assert!(!needs_answer(true, BOT, &[BOT]));
        let member_bot = Voice {
            person: false,
            maintainer: true,
        };
        assert!(needs_answer(true, OUTSIDER, &[member_bot]));
    }

    fn task(key: &str, title: &str) -> Task {
        Task {
            source: TaskSourceKind::GitHub,
            key: key.to_owned(),
            url: String::new(),
            title: title.to_owned(),
            state: TaskState::Open,
            closed_as: None,
            author: Some("grace".to_owned()),
            labels: vec![TaskLabel {
                name: "Bug".to_owned(),
                color: "d73a4a".to_owned(),
            }],
            assignees: vec!["ada".to_owned()],
            comments: 0,
            created_at: String::new(),
            updated_at: String::new(),
            needs_answer: false,
            linked_pull_requests: vec![],
        }
    }

    #[test]
    fn a_filter_narrows_by_each_thing_it_names_and_by_all_of_them_together() {
        let it = task("#91", "Worktrees on a network drive");
        let pass = |filter: Filter, viewer: Option<&str>| filter.matches(&it, viewer);
        assert!(pass(Filter::default(), None));

        let needs = Filter {
            needs_answer: true,
            ..Default::default()
        };
        assert!(!pass(needs.clone(), None));
        assert!(needs.matches(
            &Task {
                needs_answer: true,
                ..it.clone()
            },
            None
        ));

        let labelled = |names: &[&str]| Filter {
            labels: names.iter().map(|name| (*name).to_owned()).collect(),
            ..Default::default()
        };
        assert!(pass(labelled(&["bug"]), None), "case is ignored");
        assert!(!pass(labelled(&["bug", "question"]), None), "every label");

        let assigned = |login: &str| Filter {
            assignee: Some(login.to_owned()),
            ..Default::default()
        };
        assert!(pass(assigned("Ada"), None));
        assert!(!pass(assigned("grace"), None));
        assert!(pass(assigned(ME), Some("ada")));
        assert!(!pass(assigned(ME), Some("grace")));
        assert!(!pass(assigned(ME), None), "nobody is known to be asking");

        let by = |login: &str| Filter {
            author: Some(login.to_owned()),
            ..Default::default()
        };
        assert!(pass(by("grace"), None));
        assert!(pass(by(ME), Some("Grace")));
        assert!(!pass(by("ada"), None));

        let search = |words: &str| Filter {
            search: Some(words.to_owned()),
            ..Default::default()
        };
        assert!(pass(search("network DRIVE"), None));
        assert!(pass(search("#91"), None));
        assert!(pass(search("9"), None), "the start of a number");
        assert!(!pass(search("1"), None), "not the middle of one");
        assert!(!pass(search("windows"), None));
        assert!(pass(search("  "), None), "blank is no search");

        let both = Filter {
            labels: vec!["bug".to_owned()],
            author: Some("ada".to_owned()),
            ..Default::default()
        };
        assert!(!pass(both, None), "one that fails is enough");
    }
}

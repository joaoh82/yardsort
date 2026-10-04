//! `ys task …` — a project's tasks, from where the project keeps them.
//!
//! To begin with that is a GitHub repository's issues, read through `gh` exactly as the app's
//! Tasks view reads them: the reading, what needs an answer and what a filter lets through are
//! all `yardsort_core::tasks`, so the two cannot disagree. Nothing is kept between runs — each
//! command asks `gh` when it is run — and the app need not be open.
//!
//! `create`, `comment`, `close`, `reopen` and `edit` write to GitHub at once: there is no list
//! of proposals for the user to approve, as there is for memory. Whoever runs them — most often
//! an agent, behind its own permission prompt — has decided. `close` and `reopen` want `--yes`
//! all the same, so that neither can be the side effect of a command run to look.

use yardsort_core::forge::{self, ForgeKind, Gh};
use yardsort_core::store::ProjectRow;
use yardsort_core::tasks::delegate;
use yardsort_core::tasks::github::{coverage, Coverage, GitHub};
use yardsort_core::tasks::{
    CloseReason, ClosedAs, Filter, NewTask, Task, TaskDetail, TaskEdit, TaskList, TaskSource,
    TaskState, CLOSED_CAP, OPEN_CAP,
};

use crate::{table, Failure, Output, Yardsort};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum State {
    Open,
    Closed,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Reason {
    /// It is done.
    Completed,
    /// It will not be done.
    NotPlanned,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// The project's tasks, most recently updated first. Open ones unless --state says otherwise.
    List {
        /// The project, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        #[arg(long, value_enum, default_value = "open")]
        state: State,
        /// Only those waiting on a maintainer: open, and the last person to speak on them was
        /// not an owner, a member or a collaborator.
        #[arg(long)]
        needs_answer: bool,
        /// Only those with this label. Repeat it to ask for several at once.
        #[arg(long, value_name = "NAME")]
        label: Vec<String>,
        /// Only those assigned to this login. `@me` is whoever `gh` is logged in as.
        #[arg(long, value_name = "LOGIN")]
        assignee: Option<String>,
        /// Only those opened by this login. `@me` is whoever `gh` is logged in as.
        #[arg(long, value_name = "LOGIN")]
        author: Option<String>,
        /// Only those with these words in the title, or this number.
        #[arg(long, value_name = "WORDS")]
        search: Option<String>,
        /// At most this many.
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// One task in full: its description and its conversation.
    Show {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Open a new task: an issue on the project's repository. It is public at once, and the
    /// people watching the repository are told.
    Create {
        /// What it is about, in a line.
        title: String,
        /// The description, as Markdown.
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the description from this file. `-` reads standard input.
        #[arg(long, value_name = "FILE")]
        body_file: Option<String>,
        /// A label to give it. Repeat it for several.
        #[arg(long, value_name = "NAME")]
        label: Vec<String>,
        /// Who to assign it to, by login. `@me` is whoever `gh` is logged in as.
        #[arg(long, value_name = "LOGIN")]
        assignee: Vec<String>,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Post a comment on a task. Everyone following it is told.
    Comment {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        /// What to say, as Markdown.
        #[arg(required_unless_present = "body_file", conflicts_with = "body_file")]
        text: Option<String>,
        /// Read the comment from this file. `-` reads standard input.
        #[arg(long, value_name = "FILE")]
        body_file: Option<String>,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Close a task. Without `--yes` it says what it would close and does nothing.
    Close {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        #[arg(long, value_enum, default_value = "completed")]
        reason: Reason,
        /// Do it. Closing tells whoever opened the task that it is finished with.
        #[arg(long)]
        yes: bool,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Reopen a closed task. Without `--yes` it says what it would reopen and does nothing.
    Reopen {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        /// Do it. Reopening tells everyone following the task.
        #[arg(long)]
        yes: bool,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Change a task's labels, assignees or title.
    Edit {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_name = "NAME")]
        add_label: Vec<String>,
        #[arg(long, value_name = "NAME")]
        remove_label: Vec<String>,
        /// Assign it to this login. `@me` is whoever `gh` is logged in as.
        #[arg(long, value_name = "LOGIN")]
        assign: Vec<String>,
        #[arg(long, value_name = "LOGIN")]
        unassign: Vec<String>,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Hand a task to an agent: a new branch and worktree, and an agent started in it with
    /// the task as its first message.
    ///
    /// What `ys workspace new` does, with the message written from the task — its title,
    /// description and latest comments, marked as text other people wrote — and the workspace
    /// named after it and recorded as started from it. The message is not shown first: read
    /// the task with `ys task show` before starting an agent on something a stranger wrote.
    Start {
        /// Which one: `91`, `#91`, or its URL.
        task: String,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        /// The branch to start from. Defaults to the project's default branch.
        #[arg(long)]
        base: Option<String>,
        /// The agent to run. With one enabled agent, it can be left out.
        #[arg(long)]
        harness: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        effort: Option<String>,
        /// Make the workspace and record the task, but start nothing in it.
        #[arg(long)]
        no_agent: bool,
        /// Write the message an agent would be given, and stop: nothing is created or started.
        ///
        /// For reading what a stranger's issue turns into before an agent is set to it. The
        /// mark on its two lines is made anew for each message, so a later start carries
        /// another; the rest is the same unless the issue changed in between.
        #[arg(long, conflicts_with = "no_agent")]
        print: bool,
    },
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List {
            project,
            state,
            needs_answer,
            label,
            assignee,
            author,
            search,
            limit,
        } => {
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let root = std::path::Path::new(&project.root_path);
            let filter = Filter {
                needs_answer,
                labels: label,
                assignee,
                author,
                search,
            };

            let mut tasks = Vec::new();
            let mut viewer = None;
            let states: &[(TaskState, usize)] = match state {
                State::Open => &[(TaskState::Open, OPEN_CAP)],
                State::Closed => &[(TaskState::Closed, CLOSED_CAP)],
                State::All => &[(TaskState::Open, OPEN_CAP), (TaskState::Closed, CLOSED_CAP)],
            };
            for (state, cap) in states {
                let found = read(source.list(root, *state, *cap), &project)?;
                said_short(&found, *state);
                viewer = found.viewer.or(viewer);
                // One closed or reopened between the two questions is in both answers. It is
                // listed once, as the open one: what the app's list does with the same pair.
                for task in found.tasks {
                    if !tasks.iter().any(|seen: &Task| seen.key == task.key) {
                        tasks.push(task);
                    }
                }
            }
            let mut tasks: Vec<Task> = tasks
                .into_iter()
                .filter(|task| filter.matches(task, viewer.as_deref()))
                .collect();
            if state == State::All {
                tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            }
            tasks.truncate(limit.unwrap_or(usize::MAX));

            out.emit(&tasks, || {
                if tasks.is_empty() {
                    println!("{}", nothing(state, &filter));
                    return;
                }
                let mut rows = vec![vec![
                    "KEY".to_owned(),
                    "STATE".to_owned(),
                    "UPDATED".to_owned(),
                    "AUTHOR".to_owned(),
                    "LABELS".to_owned(),
                    "TITLE".to_owned(),
                ]];
                rows.extend(tasks.iter().map(|task| {
                    vec![
                        task.key.clone(),
                        standing(task),
                        day(&task.updated_at).to_owned(),
                        task.author.clone().unwrap_or_else(|| "ghost".to_owned()),
                        dash(labels(task)),
                        task.title.clone(),
                    ]
                }));
                table(&rows);
            })
        }
        Command::Show { task, project } => {
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let detail = source
                .show(std::path::Path::new(&project.root_path), &task)
                .map_err(|error| match error.is_logged_out() {
                    true => Failure::new(LOGGED_OUT),
                    false => Failure::new(error.to_string()),
                })?;
            out.emit(&detail, || print!("{}", written(&detail)))
        }
        Command::Create {
            title,
            body,
            body_file,
            label,
            assignee,
            project,
        } => {
            let body = text_from(body, body_file.as_deref())?.unwrap_or_default();
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let created = source
                .create(
                    std::path::Path::new(&project.root_path),
                    &NewTask {
                        title,
                        body,
                        labels: label,
                        assignees: assignee,
                    },
                )
                .map_err(said)?;
            out.emit(&created, || {
                println!("created  {}\n  url    {}", created.key, created.url)
            })
        }
        Command::Comment {
            task,
            text,
            body_file,
            project,
        } => {
            let text = text_from(text, body_file.as_deref())?.unwrap_or_default();
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            source
                .comment(std::path::Path::new(&project.root_path), &task, &text)
                .map_err(said)?;
            done(out, &task, "commented", "Commented on")
        }
        Command::Close {
            task,
            reason,
            yes,
            project,
        } => {
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let root = std::path::Path::new(&project.root_path);
            // Read first: it is what the refusal names, and a link is checked by it.
            let detail = source.show(root, &task).map_err(said)?;
            let it = &detail.task;
            if it.state == TaskState::Closed {
                return Err(Failure::new(format!("{} is already closed.", it.key)));
            }
            let (reason, as_what) = match reason {
                Reason::Completed => (CloseReason::Completed, "completed"),
                Reason::NotPlanned => (CloseReason::NotPlanned, "not planned"),
            };
            if !yes {
                return Err(Failure::new(format!(
                    "This would close {} as {as_what}: {}\nOpened by {}. Closing tells them it \
                     is finished with. Nothing was done; add --yes to do it.",
                    it.key,
                    it.title,
                    it.author.as_deref().unwrap_or("ghost"),
                )));
            }
            source.close(root, &it.key, reason).map_err(said)?;
            done(out, &it.key, "closed", "Closed")
        }
        Command::Reopen { task, yes, project } => {
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let root = std::path::Path::new(&project.root_path);
            let detail = source.show(root, &task).map_err(said)?;
            let it = &detail.task;
            if it.state == TaskState::Open {
                return Err(Failure::new(format!("{} is already open.", it.key)));
            }
            if !yes {
                return Err(Failure::new(format!(
                    "This would reopen {}: {}\nReopening tells everyone following it. Nothing \
                     was done; add --yes to do it.",
                    it.key, it.title,
                )));
            }
            source.reopen(root, &it.key).map_err(said)?;
            done(out, &it.key, "reopened", "Reopened")
        }
        Command::Edit {
            task,
            title,
            add_label,
            remove_label,
            assign,
            unassign,
            project,
        } => {
            let change = TaskEdit {
                title,
                add_labels: add_label,
                remove_labels: remove_label,
                add_assignees: assign,
                remove_assignees: unassign,
            };
            if change.is_empty() {
                return Err(Failure::new(
                    "Nothing to change. Give --title, --add-label, --remove-label, --assign or \
                     --unassign.",
                ));
            }
            let project = project_of(ys, project.as_deref())?;
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            source
                .edit(std::path::Path::new(&project.root_path), &task, &change)
                .map_err(said)?;
            done(out, &task, "edited", "Changed")
        }
        Command::Start {
            task,
            project,
            base,
            harness,
            model,
            effort,
            no_agent,
            print,
        } => {
            let project = project_of(ys, project.as_deref())?;
            // The harness is checked before `gh` is asked, and both before anything is made.
            if !no_agent && !print {
                super::workspace::choose_harness(ys, harness.as_deref())?;
            }
            let gh = gh(ys)?;
            let source = source(ys, &gh, &project)?;
            let detail = source
                .show(std::path::Path::new(&project.root_path), &task)
                .map_err(|error| match error.is_logged_out() {
                    true => Failure::new(LOGGED_OUT),
                    false => Failure::new(error.to_string()),
                })?;
            if detail.task.state == TaskState::Closed {
                return Err(Failure::new(format!(
                    "{} is closed. Reopen it on GitHub first, or start a workspace with \
                     `ys workspace new`.",
                    detail.task.key
                )));
            }
            let delegated = delegate::delegated(&detail);
            if print {
                return out.emit(&delegated, || print!("{}", delegated.prompt));
            }
            super::workspace::new(
                ys,
                project.id,
                delegated.prompt,
                Some(delegated.task),
                base,
                harness,
                model,
                effort,
                no_agent,
                out,
            )
        }
    }
}

/// What `gh` said, or that nobody is logged in to it.
fn said(error: forge::ForgeError) -> Failure {
    match error.is_logged_out() {
        true => Failure::new(LOGGED_OUT),
        false => Failure::new(error.to_string()),
    }
}

/// Text given outright, or read from a file, or from standard input for `-`.
fn text_from(given: Option<String>, file: Option<&str>) -> Result<Option<String>, Failure> {
    match (given, file) {
        (Some(text), _) => Ok(Some(text)),
        (None, Some("-")) => {
            use std::io::Read;
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| Failure::new(format!("cannot read standard input: {e}")))?;
            Ok(Some(text))
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|e| Failure::new(format!("cannot read {path}: {e}"))),
        (None, None) => Ok(None),
    }
}

/// Say that something was done to a task.
fn done(out: &Output, task: &str, did: &str, word: &str) -> Result<(), Failure> {
    #[derive(serde::Serialize)]
    struct Done<'a> {
        task: String,
        did: &'a str,
    }
    let key = match yardsort_core::tasks::github::number(task) {
        Some(number) => format!("#{number}"),
        None => task.to_owned(),
    };
    let done = Done { task: key, did };
    out.emit(&done, || println!("{word} {}.", done.task))
}

const LOGGED_OUT: &str = "Nobody is logged in to the GitHub CLI. Run `gh auth login`, then try \
                          again.";

/// The project a command is about: `--project` when given; otherwise the one the agent
/// running it was launched in, or the folder it is run in.
fn project_of(ys: &Yardsort, wanted: Option<&str>) -> Result<ProjectRow, Failure> {
    let (id, _) = super::memory::locate(ys, wanted)?;
    ys.store
        .project(&id)?
        .ok_or_else(|| Failure::new("That project is no longer open in Yardsort."))
}

fn gh(ys: &Yardsort) -> Result<Gh, Failure> {
    Gh::find(ys.env()).ok_or_else(|| {
        Failure::new(
            "Tasks are read with the GitHub CLI, and `gh` is not installed or not on PATH. \
             Get it from https://cli.github.com, then run `gh auth login`.",
        )
    })
}

/// Where this project's tasks are, or why they cannot be read.
fn source<'a>(ys: &Yardsort, gh: &'a Gh, project: &ProjectRow) -> Result<GitHub<'a>, Failure> {
    let root = std::path::Path::new(&project.root_path);
    let name = &project.name;
    match coverage(forge::repo_at(&ys.git()?, root).as_ref()) {
        Coverage::Covered(host) => Ok(GitHub::new(gh, host)),
        Coverage::NoRemote => Err(Failure::new(format!(
            "{name} has no remote on a forge, so there is nowhere to ask about tasks."
        ))),
        Coverage::OtherForge(kind) => Err(Failure::new(format!(
            "{name} is on {}. Tasks are read for GitHub only.",
            match kind {
                ForgeKind::GitLab => "GitLab",
                ForgeKind::Bitbucket => "Bitbucket",
                ForgeKind::GitHub | ForgeKind::Unknown => "another forge",
            }
        ))),
    }
}

/// A list that could not be read at all is a failure; one that could is the answer.
fn read(found: TaskList, project: &ProjectRow) -> Result<TaskList, Failure> {
    if !found.answered {
        return Err(Failure::new(if found.logged_out {
            LOGGED_OUT.to_owned()
        } else {
            found
                .problem
                .unwrap_or_else(|| "gh did not answer.".to_owned())
        }));
    }
    if found.disabled {
        return Err(Failure::new(format!(
            "{}'s repository has issues switched off, so it has no tasks.",
            project.name
        )));
    }
    Ok(found)
}

/// Say, beside the answer and not in it, when the answer is not the whole of it: a script
/// reading the JSON must still be able to tell.
fn said_short(found: &TaskList, state: TaskState) {
    let word = match state {
        TaskState::Open => "open",
        TaskState::Closed => "closed",
    };
    if let Some(problem) = &found.problem {
        eprintln!(
            "Not every {word} task could be read: {problem}\nWhat arrived before that is listed."
        );
    } else if let Some(total) = found
        .total
        .filter(|total| *total as usize > found.tasks.len())
    {
        eprintln!(
            "Read the {} most recently updated of {total} {word} tasks. The rest are on GitHub.",
            found.tasks.len()
        );
    }
}

fn nothing(state: State, filter: &Filter) -> String {
    let word = match state {
        State::Open => "open ",
        State::Closed => "closed ",
        State::All => "",
    };
    if *filter == Filter::default() {
        format!("No {word}tasks.")
    } else {
        format!("No {word}tasks match.")
    }
}

/// `open`, `open, needs answer`, `closed (not planned)`.
fn standing(task: &Task) -> String {
    match (task.state, task.closed_as) {
        (TaskState::Open, _) if task.needs_answer => "open, needs answer".to_owned(),
        (TaskState::Open, _) => "open".to_owned(),
        (TaskState::Closed, Some(ClosedAs::NotPlanned)) => "closed (not planned)".to_owned(),
        (TaskState::Closed, Some(ClosedAs::Duplicate)) => "closed (duplicate)".to_owned(),
        (TaskState::Closed, _) => "closed".to_owned(),
    }
}

/// The date of `2026-10-02T17:43:03Z`.
fn day(at: &str) -> &str {
    at.split('T').next().unwrap_or(at)
}

fn labels(task: &Task) -> String {
    let names: Vec<&str> = task
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect();
    names.join(", ")
}

fn dash(text: String) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text
    }
}

/// A task in full, for a person or an agent to read.
fn written(detail: &TaskDetail) -> String {
    let task = &detail.task;
    let who = |author: &Option<String>| author.clone().unwrap_or_else(|| "ghost".to_owned());
    let mut text = format!("{} {}\n{}\n", task.key, task.title, task.url);
    text.push_str(&format!(
        "{} · opened by {} on {}",
        standing(task),
        who(&task.author),
        day(&task.created_at)
    ));
    if !task.labels.is_empty() {
        text.push_str(&format!(" · labels: {}", labels(task)));
    }
    if !task.assignees.is_empty() {
        text.push_str(&format!(" · assigned to: {}", task.assignees.join(", ")));
    }
    if !task.linked_pull_requests.is_empty() {
        let numbers: Vec<String> = task
            .linked_pull_requests
            .iter()
            .map(|number| format!("#{number}"))
            .collect();
        text.push_str(&format!(" · pull request: {}", numbers.join(", ")));
    }
    text.push_str("\n\n");
    text.push_str(match detail.body.trim() {
        "" => "(no description)",
        body => body,
    });
    text.push_str("\n\n");

    let shown = detail.comments.len();
    let all = task.comments as usize;
    text.push_str(&match (shown, all) {
        (0, _) => "No comments.\n".to_owned(),
        (shown, all) if all > shown => format!("Comments, the latest {shown} of {all}:\n"),
        (1, _) => "1 comment:\n".to_owned(),
        (shown, _) => format!("{shown} comments:\n"),
    });
    for comment in &detail.comments {
        let role = match (comment.bot, comment.maintainer) {
            (true, _) => " (bot)",
            (false, true) => " (maintainer)",
            (false, false) => "",
        };
        text.push_str(&format!(
            "\n{}{role}, {}:\n",
            who(&comment.author),
            day(&comment.created_at)
        ));
        text.push_str(&match &comment.hidden {
            Some(reason) if reason.is_empty() => "(hidden on GitHub)".to_owned(),
            Some(reason) => format!("(hidden on GitHub as {reason})"),
            None => comment.body.trim().to_owned(),
        });
        text.push('\n');
    }
    text
}

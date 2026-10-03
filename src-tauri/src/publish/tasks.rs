//! Every project's tasks, for the Tasks view: what its source says is waiting to be done.
//!
//! The reading and the rules are the core's (`crate::tasks`), shared with `ys task`. What is
//! here is what only the app needs: an answer kept for [`FRESH_FOR`] so that the view, the
//! sidebar's count and a window coming back into focus do not each ask the forge, and the
//! shape the webview is handed.
//!
//! Open tasks are asked for whenever the list is; closed ones only when the view is showing
//! them, which is a second question nobody needs answered otherwise.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::commands::{failed, project_root, repo_at};
use crate::error::IpcResult;
use crate::forge::{ForgeError, ForgeResult, Gh, Repo};
use crate::state::{blocking, AppState};
use crate::tasks::delegate::{delegated, Delegated, TaskRef};
use crate::tasks::github::{coverage, Coverage, GitHub};
use crate::tasks::{
    CloseReason, CreatedTask, NewTask, Task, TaskDetail, TaskEdit, TaskLabel, TaskList, TaskSource,
    TaskState, CLOSED_CAP, OPEN_CAP,
};

/// How long an answer is reused before the source is asked again: the pull requests' interval,
/// for the same reasons.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// What a project's source says about its tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTasks {
    /// `gh` is installed. Without it there are no tasks anywhere in the app.
    pub gh: bool,
    /// The open ones, then the closed ones when they were read; each most recently updated
    /// first.
    pub tasks: Vec<Task>,
    /// Why the list is short or empty when it should not have been.
    pub problem: Option<String>,
    /// The problem is that nobody is logged in, which has its own one-line fix.
    pub logged_out: bool,
    /// The project's remote read as a repository on a forge: how the view knows a project is
    /// not on GitHub. `None` for a remote that is a path on disk, and for no remote at all.
    pub repo: Option<Repo>,
    /// Who `gh` is logged in as: what "me" means in the view's filters.
    pub viewer: Option<String>,
    /// How many open tasks the source says there are. More than the list holds when there are
    /// more than it reads. `None` until it has answered.
    pub open_total: Option<u32>,
    /// The repository has issues switched off.
    pub disabled: bool,
    /// The closed tasks are part of the list.
    pub closed: bool,
}

/// What was last read of one project's tasks in one state.
struct Tier {
    at: Instant,
    found: TaskList,
    /// Something has happened that the answer does not know about: see [`TaskCache::forget`].
    stale: bool,
}

/// (project, closed)
type TierKey = (String, bool);

/// The tasks cache. One per app.
#[derive(Default)]
pub struct TaskCache {
    tiers: Mutex<HashMap<TierKey, Tier>>,
    /// The latest request per tier: an answer that is no longer the latest is dropped.
    requests: Mutex<HashMap<TierKey, u64>>,
    next_request: AtomicU64,
    /// Tasks read in full, by project and key.
    details: Mutex<HashMap<(String, String), (Instant, TaskDetail)>>,
    /// The references handed out for delegating, by project: see [`TaskCache::issued`].
    issued: Mutex<HashMap<String, Vec<TaskRef>>>,
}

impl TaskCache {
    /// Read one tier with `fetch` unless it was read less than [`FRESH_FOR`] ago.
    fn load(
        &self,
        project_id: &str,
        closed: bool,
        refresh: bool,
        fetch: impl FnOnce() -> TaskList,
    ) {
        let key = (project_id.to_owned(), closed);
        if !refresh {
            let tiers = self.tiers.lock().unwrap_or_else(PoisonError::into_inner);
            if tiers
                .get(&key)
                .is_some_and(|tier| !tier.stale && tier.at.elapsed() < FRESH_FOR)
            {
                return;
            }
        }
        let request = {
            let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
            let request = self.next_request.fetch_add(1, Ordering::Relaxed);
            requests.insert(key.clone(), request);
            request
        };
        let found = fetch();
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        if requests.get(&key) != Some(&request) {
            return;
        }
        let mut tiers = self.tiers.lock().unwrap_or_else(PoisonError::into_inner);
        let found = match tiers.remove(&key) {
            // Nothing arrived at all: what was known is still the best there is, with the
            // reason it could not be brought up to date.
            Some(before) if !found.answered => TaskList {
                problem: found.problem,
                logged_out: found.logged_out,
                ..before.found
            },
            _ => found,
        };
        tiers.insert(
            key,
            Tier {
                at: Instant::now(),
                found,
                stale: false,
            },
        );
    }

    /// A project's tasks as last read: the open ones, and the closed ones when `closed`.
    fn composed(&self, project_id: &str, closed: bool, repo: Option<Repo>) -> ProjectTasks {
        let tiers = self.tiers.lock().unwrap_or_else(PoisonError::into_inner);
        let mut answer = ProjectTasks {
            gh: true,
            repo,
            ..Default::default()
        };
        if let Some(open) = tiers.get(&(project_id.to_owned(), false)) {
            let open = &open.found;
            answer.tasks = open.tasks.clone();
            answer.problem = open.problem.clone();
            answer.logged_out = open.logged_out;
            answer.viewer = open.viewer.clone();
            answer.open_total = open.total;
            answer.disabled = open.disabled;
        }
        let shut = tiers.get(&(project_id.to_owned(), true)).filter(|_| closed);
        if let Some(shut) = shut {
            let shut = &shut.found;
            // One that was reopened between the two questions is in both: the open tier's is
            // the one asked more often.
            let known: Vec<&str> = answer.tasks.iter().map(|task| task.key.as_str()).collect();
            let older: Vec<Task> = shut
                .tasks
                .iter()
                .filter(|task| !known.contains(&task.key.as_str()))
                .cloned()
                .collect();
            answer.tasks.extend(older);
            answer.closed = shut.answered;
            // Said once: when the open ones could not be read either, that is the reason.
            if answer.problem.is_none() {
                answer.problem = shut.problem.clone();
                answer.logged_out = shut.logged_out;
            }
        }
        answer
    }

    /// Something was just written to this project's source: what is kept is from before. The
    /// rows stay — a list that empties and refills is worse than one a moment out of date —
    /// but the next look asks again, and an answer still on its way is dropped.
    fn forget(&self, project_id: &str) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(project, _), _| project != project_id);
        let mut tiers = self.tiers.lock().unwrap_or_else(PoisonError::into_inner);
        for ((project, _), tier) in tiers.iter_mut() {
            if project == project_id {
                tier.stale = true;
            }
        }
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(project, _), _| project != project_id);
    }

    /// Remember that `task` was made ready to delegate in this project.
    fn issue(&self, project_id: &str, task: &TaskRef) {
        let mut issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        let known = issued.entry(project_id.to_owned()).or_default();
        if !known.contains(task) {
            known.push(task.clone());
        }
    }

    /// Whether `task` is one the core itself read for this project and handed to the window.
    ///
    /// A workspace records the task it was started from, and the window passes that task back
    /// when it asks for the workspace. The window holds no truth: only a reference that came
    /// from the source, for this project, in this run of the app, is written down. One made up,
    /// altered, or prepared for another project is refused.
    pub fn issued(&self, project_id: &str, task: &TaskRef) -> bool {
        let issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        issued
            .get(project_id)
            .is_some_and(|known| known.contains(task))
    }

    /// One task in full, from the cache when it was read less than [`FRESH_FOR`] ago. Unlike
    /// the list this is an `Err` when the source cannot answer: the detail pane has nothing
    /// else to show, and says why in its place.
    fn detail(
        &self,
        project_id: &str,
        key: &str,
        refresh: bool,
        fetch: impl FnOnce() -> ForgeResult<TaskDetail>,
    ) -> ForgeResult<TaskDetail> {
        let cache_key = (project_id.to_owned(), key.to_owned());
        if !refresh {
            let details = self.details.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, detail)) = details.get(&cache_key) {
                if at.elapsed() < FRESH_FOR {
                    return Ok(detail.clone());
                }
            }
        }
        let detail = fetch()?;
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key, (Instant::now(), detail.clone()));
        Ok(detail)
    }
}

fn look(state: &AppState, project_id: &str, refresh: bool, closed: bool) -> ProjectTasks {
    let Ok(root) = project_root(state, project_id) else {
        return ProjectTasks::default();
    };
    let repo = repo_at(state, &root);
    let Some(gh) = Gh::find(&state.env()) else {
        return ProjectTasks {
            repo,
            ..Default::default()
        };
    };
    // Another forge, or none: nothing is asked, and the view says which from `repo`.
    let Coverage::Covered(host) = coverage(repo.as_ref()) else {
        return ProjectTasks {
            gh: true,
            repo,
            ..Default::default()
        };
    };
    let source = GitHub::new(&gh, host);
    state.tasks.load(project_id, false, refresh, || {
        source.list(&root, TaskState::Open, OPEN_CAP)
    });
    if closed {
        state.tasks.load(project_id, true, refresh, || {
            source.list(&root, TaskState::Closed, CLOSED_CAP)
        });
    }
    state.tasks.composed(project_id, closed, repo)
}

/// A project's tasks: every open one up to the cap, and with `closed` the most recently
/// closed as well. Never an `Err` — a source that cannot be reached is a notice above the list,
/// and the reason travels in [`ProjectTasks::problem`].
#[tauri::command]
#[specta::specta]
pub async fn project_tasks(
    app: AppHandle,
    project_id: String,
    refresh: bool,
    closed: bool,
) -> IpcResult<ProjectTasks> {
    blocking(app, move |state| {
        Ok(look(state, &project_id, refresh, closed))
    })
    .await
}

/// One task in full: its description and its conversation. What the Tasks view's detail pane
/// shows, read when a row is opened.
#[tauri::command]
#[specta::specta]
pub async fn task_detail(
    app: AppHandle,
    project_id: String,
    key: String,
    refresh: bool,
) -> IpcResult<TaskDetail> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let Coverage::Covered(host) = coverage(repo_at(state, &root).as_ref()) else {
            return Err(crate::error::IpcError::new(
                "tasks_not_covered",
                "This project's tasks cannot be read: it is not on GitHub.",
            ));
        };
        let source = GitHub::new(&gh, host);
        state
            .tasks
            .detail(&project_id, &key, refresh, || source.show(&root, &key))
            .map_err(failed)
    })
    .await
}

/// A task made ready to hand to an agent: the first message, built from the task as its
/// source has it this moment, and what to record against the workspace that is started. The
/// composer shows the message for the user to read and edit; nothing starts here.
#[tauri::command]
#[specta::specta]
pub async fn task_prompt(app: AppHandle, project_id: String, key: String) -> IpcResult<Delegated> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let Coverage::Covered(host) = coverage(repo_at(state, &root).as_ref()) else {
            return Err(crate::error::IpcError::new(
                "tasks_not_covered",
                "This project's tasks cannot be read: it is not on GitHub.",
            ));
        };
        let source = GitHub::new(&gh, host);
        let detail = state
            .tasks
            .detail(&project_id, &key, true, || source.show(&root, &key))
            .map_err(failed)?;
        let delegated = delegated(&detail);
        state.tasks.issue(&project_id, &delegated.task);
        Ok(delegated)
    })
    .await
}

/// Run `write` against a project's source, and forget what was known of the project whatever
/// came of it: a refusal usually means the list was out of date.
fn writing<T>(
    state: &AppState,
    project_id: &str,
    write: impl FnOnce(&GitHub<'_>, &std::path::Path) -> ForgeResult<T>,
) -> IpcResult<T> {
    let root = project_root(state, project_id)?;
    let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
    let Coverage::Covered(host) = coverage(repo_at(state, &root).as_ref()) else {
        return Err(crate::error::IpcError::new(
            "tasks_not_covered",
            "This project's tasks cannot be changed: it is not on GitHub.",
        ));
    };
    let result = write(&GitHub::new(&gh, host), &root);
    state.tasks.forget(project_id);
    result.map_err(failed)
}

/// Open a new task in a project. Pressing Create was the confirmation.
#[tauri::command]
#[specta::specta]
pub async fn task_create(
    app: AppHandle,
    project_id: String,
    task: NewTask,
) -> IpcResult<CreatedTask> {
    blocking(app, move |state| {
        writing(state, &project_id, |source, root| {
            source.create(root, &task)
        })
    })
    .await
}

/// Post a comment on a task. The words are the user's, sent as they are; pressing Send was the
/// confirmation.
#[tauri::command]
#[specta::specta]
pub async fn task_comment(
    app: AppHandle,
    project_id: String,
    key: String,
    body: String,
) -> IpcResult<()> {
    blocking(app, move |state| {
        writing(state, &project_id, |source, root| {
            source.comment(root, &key, body.trim())
        })
    })
    .await
}

/// Close a task. The window asks first; nothing is posted on it and nothing is deleted.
#[tauri::command]
#[specta::specta]
pub async fn task_close(
    app: AppHandle,
    project_id: String,
    key: String,
    reason: CloseReason,
) -> IpcResult<()> {
    blocking(app, move |state| {
        writing(state, &project_id, |source, root| {
            source.close(root, &key, reason)
        })
    })
    .await
}

/// Reopen a closed task. The window asks first: it tells everyone following it.
#[tauri::command]
#[specta::specta]
pub async fn task_reopen(app: AppHandle, project_id: String, key: String) -> IpcResult<()> {
    blocking(app, move |state| {
        writing(state, &project_id, |source, root| source.reopen(root, &key))
    })
    .await
}

/// Change a task's labels, assignees or title.
#[tauri::command]
#[specta::specta]
pub async fn task_edit(
    app: AppHandle,
    project_id: String,
    key: String,
    change: TaskEdit,
) -> IpcResult<()> {
    blocking(app, move |state| {
        writing(state, &project_id, |source, root| {
            source.edit(root, &key, &change)
        })
    })
    .await
}

/// What a task of this project can be labelled with and who it can be assigned to: what the
/// pickers offer. Asked for when one is opened, not kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskChoices {
    pub labels: Vec<TaskLabel>,
    pub assignees: Vec<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn project_task_choices(app: AppHandle, project_id: String) -> IpcResult<TaskChoices> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let Coverage::Covered(host) = coverage(repo_at(state, &root).as_ref()) else {
            return Ok(TaskChoices::default());
        };
        let source = GitHub::new(&gh, host);
        Ok(TaskChoices {
            labels: source.labels(&root).map_err(failed)?,
            assignees: source.assignees(&root).map_err(failed)?,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskSourceKind;

    fn task(key: &str, state: TaskState) -> Task {
        Task {
            source: TaskSourceKind::GitHub,
            key: key.to_owned(),
            url: format!("https://github.com/o/r/issues/{}", &key[1..]),
            title: format!("Task {key}"),
            state,
            closed_as: None,
            author: Some("grace".to_owned()),
            labels: vec![],
            assignees: vec![],
            comments: 0,
            created_at: "2026-10-01T00:00:00Z".to_owned(),
            updated_at: "2026-10-01T00:00:00Z".to_owned(),
            needs_answer: false,
            linked_pull_requests: vec![],
        }
    }

    fn answered(keys: &[&str], state: TaskState) -> TaskList {
        TaskList {
            tasks: keys.iter().map(|key| task(key, state)).collect(),
            total: Some(keys.len() as u32),
            viewer: Some("ada".to_owned()),
            answered: true,
            ..Default::default()
        }
    }

    fn keys(found: &ProjectTasks) -> Vec<&str> {
        found.tasks.iter().map(|task| task.key.as_str()).collect()
    }

    #[test]
    fn a_fresh_answer_is_reused_and_refresh_asks_again() {
        let cache = TaskCache::default();
        cache.load("p", false, false, || {
            answered(&["#2", "#1"], TaskState::Open)
        });
        cache.load("p", false, false, || panic!("it was read a moment ago"));
        assert_eq!(keys(&cache.composed("p", false, None)), ["#2", "#1"]);

        cache.load("p", false, true, || answered(&["#3"], TaskState::Open));
        let found = cache.composed("p", false, None);
        assert_eq!(keys(&found), ["#3"]);
        assert_eq!(found.open_total, Some(1));
        assert_eq!(found.viewer.as_deref(), Some("ada"));
        assert!(found.gh);
    }

    #[test]
    fn an_answer_that_never_arrived_keeps_what_was_known_and_says_why() {
        let cache = TaskCache::default();
        cache.load("p", false, false, || {
            answered(&["#2", "#1"], TaskState::Open)
        });
        cache.load("p", false, true, || TaskList {
            problem: Some("HTTP 502".to_owned()),
            ..Default::default()
        });
        let found = cache.composed("p", false, None);
        assert_eq!(keys(&found), ["#2", "#1"], "the list is not emptied");
        assert_eq!(found.problem.as_deref(), Some("HTTP 502"));
        assert_eq!(found.open_total, Some(2));

        // Half an answer replaces a whole one: it is what the source says now.
        cache.load("p", false, true, || TaskList {
            problem: Some("HTTP 504".to_owned()),
            ..answered(&["#9"], TaskState::Open)
        });
        let found = cache.composed("p", false, None);
        assert_eq!(keys(&found), ["#9"]);
        assert_eq!(found.problem.as_deref(), Some("HTTP 504"));
    }

    #[test]
    fn closed_tasks_are_part_of_the_answer_only_when_asked_for() {
        let cache = TaskCache::default();
        cache.load("p", false, false, || {
            answered(&["#3", "#2"], TaskState::Open)
        });
        cache.load("p", true, false, || {
            answered(&["#2", "#1"], TaskState::Closed)
        });

        let open = cache.composed("p", false, None);
        assert_eq!(keys(&open), ["#3", "#2"]);
        assert!(!open.closed);

        let all = cache.composed("p", true, None);
        assert_eq!(keys(&all), ["#3", "#2", "#1"], "one in both is listed once");
        assert_eq!(
            all.tasks[1].state,
            TaskState::Open,
            "and it is the open tier's"
        );
        assert!(all.closed);
        assert_eq!(all.open_total, Some(2), "the total is of the open ones");

        // Another project's are its own.
        assert!(cache.composed("q", true, None).tasks.is_empty());
    }

    #[test]
    fn a_closed_tier_that_failed_says_so_unless_the_open_one_already_did() {
        let cache = TaskCache::default();
        cache.load("p", false, false, || answered(&["#3"], TaskState::Open));
        cache.load("p", true, false, || TaskList {
            problem: Some("HTTP 502".to_owned()),
            ..Default::default()
        });
        let found = cache.composed("p", true, None);
        assert_eq!(found.problem.as_deref(), Some("HTTP 502"));
        assert!(!found.closed, "they were not read");
        assert_eq!(cache.composed("p", false, None).problem, None);
    }

    #[test]
    fn after_a_write_the_rows_stay_and_the_next_look_asks_again() {
        let cache = TaskCache::default();
        let detail = TaskDetail {
            task: task("#2", TaskState::Open),
            body: "Before.".to_owned(),
            comments: vec![],
        };
        cache.load("p", false, false, || {
            answered(&["#2", "#1"], TaskState::Open)
        });
        cache.load("q", false, false, || answered(&["#9"], TaskState::Open));
        cache
            .detail("p", "#2", false, || Ok(detail.clone()))
            .unwrap();

        cache.forget("p");
        assert_eq!(
            keys(&cache.composed("p", false, None)),
            ["#2", "#1"],
            "the list is not emptied"
        );
        // Not refreshing, and still asked: what was kept is from before the write.
        cache.load("p", false, false, || answered(&["#1"], TaskState::Open));
        assert_eq!(keys(&cache.composed("p", false, None)), ["#1"]);
        let again = cache.detail("p", "#2", false, || {
            Ok(TaskDetail {
                body: "After.".to_owned(),
                ..detail.clone()
            })
        });
        assert_eq!(again.unwrap().body, "After.");
        // Another project's is its own.
        cache.load("q", false, false, || panic!("nothing was written there"));
    }

    #[test]
    fn only_a_reference_the_core_handed_out_for_that_project_is_one_it_issued() {
        let cache = TaskCache::default();
        let it = TaskRef::of(&task("#7", TaskState::Open));
        assert!(!cache.issued("p", &it), "nothing has been prepared");
        cache.issue("p", &it);
        cache.issue("p", &it);
        assert!(cache.issued("p", &it));
        assert!(
            !cache.issued("q", &it),
            "it was prepared for another project"
        );
        let altered = TaskRef {
            url: "https://github.com/someone/else/issues/7".to_owned(),
            ..it.clone()
        };
        assert!(!cache.issued("p", &altered));
    }

    #[test]
    fn a_task_in_full_is_kept_briefly_and_a_failure_is_not_kept_at_all() {
        let cache = TaskCache::default();
        let detail = || TaskDetail {
            task: task("#7", TaskState::Open),
            body: "The description.".to_owned(),
            comments: vec![],
        };
        let refused = cache.detail("p", "#7", false, || {
            Err(ForgeError::Unreadable("expected an issue".to_owned()))
        });
        assert!(refused.is_err());
        let read = cache.detail("p", "#7", false, || Ok(detail()));
        assert_eq!(read.unwrap(), detail());
        let again = cache.detail("p", "#7", false, || panic!("it was read a moment ago"));
        assert_eq!(again.unwrap().body, "The description.");
        let fresh = cache.detail("p", "#7", true, || {
            Ok(TaskDetail {
                body: "Edited.".to_owned(),
                ..detail()
            })
        });
        assert_eq!(fresh.unwrap().body, "Edited.");
    }
}

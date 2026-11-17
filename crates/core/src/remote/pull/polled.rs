//! Pull requests read for a device by a host with no window: `yardsort-host`.
//!
//! The desktop app lends what its window last read. A host with no window has no window to read
//! them, so this reads them itself, the same way and only when a device asks: a question finds
//! what was last read and has it at once, and when that is old, or there is nothing, a reading
//! is started on a thread of its own and the answer says one is on its way. The device asks
//! again and has it. Nothing here waits for `gh`, since a device's channel also carries its
//! terminal.
//!
//! The reading is `gh pr list` in the project's checkout, with the sign-in `gh` has on this
//! computer, and `gh`'s open pull requests when a device asks for every open one. Which
//! workspace opened which is worked out as the window does ([`crate::forge::pull_requests_from`]).
//! Pull requests a workspace follows in another repository are not read here yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{ProjectPulls, PullRequests};
use crate::forge::{pull_requests_from, ForgeKind, Gh, PullRequest};
use crate::git::{Git, Head};
use crate::store::Store;

/// How old a project's pull requests may be before a device's question has them read again. A
/// phone asks about once a minute while it shows them, as for the desktop app's.
pub const OLD_AFTER: Duration = Duration::from_secs(45);

/// How soon a project is read again when the last reading began: a project whose reading left
/// nothing, or failed, is not read again at every question.
pub const AGAIN_AFTER: Duration = Duration::from_secs(10);

/// As many pull requests as the window's own list reads.
const RECENT: u32 = 50;
/// As many open ones as the window's Pull requests view reads.
const OPEN_CAP: usize = 500;
/// How long one page of open pull requests may take.
const PAGE_LIMIT: Duration = Duration::from_secs(20);

/// What one reading found.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub pull_requests: Vec<PullRequest>,
    pub workspaces: BTreeMap<String, Vec<u32>>,
    /// Every open one was read too.
    pub every_open: bool,
}

/// Reads a project's pull requests: `None` when there is no project to read, such as one
/// forgotten since. Blocking: it runs `gh`.
pub type Reader = dyn Fn(&str, bool) -> Option<Reading> + Send + Sync;

struct Kept {
    at: Instant,
    /// When every open one was last read.
    open_at: Option<Instant>,
    found: Reading,
}

#[derive(Default)]
struct Marks {
    kept: HashMap<String, Kept>,
    reading: HashSet<String>,
    began: HashMap<String, Instant>,
    began_full: HashMap<String, Instant>,
}

/// A host's own reading of its projects' pull requests, lent to a device's channel.
pub struct Polled {
    marks: Arc<Mutex<Marks>>,
    read: Arc<Reader>,
}

impl Polled {
    pub fn new(read: Arc<Reader>) -> Self {
        Self {
            marks: Arc::default(),
            read,
        }
    }

    /// Whether to read `project_id` now, and if so whether every open one too. `Some` once for
    /// each reading, which is then marked as being read until it is over.
    fn begin(&self, project_id: &str, every_open: bool, now: Instant) -> Option<bool> {
        let mut marks = self.marks.lock().unwrap_or_else(PoisonError::into_inner);
        if marks.reading.contains(project_id) {
            return None;
        }
        let since = |began: &HashMap<String, Instant>, gap: Duration| {
            began
                .get(project_id)
                .is_none_or(|at| now.saturating_duration_since(*at) >= gap)
        };
        let kept = marks.kept.get(project_id);
        let recent = kept.is_none_or(|kept| now.saturating_duration_since(kept.at) >= OLD_AFTER)
            && since(&marks.began, AGAIN_AFTER);
        let full = every_open
            && kept
                .and_then(|kept| kept.open_at)
                .is_none_or(|at| now.saturating_duration_since(at) >= OLD_AFTER)
            && since(&marks.began_full, OLD_AFTER);
        if !recent && !full {
            return None;
        }
        marks.reading.insert(project_id.to_owned());
        marks.began.insert(project_id.to_owned(), now);
        if full {
            marks.began_full.insert(project_id.to_owned(), now);
        }
        Some(full)
    }

    /// Reads `project_id` on a thread of its own.
    fn start(&self, project_id: &str, full: bool) {
        let marks = Arc::clone(&self.marks);
        let read = Arc::clone(&self.read);
        let project_id = project_id.to_owned();
        let spawned = std::thread::Builder::new()
            .name("pull-requests".into())
            .spawn({
                let marks = Arc::clone(&marks);
                let project_id = project_id.clone();
                move || {
                    // Over however it ends, a `gh` that panicked the parse included.
                    let over = Over {
                        marks: Arc::clone(&marks),
                        project_id: project_id.clone(),
                    };
                    let found = read(&project_id, full);
                    keep(&marks, &project_id, found, Instant::now());
                    drop(over);
                }
            });
        if spawned.is_err() {
            marks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reading
                .remove(&project_id);
        }
    }
}

/// Put a reading's answer in place of what was kept. A reading of only the newest keeps the
/// older open ones the last full one found, and when it was.
fn keep(marks: &Mutex<Marks>, project_id: &str, found: Option<Reading>, now: Instant) {
    let mut marks = marks.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(mut found) = found else {
        marks.kept.remove(project_id);
        return;
    };
    let open_at = if found.every_open {
        Some(now)
    } else {
        let before = marks.kept.remove(project_id);
        match before {
            Some(before) if before.open_at.is_some() => {
                let older: Vec<PullRequest> = before
                    .found
                    .pull_requests
                    .into_iter()
                    .filter(|pr| !found.pull_requests.iter().any(|n| n.number == pr.number))
                    .filter(|pr| pr.state == crate::forge::PullRequestState::Open)
                    .collect();
                // The newest alone cannot say which workspace opened an older one: the last
                // full reading did, and that holds for as long as the pull request is kept.
                for (workspace, numbers) in before.found.workspaces {
                    let kept: Vec<u32> = numbers
                        .into_iter()
                        .filter(|n| older.iter().any(|pr| pr.number == *n))
                        .collect();
                    if kept.is_empty() {
                        continue;
                    }
                    let own = found.workspaces.entry(workspace).or_default();
                    for number in kept {
                        if !own.contains(&number) {
                            own.push(number);
                        }
                    }
                }
                found.pull_requests.extend(older);
                before.open_at
            }
            _ => None,
        }
    };
    marks.kept.insert(
        project_id.to_owned(),
        Kept {
            at: now,
            open_at,
            found,
        },
    );
}

struct Over {
    marks: Arc<Mutex<Marks>>,
    project_id: String,
}

impl Drop for Over {
    fn drop(&mut self) {
        self.marks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reading
            .remove(&self.project_id);
    }
}

impl PullRequests for Polled {
    fn of_project(&self, project_id: &str, every_open: bool) -> ProjectPulls {
        if let Some(full) = self.begin(project_id, every_open, Instant::now()) {
            self.start(project_id, full);
        }
        let marks = self.marks.lock().unwrap_or_else(PoisonError::into_inner);
        let reading = marks.reading.contains(project_id);
        match marks.kept.get(project_id) {
            Some(kept) => ProjectPulls {
                pull_requests: kept.found.pull_requests.clone(),
                workspaces: kept.found.workspaces.clone(),
                followed: Vec::new(),
                reading,
            },
            None => ProjectPulls {
                reading,
                ..Default::default()
            },
        }
    }
}

/// Read a project's pull requests with `gh`, in its checkout, and share them out among its
/// workspaces. `None` for a project that is not in the store. Without `gh`, or when it fails,
/// the reading is of none: a device shows no pull requests, as the window shows no badges.
pub fn read(
    store: &Store,
    git: &Git,
    gh: Option<&Gh>,
    project_id: &str,
    every_open: bool,
) -> Option<Reading> {
    let project = store.project(project_id).ok()??;
    let root = Path::new(&project.root_path);
    let Some(gh) = gh else {
        return Some(Reading::default());
    };
    let mut pull_requests = match gh.pull_requests(root, RECENT) {
        Ok(found) => found,
        // Signed out, the open ones cannot be read either.
        Err(error) if error.is_logged_out() => {
            eprintln!("pull requests: could not read {}: {error}", project.name);
            return Some(Reading::default());
        }
        // The open ones are asked another way, which can answer when this timed out: as the
        // window does, they are still asked for.
        Err(error) => {
            eprintln!("pull requests: could not read {}: {error}", project.name);
            Vec::new()
        }
    };
    let mut read_open = false;
    if every_open {
        // As the window: not of a forge `gh` does not speak to, and of a GitHub of its own name.
        let repo = crate::forge::repo_at(git, root);
        let host = match &repo {
            Some(repo) => match repo.kind {
                ForgeKind::GitLab | ForgeKind::Bitbucket => None,
                ForgeKind::GitHub if repo.host != "github.com" => Some(Some(repo.host.as_str())),
                ForgeKind::GitHub | ForgeKind::Unknown => Some(None),
            },
            None => None,
        };
        if let Some(host) = host {
            let open = gh.open_pull_requests(root, host, OPEN_CAP, PAGE_LIMIT);
            read_open = open.answered;
            for pr in open.pull_requests {
                if !pull_requests.iter().any(|seen| seen.number == pr.number) {
                    pull_requests.push(pr);
                }
            }
        }
    }
    let workspaces = by_workspace(store, git, project_id, &pull_requests);
    Some(Reading {
        pull_requests,
        workspaces,
        every_open: read_open,
    })
}

/// Each workspace's own, by number, the one it shows first leading.
fn by_workspace(
    store: &Store,
    git: &Git,
    project_id: &str,
    requests: &[PullRequest],
) -> BTreeMap<String, Vec<u32>> {
    let mut found = BTreeMap::new();
    if requests.is_empty() {
        return found;
    }
    let Ok(rows) = store.workspaces() else {
        return found;
    };
    for row in rows
        .into_iter()
        .filter(|row| row.project_id == project_id && !row.archived && !row.forgotten)
    {
        let root = Path::new(&row.path);
        if !root.is_dir() {
            continue;
        }
        let Ok(head) = git.head(root) else { continue };
        let branch = match head {
            Head::Branch(name) => Some(name),
            Head::Unborn(_) | Head::Detached(_) => None,
        };
        let Ok(history) = git.head_history(root) else {
            continue;
        };
        let mut numbers =
            pull_requests_from(requests, branch.as_deref(), row.branch.as_deref(), &history);
        // A workspace started from a fork's pull request says which one in git's config.
        let followed = branch
            .as_deref()
            .and_then(|branch| git.followed_pull_request(root, branch).ok().flatten());
        if let Some(number) = followed {
            if requests.iter().any(|pr| pr.number == number) && !numbers.contains(&number) {
                numbers.insert(0, number);
            }
        }
        found.insert(row.id, numbers);
    }
    found
}

#[cfg(test)]
mod tests;

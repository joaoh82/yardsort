//! The pull request events: what changed on a workspace's pull request since the last look.
//!
//! A `pr.*` event is a difference, not a state. The store keeps what the last look saw
//! ([`PrWatchRow`]): which pull request, on which commit, what its checks came to and whether
//! that was told, whether it conflicts, and every review and comment seen. [`compare`] puts
//! the next look beside it and says what is news, as [`Observation`]s for
//! [`super::fire::observe`]. It is a function of the two and the time, so every rule is a test.
//!
//! - **Seeded first.** A watch is written from what the forge says when a trigger on a `pr.*`
//!   event is saved ([`seed`]), and fires nothing then: a pull request already open is not
//!   `pr.opened`, and checks already red are not `pr.checks_failed`. When the forge could not be
//!   asked then, the first look does the seeding. Reviews and comments are news either way when
//!   they are newer than the trigger, which `observe` sees to.
//! - **Each change once.** Checks that finished are told once per commit and verdict; a conflict
//!   once per commit; a post once.
//! - **A pull request merged or closed ends the watch's subject.** The next one on the branch is
//!   `pr.opened` again.
//!
//! Where the looks happen — a thread of the app's, once a minute — is the app's business.

use std::collections::BTreeMap;
use std::path::Path;

use super::fire::{Observation, Post};
use super::TriggerEvent;
use crate::error::IpcResult;
use crate::forge::{Mergeable, PrLook, PullRequestState, WatchPostKind};
use crate::git::Git;
use crate::store::{PrWatchRow, Store, WorkspaceRow};
use crate::workflow::runs::Look;

/// How often a watched pull request is looked at.
pub const LOOK_EVERY_MS: i64 = 60_000;

/// What a watch knows before it has looked.
fn blank(workspace_id: &str) -> PrWatchRow {
    PrWatchRow {
        workspace_id: workspace_id.to_owned(),
        ..PrWatchRow::default()
    }
}

/// Workspaces an enabled trigger on a `pr.*` event listens to.
pub fn watched(store: &Store) -> IpcResult<Vec<WorkspaceRow>> {
    let mut ids: Vec<String> = store
        .triggers()?
        .into_iter()
        .filter(|trigger| {
            trigger.enabled
                && super::events_of(trigger)
                    .iter()
                    .any(|event| event.of_a_pull_request())
        })
        .filter_map(|trigger| trigger.workspace_id)
        .collect();
    ids.sort();
    ids.dedup();
    let mut found = Vec::new();
    for id in ids {
        // `workspace_triggers` leaves out a removed project's.
        if store.workspace_triggers(&id)?.is_empty() {
            continue;
        }
        if let Some(workspace) = store
            .workspace(&id)?
            .filter(|w| !w.archived && !w.forgotten)
        {
            found.push(workspace);
        }
    }
    Ok(found)
}

/// The watched workspaces whose pull request is due a look at `now`. Watches nothing listens
/// to any more go, so a trigger saved later starts from what is true then.
pub fn due(store: &Store, now: i64) -> IpcResult<Vec<WorkspaceRow>> {
    let watched = watched(store)?;
    for watch in store.pr_watches()? {
        if !watched.iter().any(|w| w.id == watch.workspace_id) {
            store.remove_pr_watch(&watch.workspace_id)?;
        }
    }
    let mut due = Vec::new();
    for workspace in watched {
        let looked_at = store.pr_watch(&workspace.id)?.map_or(0, |w| w.looked_at);
        if now - looked_at >= LOOK_EVERY_MS {
            due.push(workspace);
        }
    }
    Ok(due)
}

/// Write the workspace's watch from what the forge says now, firing nothing: what is true when
/// a trigger is saved is not news to it. When the forge cannot be asked the watch is left
/// unseeded, for the first look to seed.
///
/// A watch already there is left alone. Another trigger is watching with it, so it is as
/// current as a look, and seeding it again would count as seen what that trigger has not been
/// told yet. A trigger switched back on needs nothing either: its watch is current, or was
/// dropped while nothing watched and is seeded at its first look, and what happened while it
/// was off is older than it is.
pub fn seed(store: &Store, look: &dyn Look, workspace: &WorkspaceRow, now: i64) -> IpcResult<()> {
    if store.pr_watch(&workspace.id)?.is_some() {
        return Ok(());
    }
    let mut watch = blank(&workspace.id);
    match look.watch_pull_request(workspace, None) {
        Ok(found) => {
            watch.seeded = true;
            let (_, seeded) = compare(&watch, found.as_ref(), now);
            watch = seeded;
        }
        Err(why) => watch.problem = Some(why),
    }
    store.put_pr_watch(&watch)?;
    Ok(())
}

/// Look at the workspace's pull request, write what was seen, and return what is news. A look
/// the forge could not answer is written down as a problem and is news of nothing.
pub fn look_at(
    store: &Store,
    look: &dyn Look,
    git: &Git,
    workspace: &WorkspaceRow,
    now: i64,
) -> IpcResult<Vec<Observation>> {
    let mut old = store
        .pr_watch(&workspace.id)?
        .unwrap_or_else(|| blank(&workspace.id));
    // Another branch checked out has another pull request, and what is true of that one is not
    // news: it is seeded again.
    if let (Some(watched), Ok(crate::git::Head::Branch(now_on))) =
        (old.branch.as_deref(), git.head(Path::new(&workspace.path)))
    {
        if watched != now_on {
            old = blank(&workspace.id);
        }
    }
    match look.watch_pull_request(workspace, old.number) {
        Ok(found) => {
            let (seen, next) = compare(&old, found.as_ref(), now);
            store.put_pr_watch(&next)?;
            Ok(seen)
        }
        Err(why) => {
            old.looked_at = now;
            old.problem = Some(why);
            store.put_pr_watch(&old)?;
            Ok(Vec::new())
        }
    }
}

/// What is news in `found` beside `old`, and the watch to keep. `found` is the pull request
/// `old` names, or the branch's open one when it names none; `None` when the branch has none.
pub fn compare(
    old: &PrWatchRow,
    found: Option<&PrLook>,
    now: i64,
) -> (Vec<Observation>, PrWatchRow) {
    let mut next = old.clone();
    next.looked_at = now;
    next.problem = None;
    // Until it is seeded only reviews and comments are told, and only those newer than the
    // trigger: the states are recorded.
    let tell = old.seeded;
    next.seeded = true;
    let Some(pr) = found else {
        return (
            Vec::new(),
            PrWatchRow {
                looked_at: now,
                seeded: true,
                ..blank(&old.workspace_id)
            },
        );
    };
    let fresh = old.number != Some(pr.number);
    if fresh {
        next = PrWatchRow {
            seeded: true,
            looked_at: now,
            ..blank(&old.workspace_id)
        };
    }
    next.number = Some(pr.number);
    next.branch = Some(pr.branch.clone()).filter(|b| !b.is_empty());

    let mut facts = BTreeMap::new();
    facts.insert("url".to_owned(), pr.url.clone());
    facts.insert("number".to_owned(), pr.number.to_string());
    facts.insert("title".to_owned(), pr.title.clone());
    let mut seen = Vec::new();
    let news = |event: TriggerEvent, at: i64, more: &[(&str, String)], status: Option<&str>| {
        let mut facts = facts.clone();
        for (key, value) in more {
            facts.insert((*key).to_owned(), value.clone());
        }
        Observation {
            event,
            workspace_id: old.workspace_id.clone(),
            session_id: None,
            terminal: None,
            agent_run: None,
            at,
            facts,
            depth: 0,
            target_id: None,
            status: status.map(str::to_owned),
            post: None,
        }
    };

    if pr.state != PullRequestState::Open {
        // One seen open before is news; one that was never seen open is not.
        if tell && !fresh {
            let event = match pr.state {
                PullRequestState::Merged => TriggerEvent::PrMerged,
                _ => TriggerEvent::PrClosed,
            };
            seen.push(news(event, now, &[], None));
        }
        // Its subject ends here: the next pull request on the branch opens afresh.
        let ended = PrWatchRow {
            seeded: true,
            looked_at: now,
            branch: next.branch.clone(),
            ..blank(&old.workspace_id)
        };
        return (seen, ended);
    }

    if tell && fresh {
        let status = if pr.draft { "draft" } else { "open" };
        seen.push(news(TriggerEvent::PrOpened, now, &[], Some(status)));
    } else if tell && old.draft && !pr.draft {
        // Marked ready: opened, for a trigger that leaves drafts out.
        seen.push(news(TriggerEvent::PrOpened, now, &[], Some("ready")));
    }
    next.draft = pr.draft;
    next.head = Some(pr.head.clone()).filter(|h| !h.is_empty());

    let counts = pr.check_counts;
    if counts.running == 0 && counts.total() > 0 {
        let verdict = if counts.failed > 0 {
            "failed"
        } else {
            "passed"
        };
        let told = next.checks_head.as_deref() == Some(pr.head.as_str())
            && next.checks.as_deref() == Some(verdict);
        if !told {
            if tell {
                if counts.failed > 0 {
                    let summary = format!(
                        "{} failed: {}",
                        plural(counts.failed, "check"),
                        pr.failed_checks.join(", ")
                    );
                    seen.push(news(
                        TriggerEvent::PrChecksFailed,
                        now,
                        &[("head", pr.head.clone()), ("summary", summary)],
                        None,
                    ));
                } else {
                    seen.push(news(
                        TriggerEvent::PrChecksPassed,
                        now,
                        &[("head", pr.head.clone())],
                        None,
                    ));
                }
            }
            next.checks_head = Some(pr.head.clone());
            next.checks = Some(verdict.to_owned());
        }
    }

    match pr.mergeable {
        Mergeable::Conflicting if next.conflict_head.as_deref() != Some(pr.head.as_str()) => {
            if tell {
                seen.push(news(
                    TriggerEvent::PrConflicted,
                    now,
                    &[("head", pr.head.clone())],
                    None,
                ));
            }
            next.conflict_head = Some(pr.head.clone());
        }
        Mergeable::Mergeable => next.conflict_head = None,
        // GitHub is still working it out.
        Mergeable::Conflicting | Mergeable::Unknown => {}
    }

    for post in &pr.posts {
        if !next.seen.insert(post.id.clone()) {
            continue;
        }
        let event = match post.kind {
            WatchPostKind::Review => TriggerEvent::PrReview,
            WatchPostKind::Comment | WatchPostKind::Reply => TriggerEvent::PrComment,
        };
        // Whose account `gh` asks as is not known: counted as the user's, so that "others"
        // never counts the agents' own answers.
        let mine = match (&pr.viewer, &post.author) {
            (Some(viewer), Some(author)) => viewer == author,
            (None, _) => true,
            (Some(_), None) => false,
        };
        let mut more = vec![(
            "authors",
            post.author.clone().unwrap_or_else(|| "ghost".to_owned()),
        )];
        let kind = if event == TriggerEvent::PrReview {
            "reviews"
        } else {
            "comments"
        };
        more.push((kind, "1".to_owned()));
        if let Some(url) = &post.url {
            more.push(("url", url.clone()));
        }
        let mut observation = news(event, post.at, &more, None);
        observation
            .facts
            .insert("summary".to_owned(), summary(&observation.facts));
        observation.post = Some(Post {
            author: post.author.clone(),
            mine,
            state: post.state,
        });
        seen.push(observation);
    }
    (seen, next)
}

/// Fold what a review or comment left into what its fire already gathered: the counts add up,
/// the authors are each named once, and the link is to the newest.
pub fn gather_posts(
    gathered: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut facts = new.clone();
    let count = |facts: &BTreeMap<String, String>, key: &str| {
        facts
            .get(key)
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    };
    for key in ["reviews", "comments"] {
        let sum = count(gathered, key) + count(new, key);
        if sum > 0 {
            facts.insert(key.to_owned(), sum.to_string());
        }
    }
    let mut authors: Vec<&str> = Vec::new();
    for list in [gathered.get("authors"), new.get("authors")]
        .into_iter()
        .flatten()
    {
        for author in list.split(", ").filter(|a| !a.is_empty()) {
            if !authors.contains(&author) {
                authors.push(author);
            }
        }
    }
    if !authors.is_empty() {
        facts.insert("authors".to_owned(), authors.join(", "));
    }
    facts.insert("summary".to_owned(), summary(&facts));
    facts
}

/// "2 reviews and 3 comments from alice and coderabbitai[bot]".
fn summary(facts: &BTreeMap<String, String>) -> String {
    let count = |key: &str| {
        facts
            .get(key)
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let what = match (count("reviews"), count("comments")) {
        (0, 0) => "nothing".to_owned(),
        (r, 0) => plural(r, "review"),
        (0, c) => plural(c, "comment"),
        (r, c) => format!("{} and {}", plural(r, "review"), plural(c, "comment")),
    };
    let authors: Vec<&str> = facts
        .get("authors")
        .map(|a| a.split(", ").collect())
        .unwrap_or_default();
    match authors.as_slice() {
        [] => what,
        [one] => format!("{what} from {one}"),
        [rest @ .., last] => format!("{what} from {} and {last}", rest.join(", ")),
    }
}

fn plural(n: u32, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// [`Look::watch_pull_request`] for the app and `ys`: pull request `number`, or else the open
/// one for the branch `workspace` has checked out, as a watch sees it.
pub fn watch_pull_request_of(
    gh: Option<&crate::forge::Gh>,
    git: &Git,
    workspace: &WorkspaceRow,
    number: Option<u32>,
) -> Result<Option<PrLook>, String> {
    let number = match number {
        Some(number) => number,
        None => match crate::workflow::runs::pull_request_of(gh, git, workspace)? {
            Some(pr) => pr.number,
            None => return Ok(None),
        },
    };
    let gh = gh.ok_or_else(|| {
        "Watching the workspace's pull request needs the GitHub CLI (`gh`) on your PATH.".to_owned()
    })?;
    let root = Path::new(&workspace.path);
    // `gh api` asks github.com unless told otherwise, whatever the remote says.
    let host = crate::forge::repo_at(git, root)
        .map(|repo| repo.host)
        .filter(|host| host != "github.com");
    gh.pr_look(
        root,
        host.as_deref(),
        number,
        crate::workflow::runs::PULL_REQUEST_LIMIT,
    )
    .map(Some)
    .map_err(|error| {
        let asked = format!("Could not ask GitHub about #{number}");
        match &error {
            _ if error.is_logged_out() => {
                format!("{asked}: gh is not logged in. Run `gh auth login`.")
            }
            crate::forge::ForgeError::Failed { stderr, .. } => format!("{asked}: gh says {stderr}"),
            other => format!("{asked}: {other}"),
        }
    })
}

#[cfg(test)]
mod tests;

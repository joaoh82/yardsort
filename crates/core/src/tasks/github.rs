//! A GitHub repository's issues as tasks, read through `gh`.
//!
//! Queries of our own through `gh api graphql` rather than `gh issue list`, for what that does
//! not give in one answer: how many issues there are, who is asking, whether the repository has
//! issues at all, the pull requests that will close one, and the *end* of each conversation
//! with who wrote it — which is all that [needing an answer](super::needs_answer) costs. The
//! measurements are in `docs/design/23-tasks.md`.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{
    needs_answer, ClosedAs, Task, TaskComment, TaskDetail, TaskLabel, TaskList, TaskSource,
    TaskSourceKind, TaskState, Voice, PAGE_LIMIT,
};
use crate::forge::{ForgeError, ForgeKind, ForgeResult, Gh, Repo};

/// What is asked about every issue, in a list and by itself.
const FIELDS: &str = "number url title state stateReason createdAt updatedAt authorAssociation \
author{__typename login} labels(first:10){nodes{name color}} assignees(first:5){nodes{login}} \
closedByPullRequestsReferences(first:5,includeClosedPrs:false){nodes{number}}";

/// How much of the end of a conversation a list asks for. The last person in it decides whether
/// an answer is owed, and bots — which are nobody — come in runs: a triage bot and a label bot
/// after every new issue is ordinary.
const LATEST: u32 = 5;

/// How much of a conversation is read for one issue in full: the most GitHub hands over in one
/// answer. A longer one shows its latest hundred and says how many there are.
const CONVERSATION: u32 = 100;

/// One page of a repository's issues in `state`, fifty to the page.
fn list_query(state: TaskState) -> String {
    let states = match state {
        TaskState::Open => "OPEN",
        TaskState::Closed => "CLOSED",
    };
    format!(
        "query($owner:String!,$name:String!,$after:String){{\
repository(owner:$owner,name:$name){{hasIssuesEnabled \
issues(states:{states},first:50,after:$after,orderBy:{{field:UPDATED_AT,direction:DESC}}){{\
totalCount pageInfo{{hasNextPage endCursor}} \
nodes{{{FIELDS} comments(last:{LATEST}){{totalCount nodes{{authorAssociation author{{__typename login}}}}}}}}}}}} \
viewer{{login}}}}"
    )
}

/// One issue, with its description and the end of its conversation.
fn detail_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$number:Int!){{\
repository(owner:$owner,name:$name){{issue(number:$number){{{FIELDS} body \
comments(last:{CONVERSATION}){{totalCount nodes{{authorAssociation author{{__typename login}} \
body createdAt url isMinimized minimizedReason}}}}}}}}}}"
    )
}

/// Whether tasks can be read for a project whose remote is `repo`, and of which host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Yes. The host is `None` for `gh`'s default.
    Covered(Option<String>),
    /// No remote, or one that is a path on disk: there is nowhere to ask.
    NoRemote,
    /// A forge `gh` does not speak to.
    OtherForge(ForgeKind),
}

/// A host this cannot place (an ssh alias, a GitHub Enterprise under a name of its own) is
/// asked through `gh`'s default, as the pull requests are: `gh` knows which host it is logged
/// in to better than a guess from a URL does.
pub fn coverage(repo: Option<&Repo>) -> Coverage {
    match repo {
        None => Coverage::NoRemote,
        Some(repo) => match repo.kind {
            ForgeKind::GitLab | ForgeKind::Bitbucket => Coverage::OtherForge(repo.kind),
            ForgeKind::GitHub if repo.host != "github.com" => {
                Coverage::Covered(Some(repo.host.clone()))
            }
            ForgeKind::GitHub | ForgeKind::Unknown => Coverage::Covered(None),
        },
    }
}

/// A repository's issues, through `gh`.
pub struct GitHub<'a> {
    pub gh: &'a Gh,
    /// Needed for a GitHub that is not github.com: see [`Gh::graphql`].
    pub host: Option<String>,
    /// How long `gh` may take over one answer.
    pub limit: Duration,
}

impl<'a> GitHub<'a> {
    pub fn new(gh: &'a Gh, host: Option<String>) -> Self {
        Self {
            gh,
            host,
            limit: PAGE_LIMIT,
        }
    }

    fn page(&self, root: &Path, state: TaskState, after: Option<&str>) -> ForgeResult<Page> {
        let after = after.map(|cursor| format!("after={cursor}"));
        let mut extra = Vec::new();
        if let Some(after) = &after {
            extra.extend(["-f", after.as_str()]);
        }
        let out = self.gh.graphql(
            root,
            self.host.as_deref(),
            &list_query(state),
            &extra,
            self.limit,
        )?;
        page(&out)
    }
}

impl TaskSource for GitHub<'_> {
    fn list(&self, root: &Path, state: TaskState, cap: usize) -> TaskList {
        let mut found = TaskList::default();
        let mut after: Option<String> = None;
        loop {
            let page = match self.page(root, state, after.as_deref()) {
                Ok(page) => page,
                Err(error) => {
                    found.logged_out = error.is_logged_out();
                    found.problem = Some(error.to_string());
                    break;
                }
            };
            found.answered = true;
            found.disabled = page.disabled;
            found.total = page.total.or(found.total);
            found.viewer = page.viewer.or(found.viewer);
            for task in page.tasks {
                // Ordered by when they last changed, so one that changed between two pages can
                // turn up on both.
                if !found.tasks.iter().any(|seen| seen.key == task.key) {
                    found.tasks.push(task);
                }
            }
            if found.tasks.len() >= cap {
                found.tasks.truncate(cap);
                break;
            }
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        found
    }

    fn show(&self, root: &Path, key: &str) -> ForgeResult<TaskDetail> {
        let number = number(key).ok_or_else(|| ForgeError::Failed {
            command: "issue".to_owned(),
            stderr: format!("{key:?} is not an issue's number"),
        })?;
        let number = format!("number={number}");
        let out = self.gh.graphql(
            root,
            self.host.as_deref(),
            &detail_query(),
            // `-F`, so that it arrives as the number the query declares and not as text.
            &["-F", &number],
            self.limit,
        )?;
        let detail = detail(&out)?;
        // A link says which repository it is about, and only its number was asked for. One to
        // another repository's issue would otherwise be answered with this project's issue of
        // the same number, which looks exactly like a right answer.
        if let (Some(asked), Some(answered)) = (repository(key), repository(&detail.task.url)) {
            if asked != answered {
                return Err(ForgeError::Failed {
                    command: "issue".to_owned(),
                    stderr: format!(
                        "that link is to an issue of {asked}, and this project's tasks are \
                         {answered}'s"
                    ),
                });
            }
        }
        Ok(detail)
    }
}

/// The repository an issue's URL is in, as `host/owner/name` in lower case. `None` for
/// anything that is not such a URL — a bare number most of all.
pub(crate) fn repository(url: &str) -> Option<String> {
    let (before, _) = url.trim().rsplit_once("/issues/")?;
    let (_, rest) = before.split_once("://")?;
    let rest = rest.trim_start_matches("www.");
    (rest.split('/').count() == 3).then(|| rest.to_ascii_lowercase())
}

/// The number in what a person typed for an issue: `91`, `#91`, or its URL.
pub fn number(key: &str) -> Option<u32> {
    let key = key.trim();
    let digits = match key.rsplit_once("/issues/") {
        Some((_, rest)) => rest.split(['#', '?', '/']).next().unwrap_or(rest),
        None => key.strip_prefix('#').unwrap_or(key),
    };
    digits.parse().ok().filter(|number| *number > 0)
}

/// One page of [`list_query`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Page {
    tasks: Vec<Task>,
    total: Option<u32>,
    /// The cursor to ask for the next page with. `None` on the last one.
    next: Option<String>,
    viewer: Option<String>,
    disabled: bool,
}

fn unreadable(what: &str) -> ForgeError {
    ForgeError::Unreadable(what.to_owned())
}

fn nodes(value: Option<&Value>) -> &[Value] {
    value
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
}

fn login(author: Option<&Value>) -> Option<String> {
    author
        .and_then(|author| author.get("login"))
        .and_then(|login| login.as_str())
        .filter(|login| !login.is_empty())
        .map(str::to_owned)
}

/// Who wrote an issue or a comment, as far as [`needs_answer`] cares.
fn voice(node: &Value) -> Voice {
    let kind = node
        .get("author")
        .and_then(|author| author.get("__typename"))
        .and_then(|kind| kind.as_str());
    let association = node.get("authorAssociation").and_then(|a| a.as_str());
    Voice {
        // No author at all is an account that was deleted.
        person: kind.is_some_and(|kind| kind != "Bot"),
        maintainer: matches!(association, Some("OWNER" | "MEMBER" | "COLLABORATOR")),
    }
}

/// Read one issue as a task. `None` for a node without the few things a row cannot do without.
fn task(node: &Value) -> Option<Task> {
    let number = count(node.get("number"))?;
    let url = node.get("url")?.as_str()?.to_owned();
    let open = node.get("state").and_then(|s| s.as_str()) != Some("CLOSED");
    let conversation = node.get("comments");
    // The end of the conversation, and no more of it than a list is given: one issue in full
    // comes with a hundred comments, and deciding from those would let `ys task show` say
    // something `ys task list` does not about the same issue.
    let comments = nodes(conversation);
    let latest: Vec<Voice> = comments[comments.len().saturating_sub(LATEST as usize)..]
        .iter()
        .map(voice)
        .collect();
    Some(Task {
        source: TaskSourceKind::GitHub,
        key: format!("#{number}"),
        url,
        title: text(node, "title"),
        state: if open {
            TaskState::Open
        } else {
            TaskState::Closed
        },
        closed_as: match node.get("stateReason").and_then(|r| r.as_str()) {
            _ if open => None,
            Some("COMPLETED") => Some(ClosedAs::Completed),
            Some("NOT_PLANNED") => Some(ClosedAs::NotPlanned),
            Some("DUPLICATE") => Some(ClosedAs::Duplicate),
            _ => None,
        },
        author: login(node.get("author")),
        labels: nodes(node.get("labels"))
            .iter()
            .map(|label| TaskLabel {
                name: text(label, "name"),
                color: text(label, "color"),
            })
            .collect(),
        assignees: nodes(node.get("assignees"))
            .iter()
            .filter_map(|assignee| login(Some(assignee)))
            .collect(),
        comments: count(conversation.and_then(|c| c.get("totalCount"))).unwrap_or(0),
        created_at: text(node, "createdAt"),
        updated_at: text(node, "updatedAt"),
        needs_answer: needs_answer(open, voice(node), &latest),
        linked_pull_requests: nodes(node.get("closedByPullRequestsReferences"))
            .iter()
            .filter_map(|pr| count(pr.get("number")))
            .collect(),
    })
}

fn data(json: &str) -> ForgeResult<Value> {
    let mut parsed: Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    parsed
        .get_mut("data")
        .map(Value::take)
        .ok_or_else(|| unreadable("expected data"))
}

/// Read one answer to [`list_query`].
fn page(json: &str) -> ForgeResult<Page> {
    let data = data(json)?;
    let repository = data
        .get("repository")
        .filter(|repository| !repository.is_null())
        .ok_or_else(|| unreadable("expected a repository"))?;
    let list = repository
        .get("issues")
        .filter(|list| !list.is_null())
        .ok_or_else(|| unreadable("expected a repository's issues"))?;
    let info = list.get("pageInfo");
    let more = info
        .and_then(|info| info.get("hasNextPage"))
        .and_then(|more| more.as_bool())
        .unwrap_or(false);
    Ok(Page {
        tasks: nodes(Some(list)).iter().filter_map(task).collect(),
        total: count(list.get("totalCount")),
        next: info
            .and_then(|info| info.get("endCursor"))
            .and_then(|cursor| cursor.as_str())
            .filter(|_| more)
            .map(str::to_owned),
        viewer: login(data.get("viewer")),
        disabled: repository.get("hasIssuesEnabled").and_then(|v| v.as_bool()) == Some(false),
    })
}

/// Read one answer to [`detail_query`].
fn detail(json: &str) -> ForgeResult<TaskDetail> {
    let data = data(json)?;
    let issue = data
        .get("repository")
        .and_then(|repository| repository.get("issue"))
        .filter(|issue| !issue.is_null())
        .ok_or_else(|| unreadable("expected an issue"))?;
    let comments = nodes(issue.get("comments"))
        .iter()
        .map(|comment| {
            let hidden = comment
                .get("isMinimized")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                .then(|| text(comment, "minimizedReason").to_ascii_lowercase());
            let voice = voice(comment);
            TaskComment {
                author: login(comment.get("author")),
                created_at: text(comment, "createdAt"),
                // What the forge hid is not shown by another door.
                body: match hidden {
                    Some(_) => String::new(),
                    None => text(comment, "body"),
                },
                url: comment
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                hidden,
                bot: comment.get("author").is_some_and(|a| !a.is_null()) && !voice.person,
                maintainer: voice.maintainer,
            }
        })
        .collect();
    Ok(TaskDetail {
        task: task(issue).ok_or_else(|| unreadable("expected an issue"))?,
        body: text(issue, "body"),
        comments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/gh/2.102.0")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn a_number_is_read_from_what_a_person_would_type() {
        assert_eq!(number("91"), Some(91));
        assert_eq!(number(" #91 "), Some(91));
        assert_eq!(number("https://github.com/o/r/issues/91"), Some(91));
        assert_eq!(
            number("https://github.com/o/r/issues/91#issuecomment-5"),
            Some(91)
        );
        assert_eq!(number("0"), None);
        assert_eq!(number("ninety-one"), None);
        assert_eq!(number("https://github.com/o/r/pull/91"), None);
    }

    #[test]
    fn which_projects_are_covered_follows_the_remote() {
        let repo = |host: &str, kind| Repo {
            host: host.to_owned(),
            owner: "o".to_owned(),
            name: "r".to_owned(),
            kind,
        };
        assert_eq!(coverage(None), Coverage::NoRemote);
        assert_eq!(
            coverage(Some(&repo("github.com", ForgeKind::GitHub))),
            Coverage::Covered(None)
        );
        assert_eq!(
            coverage(Some(&repo("github.example.com", ForgeKind::GitHub))),
            Coverage::Covered(Some("github.example.com".to_owned()))
        );
        assert_eq!(
            coverage(Some(&repo("gitlab.com", ForgeKind::GitLab))),
            Coverage::OtherForge(ForgeKind::GitLab)
        );
        assert_eq!(
            coverage(Some(&repo("git.example.com", ForgeKind::Unknown))),
            Coverage::Covered(None)
        );
    }

    /// What `gh api graphql` really answered: see the README beside the fixtures.
    #[test]
    fn reads_a_recorded_page_of_open_issues() {
        let page = page(&fixture("issues-page-1.json")).unwrap();
        assert_eq!(page.total, Some(9));
        assert_eq!(page.viewer.as_deref(), Some("ada"));
        assert!(page.next.is_some(), "there is a page after it");
        assert!(!page.disabled);
        let keys: Vec<&str> = page.tasks.iter().map(|task| task.key.as_str()).collect();
        assert_eq!(
            keys,
            ["#14386", "#14584", "#14563", "#14467", "#13840", "#14558", "#13876"]
        );

        let find = |key: &str| page.tasks.iter().find(|task| task.key == key).unwrap();
        // An outsider's issue, eight comments: members answered, and another outsider spoke last.
        let asked_again = find("#14386");
        assert_eq!(asked_again.state, TaskState::Open);
        assert_eq!(asked_again.closed_as, None);
        assert_eq!(asked_again.comments, 8);
        assert!(asked_again.needs_answer);
        assert!(asked_again.url.starts_with("https://"));
        assert_eq!(asked_again.created_at, "2026-09-08T10:31:38Z");
        // An outsider's issue only a bot has commented on: still waiting.
        assert!(find("#14584").needs_answer);
        // A member's own issue with two bot comments: waiting on no one.
        let own = find("#14563");
        assert!(!own.needs_answer);
        assert!(!own.labels.is_empty());
        assert!(own.labels.iter().all(|label| label.color.len() == 6));
        // Bots, then an outsider.
        assert!(find("#14467").needs_answer);
        // Opened by a bot, 149 comments, the last five all bots.
        let bots = find("#13840");
        assert_eq!(bots.comments, 149);
        assert!(!bots.needs_answer);
        // Opened by an account that is gone; a member spoke last among the people.
        let ghost = find("#14558");
        assert_eq!(ghost.author, None);
        assert!(!ghost.needs_answer);

        // Assigned, and with an open pull request that will close it.
        let claimed = find("#13876");
        assert_eq!(claimed.assignees, ["donald"]);
        assert_eq!(claimed.linked_pull_requests, [13894]);
    }

    #[test]
    fn reads_the_last_page_and_a_page_of_closed_issues() {
        let last = page(&fixture("issues-page-2.json")).unwrap();
        assert_eq!(last.next, None);
        assert_eq!(last.tasks.len(), 2);

        let closed = page(&fixture("issues-closed.json")).unwrap();
        assert!(closed.tasks.iter().all(|t| t.state == TaskState::Closed));
        assert!(closed.tasks.iter().all(|t| !t.needs_answer));
        let reasons: Vec<Option<ClosedAs>> = closed.tasks.iter().map(|t| t.closed_as).collect();
        assert_eq!(
            reasons,
            [
                Some(ClosedAs::NotPlanned),
                Some(ClosedAs::Completed),
                Some(ClosedAs::Duplicate)
            ]
        );
    }

    #[test]
    fn a_repository_with_issues_switched_off_says_so() {
        let page = page(&fixture("issues-disabled.json")).unwrap();
        assert!(page.disabled);
        assert!(page.tasks.is_empty());
    }

    #[test]
    fn reads_a_recorded_issue_in_full() {
        let detail = detail(&fixture("issue-view.json")).unwrap();
        assert_eq!(detail.task.key, "#14394");
        assert!(!detail.body.is_empty());
        assert_eq!(detail.comments.len(), 5);
        assert_eq!(detail.task.comments, 5);
        let first = &detail.comments[0];
        assert!(first.bot, "a triage bot spoke first");
        assert!(!first.maintainer);
        assert!(detail.comments.iter().any(|c| c.maintainer && !c.bot));
        assert!(detail.comments.iter().all(|c| c.url.is_some()));
        assert!(detail.comments.iter().all(|c| c.hidden.is_none()));
        assert!(
            detail
                .comments
                .windows(2)
                .all(|pair| pair[0].created_at <= pair[1].created_at),
            "oldest first"
        );
        // The same rule as the list, from the same answer: an outsider spoke last. It was
        // reopened, which is a reason for being open and not one for being closed.
        assert!(detail.task.needs_answer);
        assert_eq!(detail.task.state, TaskState::Open);
        assert_eq!(detail.task.closed_as, None);
    }

    #[test]
    fn a_comment_the_forge_hid_stays_hidden_and_a_long_conversation_says_how_long() {
        let detail = detail(&fixture("issue-view-hidden.json")).unwrap();
        assert_eq!(detail.task.state, TaskState::Closed);
        assert_eq!(detail.comments.len(), 6, "the ones that were handed over");
        assert_eq!(detail.task.comments, 62, "how many there are");
        let hidden: Vec<&TaskComment> = detail
            .comments
            .iter()
            .filter(|comment| comment.hidden.is_some())
            .collect();
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].hidden.as_deref(), Some("spam"));
        assert_eq!(hidden[0].body, "", "what the forge hid stays hidden");
        assert!(detail.comments.iter().any(|c| !c.body.is_empty()));
    }

    /// An issue as either query answers it, with these comments: each is a kind of author and
    /// an association.
    fn issue_with(comments: &[(&str, &str)]) -> String {
        let nodes: Vec<String> = comments
            .iter()
            .map(|(kind, association)| {
                format!(
                    r#"{{"authorAssociation":"{association}","author":{{"__typename":"{kind}","login":"x"}},"body":"","createdAt":"2026-10-01T00:00:00Z","url":"u","isMinimized":false,"minimizedReason":null}}"#
                )
            })
            .collect();
        format!(
            r#"{{"number":7,"url":"https://github.com/o/r/issues/7","title":"t","state":"OPEN","stateReason":null,"createdAt":"","updatedAt":"","authorAssociation":"NONE","author":{{"__typename":"User","login":"grace"}},"labels":{{"nodes":[]}},"assignees":{{"nodes":[]}},"closedByPullRequestsReferences":{{"nodes":[]}},"body":"","comments":{{"totalCount":{},"nodes":[{}]}}}}"#,
            comments.len(),
            nodes.join(",")
        )
    }

    /// A list is given the last five comments and one issue in full a hundred. Both decide
    /// from the last five, or `ys task list` and `ys task show` would disagree.
    #[test]
    fn the_list_and_the_detail_agree_on_whether_an_answer_is_owed() {
        let bots = [("Bot", "NONE"); 5];
        // An outsider opened it, a maintainer answered, and five bots followed.
        let mut answered = vec![("User", "MEMBER")];
        answered.extend(bots);
        // An outsider's question, further back than five bots.
        let mut asked = vec![("User", "MEMBER"), ("User", "NONE")];
        asked.extend(bots);
        for comments in [answered, asked] {
            let whole = issue_with(&comments);
            let end = issue_with(&comments[comments.len() - 5..]);
            let in_full = detail(&format!(
                r#"{{"data":{{"repository":{{"issue":{whole}}}}}}}"#
            ))
            .unwrap();
            let in_list = page(&format!(
                r#"{{"data":{{"repository":{{"hasIssuesEnabled":true,"issues":{{"totalCount":1,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[{end}]}}}},"viewer":{{"login":"ada"}}}}}}"#
            ))
            .unwrap();
            assert_eq!(in_full.comments.len(), comments.len(), "all of it is shown");
            assert_eq!(
                in_full.task.needs_answer, in_list.tasks[0].needs_answer,
                "{comments:?}"
            );
            // Five bots are nobody, so it is the opener — an outsider — who is waiting.
            assert!(in_list.tasks[0].needs_answer);
        }
        // Within the last five, the last person decides in both.
        let recent = [("User", "NONE"), ("User", "MEMBER"), ("Bot", "NONE")];
        let in_full = detail(&format!(
            r#"{{"data":{{"repository":{{"issue":{}}}}}}}"#,
            issue_with(&recent)
        ))
        .unwrap();
        assert!(!in_full.task.needs_answer);
    }

    #[test]
    fn a_links_repository_is_read_and_a_number_has_none() {
        assert_eq!(
            repository("https://GitHub.com/Example/Widgets/issues/91#issuecomment-5").as_deref(),
            Some("github.com/example/widgets")
        );
        assert_eq!(
            repository("https://www.github.com/o/r/issues/1").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(repository("91"), None);
        assert_eq!(repository("#91"), None);
        assert_eq!(repository("https://github.com/o/r/pull/91"), None);
    }

    #[test]
    fn an_answer_that_is_not_one_is_an_error_and_not_an_empty_list() {
        assert!(page("{}").is_err());
        assert!(page(r#"{"data":{"repository":null}}"#).is_err());
        assert!(detail(r#"{"data":{"repository":{"issue":null}}}"#).is_err());
        assert!(page("not json").is_err());
    }

    /// Against the real `gh`, the real network and this repository — so the queries above are
    /// checked against what GitHub actually accepts and returns rather than against fixtures
    /// that were right once. Ignored by default: CI has no network and no login.
    ///
    /// `cargo test -p yardsort-core -- --ignored --nocapture reads_this_repositorys`
    #[test]
    #[ignore = "needs gh, a login and the network"]
    fn reads_this_repositorys_own_issues() {
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::find(&env).expect("gh on PATH");
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = GitHub::new(&gh, None);
        for state in [TaskState::Open, TaskState::Closed] {
            let found = source.list(here, state, 5);
            assert_eq!(found.problem, None);
            assert!(found.answered && !found.disabled);
            assert!(found.viewer.is_some() && found.total.is_some());
            for task in &found.tasks {
                println!(
                    "{} {:?} needs answer: {}",
                    task.key, state, task.needs_answer
                );
                assert_eq!(task.state, state);
                assert!(task.url.starts_with("https://"));
                let detail = source.show(here, &task.key).expect("gh answered");
                assert_eq!(detail.task.key, task.key);
                assert_eq!(detail.task.needs_answer, task.needs_answer);
            }
        }
        let missing = source.show(here, "999999").unwrap_err();
        assert!(
            missing.to_string().contains("Could not resolve"),
            "{missing}"
        );
    }

    #[cfg(unix)]
    fn stand_in(dir: &Path, script: &str) -> Gh {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("gh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        Gh::at(path, &env)
    }

    /// A `gh` that answers the first question with page one and the one after its cursor with
    /// `second`, writing down everything it was asked.
    #[cfg(unix)]
    fn paged(dir: &Path, second: &str) -> Gh {
        let here = dir.display();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0");
        let first = fixtures.join("issues-page-1.json");
        stand_in(
            dir,
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncase \"$*\" in\n\
                 *after=*) {second} ;;\n\
                 *) cat '{}' ;;\nesac\n",
                first.display()
            ),
        )
    }

    #[cfg(unix)]
    fn asked(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("asked"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn pages_are_read_in_order_with_the_cursor_of_the_one_before() {
        let dir = tempfile::tempdir().unwrap();
        let second =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0/issues-page-2.json");
        let gh = paged(dir.path(), &format!("cat '{}'", second.display()));
        let found = GitHub::new(&gh, None).list(dir.path(), TaskState::Open, 200);
        assert!(found.answered);
        assert_eq!(found.problem, None);
        assert_eq!(found.total, Some(9));
        assert_eq!(found.viewer.as_deref(), Some("ada"));
        assert_eq!(found.tasks.len(), 9);

        let asked = asked(dir.path());
        assert_eq!(asked.len(), 2);
        assert!(asked[0].starts_with("api graphql -F owner={owner} -F name={repo} -f query="));
        assert!(asked[0].contains("states:OPEN"), "{}", asked[0]);
        assert!(!asked[0].contains("after="), "the first page has no cursor");
        assert!(
            asked[1].ends_with("-f after=Y3Vyc29yOnYyOpK0"),
            "{}",
            asked[1]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_fails_keeps_the_ones_before_it_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged(dir.path(), "echo 'HTTP 502: Bad Gateway' >&2; exit 1");
        let found = GitHub::new(&gh, None).list(dir.path(), TaskState::Open, 200);
        assert!(found.answered);
        assert_eq!(found.tasks.len(), 7, "the first page is kept");
        let problem = found.problem.unwrap();
        assert!(problem.contains("502"), "{problem}");
        assert!(
            !problem.contains("query("),
            "not the whole query: {problem}"
        );
        assert!(!found.logged_out);
    }

    #[cfg(unix)]
    #[test]
    fn reading_stops_at_the_cap_and_still_reports_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged(dir.path(), "exit 1");
        let found = GitHub::new(&gh, None).list(dir.path(), TaskState::Open, 4);
        assert_eq!(found.tasks.len(), 4);
        assert_eq!(found.total, Some(9));
        assert_eq!(found.problem, None);
        assert_eq!(
            asked(dir.path()).len(),
            1,
            "no page past the cap is asked for"
        );
    }

    #[cfg(unix)]
    #[test]
    fn closed_issues_and_another_host_are_asked_for_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged(dir.path(), "exit 1");
        let source = GitHub::new(&gh, Some("github.example.com".to_owned()));
        source.list(dir.path(), TaskState::Closed, 7);
        let asked = asked(dir.path());
        assert!(
            asked[0].starts_with("api graphql --hostname github.example.com -F owner="),
            "{}",
            asked[0]
        );
        assert!(asked[0].contains("states:CLOSED"), "{}", asked[0]);
    }

    #[cfg(unix)]
    #[test]
    fn nobody_logged_in_is_said_as_that() {
        let dir = tempfile::tempdir().unwrap();
        let gh = stand_in(
            dir.path(),
            "#!/bin/sh\necho 'To get started with GitHub CLI, please run: gh auth login' >&2\nexit 4\n",
        );
        let found = GitHub::new(&gh, None).list(dir.path(), TaskState::Open, 200);
        assert!(!found.answered);
        assert!(found.logged_out);
        assert!(found.tasks.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn one_issue_is_asked_for_by_its_number_as_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display();
        let view =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0/issue-view.json");
        let gh = stand_in(
            dir.path(),
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncat '{}'\n",
                view.display()
            ),
        );
        let source = GitHub::new(&gh, None);
        let detail = source.show(dir.path(), "#14394").unwrap();
        assert_eq!(detail.task.key, "#14394");
        let first = asked(dir.path());
        assert!(first[0].ends_with("-F number=14394"), "{}", first[0]);
        assert!(first[0].contains("issue(number:$number)"), "{}", first[0]);

        // Its own link is as good as its number; another repository's is refused, by name,
        // rather than answered with this project's issue of that number.
        let linked = "https://github.com/Example/widgets/issues/14394";
        assert_eq!(source.show(dir.path(), linked).unwrap().task.key, "#14394");
        let elsewhere = source
            .show(dir.path(), "https://github.com/someone/else/issues/14394")
            .unwrap_err()
            .to_string();
        assert!(elsewhere.contains("github.com/someone/else"), "{elsewhere}");
        assert!(
            elsewhere.contains("github.com/example/widgets"),
            "{elsewhere}"
        );

        let refused = source.show(dir.path(), "soon").unwrap_err();
        assert!(refused.to_string().contains("not an issue's number"));
        assert_eq!(asked(dir.path()).len(), 3, "nothing was asked for that");
    }
}

//! One pull request as a trigger watches it: its state, its head and the checks on it, whether
//! it conflicts, and every review and comment with who posted it. See
//! `docs/design/36-triggers.md`.
//!
//! One question of our own to GitHub, by number, so a pull request that was merged or closed is
//! still answered for. `gh pr view` has most of this but not the replies on review threads,
//! which only the comments API has, and neither says whose account `gh` is: three calls where
//! one does.
//!
//! **A reply on a review thread is a comment, not a review.** GitHub records each reply as a
//! review of its own as well — submitted, `COMMENTED`, with no body and one comment, the reply.
//! Seen against a real pull request on 2026-11-07. So a review with no words of its own whose
//! every comment answers another is read as one [`WatchPostKind::Reply`] per reply, and is not a
//! review. A review that says something, or comments on a line afresh, is a review, and replies
//! made in it are part of it.

use std::path::Path;
use std::time::Duration;

use super::{items, string_field, CheckCounts, ForgeError, ForgeResult, Gh, Mergeable};
use super::{PullRequestState, ReviewState};

/// A pull request as one look saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrLook {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PullRequestState,
    pub draft: bool,
    /// The branch it would merge.
    pub branch: String,
    /// Its newest commit.
    pub head: String,
    /// What the checks on `head` came to: every one of them, counted by GitHub by state, however
    /// many there are.
    pub check_counts: CheckCounts,
    /// The name of each check on `head` that failed, of the first hundred.
    pub failed_checks: Vec<String>,
    /// More checks failed than are named in `failed_checks`.
    pub more_failed: bool,
    pub mergeable: Mergeable,
    /// Every review and comment, oldest first.
    pub posts: Vec<WatchPost>,
    /// The account `gh` asks as: the user's, and every agent's.
    pub viewer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchPostKind {
    /// A submitted review, with any comments on lines in it.
    Review,
    /// A comment on the conversation.
    Comment,
    /// A reply on a review thread.
    Reply,
}

/// A review or a comment, as a watch counts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchPost {
    /// The forge's id, with what kind it is in front, so ids of two kinds cannot meet:
    /// `review:5810872125`.
    pub id: String,
    pub kind: WatchPostKind,
    /// By login, a bot's ending in `[bot]` as GitHub's other answers write it. `None` for an
    /// account that no longer exists.
    pub author: Option<String>,
    /// What a review concluded. `None` for a comment or a reply.
    pub state: Option<ReviewState>,
    /// In epoch milliseconds.
    pub at: i64,
    pub url: Option<String>,
}

/// Everything a watch needs about pull request `$number`, and who is asking.
const WATCH_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){\
repository(owner:$owner,name:$name){pullRequest(number:$number){\
number url title state isDraft headRefName headRefOid mergeable \
commits(last:1){nodes{commit{statusCheckRollup{contexts(first:100){\
checkRunCountsByState{state count} statusContextCountsByState{state count} nodes{__typename \
...on CheckRun{name status conclusion} ...on StatusContext{context state}}}}}}} \
reviews(last:100){nodes{databaseId state submittedAt body url author{__typename login} \
comments(first:100){totalCount nodes{databaseId url replyTo{databaseId}}}}} \
comments(last:100){nodes{databaseId createdAt url author{__typename login}}}}} \
viewer{login}}";

impl Gh {
    /// Pull request `number` of the repository at `root`, as a watch sees it. `host` as for
    /// [`Gh::open_page`]; `limit` is how long `gh` may take.
    pub fn pr_look(
        &self,
        root: &Path,
        host: Option<&str>,
        number: u32,
        limit: Duration,
    ) -> ForgeResult<PrLook> {
        let number = format!("number={number}");
        let out = self.graphql(root, host, WATCH_QUERY, &["-F", &number], limit)?;
        pr_look(&out)
    }

    /// Pull request `number` of `repo`, wherever that is, as a watch sees it: one a workspace
    /// tracks in another repository. `root` is only where `gh` runs.
    pub fn pr_look_in(
        &self,
        root: &Path,
        repo: &super::Repo,
        number: u32,
        limit: Duration,
    ) -> ForgeResult<PrLook> {
        let number = format!("number={number}");
        // `gh api` asks github.com unless told otherwise.
        let host = Some(repo.host.as_str()).filter(|host| *host != "github.com");
        let out = self.graphql_in(
            root,
            host,
            Some((&repo.owner, &repo.name)),
            WATCH_QUERY,
            &["-F", &number],
            limit,
        )?;
        pr_look(&out)
    }
}

/// Read what GitHub answered to [`WATCH_QUERY`].
pub fn pr_look(json: &str) -> ForgeResult<PrLook> {
    let unreadable = |what: &str| ForgeError::Unreadable(what.to_owned());
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let data = parsed
        .get("data")
        .ok_or_else(|| unreadable("expected data"))?;
    let pr = data
        .get("repository")
        .and_then(|repository| repository.get("pullRequest"))
        .filter(|pr| !pr.is_null())
        .ok_or_else(|| unreadable("expected a pull request"))?;
    let nodes = |value: Option<&serde_json::Value>| -> Vec<serde_json::Value> {
        items(value.and_then(|v| v.get("nodes"))).to_vec()
    };
    let at = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|t| t.as_str())
            .and_then(crate::activity::iso_to_ms)
    };
    let id = |value: &serde_json::Value| value.get("databaseId").and_then(|v| v.as_u64());
    let url =
        |value: &serde_json::Value| value.get("url").and_then(|v| v.as_str()).map(str::to_owned);

    // The verdict is GitHub's own count of every check by state, as the list of open pull
    // requests reads it: the nodes are one page, and a green first page says nothing of the
    // second. The nodes are only for the names of the ones that failed.
    let commits = nodes(pr.get("commits"));
    let contexts = commits
        .first()
        .and_then(|node| node.get("commit"))
        .and_then(|commit| commit.get("statusCheckRollup"))
        .and_then(|rollup| rollup.get("contexts"));
    let mut check_counts = CheckCounts::default();
    for key in ["checkRunCountsByState", "statusContextCountsByState"] {
        for by_state in items(contexts.and_then(|contexts| contexts.get(key))) {
            let state = by_state.get("state").and_then(|s| s.as_str());
            let count = by_state
                .get("count")
                .and_then(|c| c.as_u64())
                .and_then(|c| u32::try_from(c).ok());
            if let (Some(state), Some(count)) = (state, count) {
                check_counts.add(state, true, count);
            }
        }
    }
    let mut named = CheckCounts::default();
    let mut failed_checks = Vec::new();
    // A union's connection can hold nulls; they are no check at all.
    for check in nodes(contexts).iter().filter(|check| !check.is_null()) {
        let text = |key: &str| check.get(key).and_then(|v| v.as_str());
        // A check run has a status and, once it has finished, a conclusion; a commit status has
        // only a state.
        let (name, verdict, finished) = match text("__typename") {
            Some("StatusContext") => (text("context"), text("state").unwrap_or(""), true),
            _ => (
                text("name"),
                text("conclusion").unwrap_or(""),
                text("status").is_none_or(|status| status.eq_ignore_ascii_case("COMPLETED")),
            ),
        };
        let failed_before = named.failed;
        named.add(verdict, finished, 1);
        if named.failed > failed_before {
            failed_checks.push(name.unwrap_or("Check").to_owned());
        }
    }
    let more_failed = check_counts.failed as usize > failed_checks.len();

    let mut posts = Vec::new();
    for review in nodes(pr.get("reviews")) {
        // One still being written has no time: it has not been posted.
        let (Some(review_id), Some(when)) = (id(&review), at(&review, "submittedAt")) else {
            continue;
        };
        let state = match review.get("state").and_then(|s| s.as_str()) {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            Some("DISMISSED") => ReviewState::Dismissed,
            _ => continue,
        };
        let comments = nodes(review.get("comments"));
        let replies: Vec<&serde_json::Value> = comments
            .iter()
            .filter(|comment| comment.get("replyTo").is_some_and(|to| !to.is_null()))
            .collect();
        let only_replies = state == ReviewState::Commented
            && string_field(&review, "body").trim().is_empty()
            && !comments.is_empty()
            && replies.len() == comments.len()
            // Every comment in it was read: a review of more than a hundred is not only replies
            // as far as anyone can tell.
            && review
                .get("comments")
                .and_then(|c| c.get("totalCount"))
                .and_then(|n| n.as_u64())
                .is_none_or(|total| total as usize == comments.len());
        if only_replies {
            for reply in replies {
                let Some(reply_id) = id(reply) else {
                    continue;
                };
                posts.push(WatchPost {
                    id: format!("reply:{reply_id}"),
                    kind: WatchPostKind::Reply,
                    author: author(&review),
                    state: None,
                    at: when,
                    url: url(reply),
                });
            }
            continue;
        }
        posts.push(WatchPost {
            id: format!("review:{review_id}"),
            kind: WatchPostKind::Review,
            author: author(&review),
            state: Some(state),
            at: when,
            url: url(&review),
        });
    }
    for comment in nodes(pr.get("comments")) {
        let (Some(comment_id), Some(when)) = (id(&comment), at(&comment, "createdAt")) else {
            continue;
        };
        posts.push(WatchPost {
            id: format!("comment:{comment_id}"),
            kind: WatchPostKind::Comment,
            author: author(&comment),
            state: None,
            at: when,
            url: url(&comment),
        });
    }
    posts.sort_by_key(|post| post.at);

    Ok(PrLook {
        number: pr
            .get("number")
            .and_then(|n| n.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| unreadable("expected a pull request's number"))?,
        url: string_field(pr, "url"),
        title: string_field(pr, "title"),
        state: match pr.get("state").and_then(|s| s.as_str()) {
            Some("MERGED") => PullRequestState::Merged,
            Some("CLOSED") => PullRequestState::Closed,
            _ => PullRequestState::Open,
        },
        draft: pr.get("isDraft").and_then(|d| d.as_bool()).unwrap_or(false),
        branch: string_field(pr, "headRefName"),
        head: string_field(pr, "headRefOid"),
        check_counts,
        failed_checks,
        more_failed,
        mergeable: match pr.get("mergeable").and_then(|m| m.as_str()) {
            Some("MERGEABLE") => Mergeable::Mergeable,
            Some("CONFLICTING") => Mergeable::Conflicting,
            _ => Mergeable::Unknown,
        },
        posts,
        viewer: data
            .get("viewer")
            .and_then(|viewer| viewer.get("login"))
            .and_then(|login| login.as_str())
            .map(str::to_owned),
    })
}

/// Who posted it. GitHub's GraphQL names a bot without the `[bot]` its REST answers and its
/// pages put after it; it is put back, so a login reads the same whichever answer it came from.
fn author(post: &serde_json::Value) -> Option<String> {
    let author = post.get("author").filter(|a| !a.is_null())?;
    let login = author
        .get("login")
        .and_then(|l| l.as_str())
        .filter(|l| !l.is_empty())?;
    let bot = author.get("__typename").and_then(|t| t.as_str()) == Some("Bot");
    Some(if bot && !login.ends_with("[bot]") {
        format!("{login}[bot]")
    } else {
        login.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped as GitHub answered for a real pull request: a review with comments on lines, two
    /// replies on its threads (each a review of its own), a bot's review, a comment, a review
    /// still being written, and a check run of each kind.
    fn answer() -> serde_json::Value {
        serde_json::json!({"data": {"repository": {"pullRequest": {
            "number": 142, "url": "https://github.com/acme/tool/pull/142", "title": "Fix login",
            "state": "OPEN", "isDraft": false, "headRefName": "fix-login",
            "headRefOid": "abc123", "mergeable": "CONFLICTING",
            "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {
              "checkRunCountsByState": [{"state": "FAILURE", "count": 1},
                {"state": "SUCCESS", "count": 1}, {"state": "IN_PROGRESS", "count": 1}],
              "statusContextCountsByState": [{"state": "ERROR", "count": 1}],
              "nodes": [
                null,
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED",
                 "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "test (ubuntu)", "status": "COMPLETED",
                 "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "name": "test (macos)", "status": "IN_PROGRESS",
                 "conclusion": null},
                {"__typename": "StatusContext", "context": "ci/legacy", "state": "ERROR"}
            ]}}}}]},
            "reviews": {"nodes": [
                {"databaseId": 1, "state": "CHANGES_REQUESTED",
                 "submittedAt": "2026-11-07T07:51:50Z", "body": "A few things.",
                 "url": "https://github.com/acme/tool/pull/142#pullrequestreview-1",
                 "author": {"__typename": "User", "login": "alice"},
                 "comments": {"totalCount": 2, "nodes": [
                    {"databaseId": 11, "url": "u11", "replyTo": null},
                    {"databaseId": 12, "url": "u12", "replyTo": null}]}},
                {"databaseId": 2, "state": "COMMENTED", "submittedAt": "2026-11-07T07:56:26Z",
                 "body": "", "url": "r2", "author": {"__typename": "User", "login": "me"},
                 "comments": {"totalCount": 1, "nodes": [
                    {"databaseId": 21, "url": "u21", "replyTo": {"databaseId": 11}}]}},
                {"databaseId": 3, "state": "COMMENTED", "submittedAt": "2026-11-07T07:58:00Z",
                 "body": "", "url": "r3", "author": {"__typename": "User", "login": "alice"},
                 "comments": {"totalCount": 1, "nodes": [
                    {"databaseId": 31, "url": "u31", "replyTo": {"databaseId": 21}}]}},
                {"databaseId": 4, "state": "COMMENTED", "submittedAt": "2026-11-07T08:00:00Z",
                 "body": "Looks fine.", "url": "r4",
                 "author": {"__typename": "Bot", "login": "coderabbitai"},
                 "comments": {"totalCount": 0, "nodes": []}},
                {"databaseId": 5, "state": "PENDING", "submittedAt": null, "body": "",
                 "url": "r5", "author": {"__typename": "User", "login": "me"},
                 "comments": {"totalCount": 1, "nodes": [
                    {"databaseId": 51, "url": "u51", "replyTo": null}]}}
            ]},
            "comments": {"nodes": [
                {"databaseId": 9, "createdAt": "2026-11-07T07:55:00Z", "url": "c9",
                 "author": {"__typename": "User", "login": "bob"}},
                {"databaseId": 10, "createdAt": "2026-11-07T07:59:00Z", "url": "c10",
                 "author": null}
            ]}
        }}, "viewer": {"login": "me"}}})
    }

    /// One elsewhere is asked about by its own repository and host, from a folder that is no
    /// repository at all: nothing is left for `gh` to fill in from the folder.
    #[cfg(unix)]
    #[test]
    fn a_pull_request_elsewhere_is_asked_about_by_its_repository_on_its_host() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display().to_string();
        std::fs::write(dir.path().join("answer.json"), answer().to_string()).unwrap();
        let script = dir.path().join("gh");
        crate::program::testing::executable(
            &script,
            format!("#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncat '{here}/answer.json'\n"),
        );
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::at(script, &env);
        let limit = Duration::from_secs(10);
        let on = |host: &str| crate::tracked::repo_from_key(&format!("{host}/acme/lib")).unwrap();
        let look = gh
            .pr_look_in(dir.path(), &on("github.com"), 142, limit)
            .unwrap();
        assert_eq!(look.number, 142);
        gh.pr_look_in(dir.path(), &on("ghe.example.com"), 7, limit)
            .unwrap();
        let asked = std::fs::read_to_string(format!("{here}/asked")).unwrap();
        let asked: Vec<&str> = asked.lines().collect();
        assert!(
            asked[0].starts_with("api graphql -F owner=acme -F name=lib -f query="),
            "{}",
            asked[0]
        );
        assert!(asked[0].ends_with("-F number=142"), "{}", asked[0]);
        assert!(!asked[0].contains("--hostname"));
        assert!(
            asked[1].starts_with(
                "api graphql --hostname ghe.example.com -F owner=acme -F name=lib -f query="
            ),
            "{}",
            asked[1]
        );
        assert!(asked[1].ends_with("-F number=7"));
    }

    #[test]
    fn a_reply_on_a_thread_is_a_comment_and_a_bot_is_named_as_a_bot() {
        let look = pr_look(&answer().to_string()).unwrap();
        let posts: Vec<(&str, WatchPostKind, Option<&str>)> = look
            .posts
            .iter()
            .map(|p| (p.id.as_str(), p.kind, p.author.as_deref()))
            .collect();
        assert_eq!(
            posts,
            [
                ("review:1", WatchPostKind::Review, Some("alice")),
                ("comment:9", WatchPostKind::Comment, Some("bob")),
                ("reply:21", WatchPostKind::Reply, Some("me")),
                ("reply:31", WatchPostKind::Reply, Some("alice")),
                ("comment:10", WatchPostKind::Comment, None),
                ("review:4", WatchPostKind::Review, Some("coderabbitai[bot]")),
            ]
        );
        assert_eq!(look.posts[0].state, Some(ReviewState::ChangesRequested));
        assert_eq!(look.posts[2].url.as_deref(), Some("u21"));
        assert_eq!(look.viewer.as_deref(), Some("me"));
    }

    #[test]
    fn checks_are_counted_with_the_names_of_those_that_failed() {
        let look = pr_look(&answer().to_string()).unwrap();
        assert_eq!(
            look.check_counts,
            CheckCounts {
                passed: 1,
                failed: 2,
                running: 1
            }
        );
        assert_eq!(look.failed_checks, ["lint", "ci/legacy"]);
        assert!(!look.more_failed);
        assert_eq!(look.mergeable, Mergeable::Conflicting);
        assert_eq!(
            (look.number, look.head.as_str(), look.branch.as_str()),
            (142, "abc123", "fix-login")
        );
        assert_eq!(look.state, PullRequestState::Open);
    }

    #[test]
    fn the_verdict_is_every_check_counted_not_the_first_page() {
        let mut answer = answer();
        let contexts = &mut answer["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]
            ["commit"]["statusCheckRollup"]["contexts"];
        // A hundred and fifty checks, one of them on a page not read failed, and one stale.
        contexts["checkRunCountsByState"] = serde_json::json!([
            {"state": "SUCCESS", "count": 147}, {"state": "FAILURE", "count": 2},
            {"state": "STALE", "count": 1}]);
        contexts["statusContextCountsByState"] = serde_json::json!([]);
        let look = pr_look(&answer.to_string()).unwrap();
        assert_eq!(
            look.check_counts,
            CheckCounts {
                passed: 148,
                failed: 2,
                running: 0
            },
            "a stale check has finished"
        );
        // The page read names the two that failed, so none is left unnamed.
        assert_eq!(look.failed_checks, ["lint", "ci/legacy"]);
        assert!(!look.more_failed);
        contexts_failed(&mut answer, 5);
        assert!(pr_look(&answer.to_string()).unwrap().more_failed);
    }

    fn contexts_failed(answer: &mut serde_json::Value, failed: u32) {
        answer["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]
            ["statusCheckRollup"]["contexts"]["checkRunCountsByState"] =
            serde_json::json!([{"state": "FAILURE", "count": failed}]);
    }

    #[test]
    fn a_review_that_says_something_is_a_review_even_with_replies_in_it() {
        let mut answer = answer();
        answer["data"]["repository"]["pullRequest"]["reviews"]["nodes"][1]["body"] =
            "And one more thing.".into();
        let look = pr_look(&answer.to_string()).unwrap();
        assert!(look.posts.iter().any(|p| p.id == "review:2"));
        assert!(!look.posts.iter().any(|p| p.id == "reply:21"));
    }

    #[test]
    fn a_pull_request_that_is_not_there_is_unreadable() {
        let answer = serde_json::json!({"data": {"repository": {"pullRequest": null}}});
        assert!(matches!(
            pr_look(&answer.to_string()),
            Err(ForgeError::Unreadable(_))
        ));
    }
}

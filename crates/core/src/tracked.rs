//! Pull requests a workspace follows in repositories other than its project's.
//!
//! A project's own pull requests are found by asking its repository and matching branches and
//! commits to workspaces (`crate::forge::pull_requests_from`). A project whose checkout holds
//! other repositories — nested clones its agents go into and open pull requests from — has pull
//! requests that leave no such trace, so they are written down: by an agent through
//! `ys pr track`, by the user in the app, or (later) by an adapter that saw one being created.
//! See `docs/design/36-pull-requests-elsewhere.md`.

use std::path::Path;

use crate::error::{IpcError, IpcResult};
use crate::forge::{ForgeKind, Repo};
use crate::git::Git;
use crate::store::{NewTrackedPullRequest, Store, TrackedPullRequestRow};

/// How many of a project checkout's nested repositories the launch section names.
const NAMED_REPOSITORIES: usize = 10;

/// Who wrote a tracked pull request down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackedBy {
    /// **Track pull request…** in the app.
    App,
    /// `ys pr track`.
    Cli,
    /// An adapter that saw the pull request being created.
    Hook,
}

impl TrackedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Cli => "cli",
            Self::Hook => "hook",
        }
    }
}

/// A link to a pull request on GitHub, checked and rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestLink {
    pub repo: Repo,
    pub number: u32,
    /// `https://host/owner/name/pull/number`, whatever was given after the number dropped.
    pub url: String,
}

impl PullRequestLink {
    /// `host/owner/name`: what the table keeps.
    pub fn repo_key(&self) -> String {
        repo_key(&self.repo)
    }
}

/// `host/owner/name`.
pub fn repo_key(repo: &Repo) -> String {
    format!("{}/{}/{}", repo.host, repo.owner, repo.name)
}

/// The repository a `host/owner/name` names, read back. `None` for anything else.
pub fn repo_from_key(key: &str) -> Option<Repo> {
    let (host, path) = key.split_once('/')?;
    let (owner, name) = path.rsplit_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    crate::forge::parse_remote(&format!("https://{host}/{owner}/{name}.git"))
}

/// Read a link to a pull request. Only GitHub's shape, `/<owner>/<name>/pull/<n>`, on any host
/// `gh` might speak to: everything past the link is `gh`, as for every other pull request.
pub fn parse(text: &str) -> IpcResult<PullRequestLink> {
    let refuse = |why: &str| IpcError::new("not_a_pull_request_link", why.to_owned());
    let text = text.trim();
    let link = crate::repositories::parse_link(text).ok_or_else(|| {
        refuse("That is not a link to a pull request, like https://github.com/owner/repo/pull/12.")
    })?;
    let path = text
        .split_once("://")
        .map_or("", |(_, rest)| rest.split(['#', '?']).next().unwrap_or(""));
    let github_shape = path
        .split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair[0] == "pull" && pair[1].parse::<u32>().is_ok_and(|n| n > 0));
    if link.kind != Some(crate::repositories::LinkKind::PullRequest) || !github_shape {
        return Err(refuse(
            "That is not a link to a pull request, like https://github.com/owner/repo/pull/12.",
        ));
    }
    if matches!(link.repo.kind, ForgeKind::GitLab | ForgeKind::Bitbucket)
        || link.repo.owner.contains('/')
    {
        return Err(refuse(
            "Only GitHub pull requests can be followed: Yardsort reads them through `gh`.",
        ));
    }
    let Repo {
        host, owner, name, ..
    } = &link.repo;
    let url = format!("https://{host}/{owner}/{name}/pull/{}", link.number);
    Ok(PullRequestLink {
        repo: link.repo.clone(),
        number: link.number,
        url,
    })
}

/// Whether two repositories are the same one: hosts and names compare without case, as GitHub
/// does.
fn same(a: &Repo, b: &Repo) -> bool {
    a.host.eq_ignore_ascii_case(&b.host)
        && a.owner.eq_ignore_ascii_case(&b.owner)
        && a.name.eq_ignore_ascii_case(&b.name)
}

/// Write down that `workspace_id` follows the pull request at `text`. Returns the row and
/// whether it is new. Refused for a link that is not a GitHub pull request, and for one of the
/// project's own repository — that one is followed already, by its branch.
pub fn track(
    store: &Store,
    git: &Git,
    workspace_id: &str,
    text: &str,
    session_id: Option<&str>,
    by: TrackedBy,
) -> IpcResult<(TrackedPullRequestRow, bool)> {
    let link = parse(text)?;
    let workspace = store
        .workspace(workspace_id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    let project = store
        .project(&workspace.project_id)?
        .ok_or_else(|| IpcError::new("unknown_project", "That project is no longer open."))?;
    let own = crate::forge::repos_at(git, Path::new(&project.root_path));
    if own.iter().any(|repo| same(repo, &link.repo)) {
        return Err(IpcError::new(
            "own_pull_request",
            format!(
                "{}/{} is this project's own repository, whose pull requests are followed \
                 already: a workspace shows the ones opened from its branch.",
                link.repo.owner, link.repo.name
            ),
        ));
    }
    // A session id that names no record — an environment left over from somewhere else — is
    // not worth failing over; the pull request is still this workspace's.
    let session_id = match session_id {
        Some(id) => store
            .session(id)?
            .filter(|session| session.workspace_id == workspace_id)
            .map(|session| session.id),
        None => None,
    };
    let repo = link.repo_key();
    Ok(store.track_pull_request(&NewTrackedPullRequest {
        workspace_id,
        session_id: session_id.as_deref(),
        repo: &repo,
        number: link.number,
        url: &link.url,
        tracked_by: by.as_str(),
    })?)
}

/// The git repositories directly inside `root`, by folder name, sorted: the nested clones a
/// parent repository keeps. A `.git` file counts as well as a folder — a worktree or a
/// submodule's checkout is one too.
pub fn nested_repositories(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.path().join(".git").exists())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    found.sort();
    found
}

/// The first-message section that tells a new agent the project's checkout holds other
/// repositories and how to have a pull request opened in one followed here. `None` when it holds
/// none, which is most projects.
pub fn launch_section(project_root: &Path) -> Option<String> {
    let nested = nested_repositories(project_root);
    if nested.is_empty() {
        return None;
    }
    let mut named = nested
        .iter()
        .take(NAMED_REPOSITORIES)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if nested.len() > NAMED_REPOSITORIES {
        named.push_str(&format!(" and {} more", nested.len() - NAMED_REPOSITORIES));
    }
    Some(format!(
        "## Other repositories\n\n\
         This project's checkout holds other git repositories: {named} (in {}). When you open \
         a pull request in one of them, run `ys pr track <its url>` so Yardsort follows it with \
         this workspace.",
        project_root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_pull_request_link_is_read_and_rebuilt() {
        let link = parse("https://github.com/o/lib/pull/7").unwrap();
        assert_eq!(link.url, "https://github.com/o/lib/pull/7");
        assert_eq!(link.number, 7);
        assert_eq!(link.repo_key(), "github.com/o/lib");
        for text in [
            "https://github.com/o/lib/pull/7/files",
            "https://github.com/o/lib/pull/7/",
            "  https://github.com/o/lib/pull/7#issuecomment-1  ",
            "https://github.com/o/lib/pull/7?w=1",
            "http://github.com/o/lib/pull/7",
        ] {
            assert_eq!(parse(text).unwrap().url, link.url, "{text}");
        }
        let enterprise = parse("https://github.example.com/team/svc/pull/3").unwrap();
        assert_eq!(enterprise.url, "https://github.example.com/team/svc/pull/3");
        assert_eq!(enterprise.repo.host, "github.example.com");
    }

    #[test]
    fn anything_else_is_refused_before_anything_is_written() {
        for text in [
            "",
            "o/lib#7",
            "https://github.com/o/lib/issues/7",
            "https://github.com/o/lib/commit/abc",
            "https://github.com/o/lib/pull/0",
            "https://github.com/o/lib",
            "https://gitlab.com/g/sub/lib/-/merge_requests/7",
            "https://bitbucket.org/o/lib/pull-requests/7",
            "ssh://github.com/o/lib/pull/7",
        ] {
            assert_eq!(
                parse(text).unwrap_err().code,
                "not_a_pull_request_link",
                "{text}"
            );
        }
    }

    #[test]
    fn a_repository_key_reads_back() {
        let repo = repo_from_key("github.com/o/lib").unwrap();
        assert_eq!(
            (repo.host.as_str(), repo.owner.as_str()),
            ("github.com", "o")
        );
        assert_eq!(repo.name, "lib");
        assert_eq!(repo.kind, ForgeKind::GitHub);
        assert!(repo_from_key("github.com/lib").is_none());
    }

    use crate::git::testing::git;

    fn project_on(remote: &str) -> (Store, crate::store::WorkspaceRow, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git().run(root, &["init", "-q"]).unwrap();
        git()
            .run(root, &["remote", "add", "origin", remote])
            .unwrap();
        let store = Store::in_memory();
        let project = store
            .add_project("parent", &root.to_string_lossy())
            .unwrap();
        let ws = store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap();
        (store, ws, dir)
    }

    #[test]
    fn a_pull_request_elsewhere_is_tracked_and_one_of_the_projects_own_is_not() {
        let (store, ws, _dir) = project_on("git@github.com:o/parent.git");
        let (row, added) = track(
            &store,
            &git(),
            &ws.id,
            "https://github.com/o/lib/pull/7/files",
            Some("no-such-session"),
            TrackedBy::Cli,
        )
        .unwrap();
        assert!(added);
        assert_eq!(row.url, "https://github.com/o/lib/pull/7");
        assert_eq!(row.repo, "github.com/o/lib");
        assert_eq!(row.tracked_by, "cli");
        assert_eq!(
            row.session_id, None,
            "a session that is not there is not kept"
        );

        let (_, added) = track(
            &store,
            &git(),
            &ws.id,
            "https://github.com/o/lib/pull/7",
            None,
            TrackedBy::App,
        )
        .unwrap();
        assert!(!added);

        let own = track(
            &store,
            &git(),
            &ws.id,
            "https://github.com/O/Parent/pull/3",
            None,
            TrackedBy::Cli,
        )
        .unwrap_err();
        assert_eq!(own.code, "own_pull_request");
        assert_eq!(store.tracked_pull_requests(&ws.id).unwrap().len(), 1);
    }

    #[test]
    fn nested_clones_are_named_in_the_launch_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(launch_section(root), None, "nothing nested, nothing said");
        std::fs::create_dir_all(root.join("lib/.git")).unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/.git"), "gitdir: elsewhere").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join(".hidden/.git")).unwrap();
        std::fs::write(root.join("file"), "").unwrap();
        assert_eq!(nested_repositories(root), ["app", "lib"]);
        let section = launch_section(root).unwrap();
        assert!(section.starts_with("## Other repositories\n\n"));
        assert!(section.contains("repositories: app, lib (in "));
        assert!(section.contains("`ys pr track <its url>`"));

        for n in 0..12 {
            std::fs::create_dir_all(root.join(format!("r{n:02}/.git"))).unwrap();
        }
        assert!(launch_section(root).unwrap().contains(" and 4 more (in "));
    }
}

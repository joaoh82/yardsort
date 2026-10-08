//! Your repositories on the forge, for **Add a project → Clone a repository**: the ones you own
//! or collaborate on as a list, and the rest by a search you ask for.
//!
//! A query of our own through `gh api graphql` rather than `gh repo list`, for what that does
//! not give in one answer: collaborations beside your own, a fork's parent with both of its
//! clone URLs, and the total. Organisation repositories are not listed — an account in a large
//! organisation can reach tens of thousands, and a page of them takes 5–8 s
//! ([24](../../../docs/design/24-repositories.md)) — so those come through [`Gh::search_repositories`].
//!
//! Yardsort holds no credential of its own: everything here is `gh`, with what it is logged in as.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::forge::{ForgeError, ForgeResult, Gh, Repo};

/// Which URL `git clone` is given: the one `gh` itself would use, by its `git_protocol` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloneProtocol {
    #[default]
    Https,
    Ssh,
}

impl CloneProtocol {
    /// What `gh config get git_protocol` printed; anything but `ssh` is HTTPS, which is also
    /// `gh`'s own default.
    pub fn from_setting(printed: &str) -> Self {
        if printed.trim().eq_ignore_ascii_case("ssh") {
            Self::Ssh
        } else {
            Self::Https
        }
    }

    fn pick(self, https: Option<&str>, ssh: Option<&str>) -> Option<String> {
        match self {
            Self::Ssh => ssh.or(https),
            Self::Https => https.or(ssh),
        }
        .map(str::to_owned)
    }
}

/// A repository on the forge, as the clone dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepository {
    /// `owner/name`, as the forge writes it.
    pub name_with_owner: String,
    pub description: Option<String>,
    /// The URL to clone, HTTPS or SSH by [`CloneProtocol`].
    pub clone_url: String,
    pub is_private: bool,
    pub is_fork: bool,
    pub is_archived: bool,
    /// The repository this one was forked from, when the forge said which.
    pub parent: Option<RemoteParent>,
    pub language: Option<String>,
    /// When it was last pushed to, as the forge wrote it; `None` for one never pushed to.
    pub pushed_at: Option<String>,
    /// The project that is a clone of it, when one of yours is.
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteParent {
    pub name_with_owner: String,
    pub clone_url: String,
}

/// One page of [`REPOSITORIES_QUERY`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryPage {
    pub repositories: Vec<RemoteRepository>,
    /// How many the account owns or collaborates on, on every page.
    pub total: Option<u32>,
    /// The cursor to ask for the next page with. `None` on the last one.
    pub next: Option<String>,
}

/// What [`Gh::repositories`] managed to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Repositories {
    pub repositories: Vec<RemoteRepository>,
    pub total: Option<u32>,
    /// At least one page arrived.
    pub answered: bool,
    /// Why the reading stopped short, when it did.
    pub problem: Option<String>,
    pub logged_out: bool,
}

impl Repositories {
    /// Say which repositories are already projects. A project is matched by its push remote —
    /// host, owner and name, in any case — never by its folder's name.
    pub fn mark_projects<'a>(&mut self, projects: impl IntoIterator<Item = (&'a str, &'a Repo)>) {
        let projects: Vec<(&str, String)> = projects
            .into_iter()
            .filter(|(_, repo)| repo.host.eq_ignore_ascii_case("github.com"))
            .map(|(id, repo)| (id, format!("{}/{}", repo.owner, repo.name)))
            .collect();
        for repository in &mut self.repositories {
            repository.project_id = projects
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(&repository.name_with_owner))
                .map(|(id, _)| (*id).to_owned());
        }
    }
}

/// The viewer's own repositories and collaborations, most recently pushed first. No `{owner}`
/// placeholder: this one is asked from anywhere, with no repository to resolve.
pub const REPOSITORIES_QUERY: &str = "query($after:String){viewer{login \
repositories(first:100,after:$after,affiliations:[OWNER,COLLABORATOR],\
ownerAffiliations:[OWNER,COLLABORATOR],orderBy:{field:PUSHED_AT,direction:DESC}){\
totalCount pageInfo{hasNextPage endCursor} nodes{nameWithOwner description url sshUrl \
isPrivate isFork isArchived hasIssuesEnabled pushedAt primaryLanguage{name} \
parent{nameWithOwner url sshUrl}}}}}";

/// Read one answer to [`REPOSITORIES_QUERY`].
pub fn page(json: &str, protocol: CloneProtocol) -> ForgeResult<RepositoryPage> {
    let unreadable = |what: &str| ForgeError::Unreadable(what.to_owned());
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let list = parsed
        .get("data")
        .and_then(|data| data.get("viewer"))
        .and_then(|viewer| viewer.get("repositories"))
        .filter(|list| !list.is_null())
        .ok_or_else(|| unreadable("expected the viewer's repositories"))?;
    let repositories = list
        .get("nodes")
        .and_then(|nodes| nodes.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| graphql_repository(node, protocol))
                .collect()
        })
        .unwrap_or_default();
    let info = list.get("pageInfo");
    let next = info
        .filter(|info| info.get("hasNextPage").and_then(|v| v.as_bool()) == Some(true))
        .and_then(|info| info.get("endCursor"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Ok(RepositoryPage {
        repositories,
        total: list
            .get("totalCount")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        next,
    })
}

fn graphql_repository(
    node: &serde_json::Value,
    protocol: CloneProtocol,
) -> Option<RemoteRepository> {
    let text = |key: &str| node.get(key).and_then(|v| v.as_str());
    let flag = |key: &str| node.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    Some(RemoteRepository {
        name_with_owner: text("nameWithOwner")?.to_owned(),
        description: text("description")
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned),
        clone_url: protocol.pick(
            text("url").map(|url| url.to_owned() + ".git").as_deref(),
            text("sshUrl"),
        )?,
        is_private: flag("isPrivate"),
        is_fork: flag("isFork"),
        is_archived: flag("isArchived"),
        parent: node
            .get("parent")
            .filter(|parent| !parent.is_null())
            .and_then(|parent| {
                let text = |key: &str| parent.get(key).and_then(|v| v.as_str());
                Some(RemoteParent {
                    name_with_owner: text("nameWithOwner")?.to_owned(),
                    clone_url: protocol.pick(
                        text("url").map(|url| url.to_owned() + ".git").as_deref(),
                        text("sshUrl"),
                    )?,
                })
            }),
        language: node
            .get("primaryLanguage")
            .and_then(|language| language.get("name"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        pushed_at: text("pushedAt").map(str::to_owned),
        project_id: None,
    })
}

/// Read what `gh api search/repositories` printed: REST's names for the same things.
pub fn search_results(json: &str, protocol: CloneProtocol) -> ForgeResult<Vec<RemoteRepository>> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let items = parsed
        .get("items")
        .and_then(|items| items.as_array())
        .ok_or_else(|| ForgeError::Unreadable("expected search results".to_owned()))?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let text = |key: &str| item.get(key).and_then(|v| v.as_str());
            let flag = |key: &str| item.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
            Some(RemoteRepository {
                name_with_owner: text("full_name")?.to_owned(),
                description: text("description")
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned),
                clone_url: protocol.pick(text("clone_url"), text("ssh_url"))?,
                is_private: flag("private"),
                is_fork: flag("fork"),
                is_archived: flag("archived"),
                // A search result names no parent; the dialog says "fork" without saying of what.
                parent: None,
                language: text("language").map(str::to_owned),
                pushed_at: text("pushed_at").map(str::to_owned),
                project_id: None,
            })
        })
        .collect())
}

impl Gh {
    /// The protocol `gh` is set to clone with. Not an error when it cannot say: HTTPS then, as
    /// `gh` itself would.
    pub fn git_protocol(&self, cwd: &Path) -> CloneProtocol {
        self.run(cwd, &["config", "get", "git_protocol"])
            .map(|printed| CloneProtocol::from_setting(&printed))
            .unwrap_or_default()
    }

    fn repositories_page(
        &self,
        cwd: &Path,
        after: Option<&str>,
        protocol: CloneProtocol,
        limit: Duration,
    ) -> ForgeResult<RepositoryPage> {
        let query = format!("query={REPOSITORIES_QUERY}");
        let mut args = vec!["api", "graphql", "-f", &query];
        let after = after.map(|cursor| format!("after={cursor}"));
        if let Some(after) = after.as_deref() {
            args.extend(["-F", after]);
        }
        let out = self
            .run_within(cwd, &args, limit)
            .map_err(|error| match error {
                // The command is the whole query otherwise, which helps nobody read the reason.
                ForgeError::Failed { stderr, .. } => ForgeError::Failed {
                    command: "api graphql".to_owned(),
                    stderr,
                },
                other => other,
            })?;
        page(&out, protocol)
    }

    /// Every repository the account owns or collaborates on, a page at a time, up to `cap`.
    ///
    /// Never an `Err`: a page that fails ends the reading and says why, and the pages before it
    /// are kept.
    pub fn repositories(
        &self,
        cwd: &Path,
        protocol: CloneProtocol,
        cap: usize,
        limit: Duration,
    ) -> Repositories {
        let mut found = Repositories::default();
        let mut after: Option<String> = None;
        loop {
            let page = match self.repositories_page(cwd, after.as_deref(), protocol, limit) {
                Ok(page) => page,
                Err(error) => {
                    found.logged_out = error.is_logged_out();
                    found.problem = Some(error.to_string());
                    break;
                }
            };
            found.answered = true;
            found.total = page.total.or(found.total);
            for repository in page.repositories {
                // Ordered by when they were pushed to, so one pushed to between two pages can
                // turn up on both.
                if !found
                    .repositories
                    .iter()
                    .any(|seen| seen.name_with_owner == repository.name_with_owner)
                {
                    found.repositories.push(repository);
                }
            }
            if found.repositories.len() >= cap {
                found.repositories.truncate(cap);
                break;
            }
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        found
    }

    /// Repositories whose name contains `text`, from the whole forge: the account's own, its
    /// organisations', and everyone's public ones. The text goes to `gh` as a query parameter,
    /// never spliced into a path.
    pub fn search_repositories(
        &self,
        cwd: &Path,
        text: &str,
        protocol: CloneProtocol,
        limit: Duration,
    ) -> ForgeResult<Vec<RemoteRepository>> {
        let q = format!("q={} in:name", text.trim());
        let out = self.run_within(
            cwd,
            &[
                "api",
                "-X",
                "GET",
                "search/repositories",
                "-f",
                &q,
                "-F",
                "per_page=20",
            ],
            limit,
        )?;
        search_results(&out, protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/gh/2.102.0")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn a_page_is_read_with_its_total_and_cursor() {
        let page = page(&fixture("repos-page-1.json"), CloneProtocol::Https).unwrap();
        assert_eq!(page.total, Some(6));
        assert!(page.next.is_some());
        let names: Vec<_> = page
            .repositories
            .iter()
            .map(|r| r.name_with_owner.as_str())
            .collect();
        assert_eq!(
            names,
            ["o/weather-cli", "o/o", "o/awesome-tools", "o/old-site"]
        );
        let first = &page.repositories[0];
        assert_eq!(first.clone_url, "https://github.com/o/weather-cli.git");
        assert_eq!(first.language.as_deref(), Some("Rust"));
        assert_eq!(
            first.description.as_deref(),
            Some("A weather forecast on the command line")
        );
        assert_eq!(first.pushed_at.as_deref(), Some("2026-10-08T16:30:18Z"));
        let bare = &page.repositories[1];
        assert_eq!(bare.description, None);
        assert_eq!(bare.language, None);
        let fork = &page.repositories[2];
        assert!(fork.is_fork);
        assert_eq!(
            fork.parent,
            Some(RemoteParent {
                name_with_owner: "upstream/awesome-tools".to_owned(),
                clone_url: "https://github.com/upstream/awesome-tools.git".to_owned(),
            })
        );
        assert!(page.repositories[3].is_archived);
        assert!(page.repositories.iter().all(|r| r.project_id.is_none()));
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        let page = page(&fixture("repos-page-2.json"), CloneProtocol::Https).unwrap();
        assert_eq!(page.next, None);
        assert_eq!(page.repositories.len(), 2);
    }

    #[test]
    fn the_protocol_decides_which_url_is_cloned() {
        let page = page(&fixture("repos-page-1.json"), CloneProtocol::Ssh).unwrap();
        assert_eq!(
            page.repositories[0].clone_url,
            "git@github.com:o/weather-cli.git"
        );
        assert_eq!(
            page.repositories[2].parent.as_ref().unwrap().clone_url,
            "git@github.com:upstream/awesome-tools.git"
        );
        assert_eq!(CloneProtocol::from_setting("ssh\n"), CloneProtocol::Ssh);
        assert_eq!(CloneProtocol::from_setting("https"), CloneProtocol::Https);
        assert_eq!(CloneProtocol::from_setting(""), CloneProtocol::Https);
    }

    #[test]
    fn search_results_are_read_in_rests_names() {
        let found = search_results(&fixture("repos-search.json"), CloneProtocol::Https).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name_with_owner, "o/weather-cli");
        assert_eq!(found[0].clone_url, "https://github.com/o/weather-cli.git");
        assert_eq!(found[0].language.as_deref(), Some("Rust"));
        assert!(found[1].is_private);
        assert_eq!(found[1].description, None);
        assert_eq!(found[1].parent, None);
        let ssh = search_results(&fixture("repos-search.json"), CloneProtocol::Ssh).unwrap();
        assert_eq!(
            ssh[1].clone_url,
            "git@github.com:someone/weather-cli-plugins.git"
        );
    }

    #[test]
    fn something_else_is_unreadable_not_empty() {
        assert!(page("{\"data\":{\"viewer\":null}}", CloneProtocol::Https).is_err());
        assert!(
            search_results("{\"message\":\"Validation Failed\"}", CloneProtocol::Https).is_err()
        );
    }

    #[test]
    fn a_project_is_matched_by_its_remote_in_any_case_and_only_on_github() {
        let mut found = Repositories {
            repositories: page(&fixture("repos-page-1.json"), CloneProtocol::Https)
                .unwrap()
                .repositories,
            ..Default::default()
        };
        let github = crate::forge::parse_remote("git@github.com:O/Weather-CLI.git").unwrap();
        let elsewhere = crate::forge::parse_remote("https://gitlab.com/o/old-site.git").unwrap();
        found.mark_projects([("p1", &github), ("p2", &elsewhere)]);
        assert_eq!(found.repositories[0].project_id.as_deref(), Some("p1"));
        assert_eq!(found.repositories[3].project_id, None);
        assert!(found.repositories[1..]
            .iter()
            .all(|r| r.project_id.is_none()));
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
        let first = fixtures.join("repos-page-1.json");
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
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0/repos-page-2.json");
        let gh = paged(dir.path(), &format!("cat '{}'", second.display()));
        let found = gh.repositories(
            dir.path(),
            CloneProtocol::Https,
            200,
            Duration::from_secs(10),
        );
        assert!(found.answered);
        assert_eq!(found.problem, None);
        assert_eq!(found.total, Some(6));
        assert_eq!(found.repositories.len(), 6);
        assert_eq!(found.repositories[5].name_with_owner, "o/pkgs");
        let asked = asked(dir.path());
        assert_eq!(asked.len(), 2);
        assert!(
            asked[0].starts_with("api graphql -f query="),
            "{}",
            asked[0]
        );
        assert!(!asked[0].contains("{owner}"), "{}", asked[0]);
        assert!(
            asked[1].ends_with("-F after=Y3Vyc29yOnYyOpK0MjAyNi0xMC0wMVQxMjowMDowMFrOBHa0OA=="),
            "{}",
            asked[1]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_fails_keeps_the_ones_before_it_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged(dir.path(), "echo 'gh: Something went wrong' >&2; exit 1");
        let found = gh.repositories(
            dir.path(),
            CloneProtocol::Https,
            200,
            Duration::from_secs(10),
        );
        assert!(found.answered);
        assert_eq!(found.repositories.len(), 4);
        let problem = found.problem.unwrap();
        assert!(problem.contains("Something went wrong"), "{problem}");
        assert!(problem.starts_with("`gh api graphql` failed"), "{problem}");
        assert!(!found.logged_out);
    }

    #[cfg(unix)]
    #[test]
    fn the_cap_stops_the_reading_and_being_logged_out_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged(dir.path(), "exit 3");
        let found = gh.repositories(dir.path(), CloneProtocol::Https, 3, Duration::from_secs(10));
        assert_eq!(found.repositories.len(), 3);
        assert_eq!(
            asked(dir.path()).len(),
            1,
            "the cap was reached on the first page"
        );

        let dir = tempfile::tempdir().unwrap();
        let gh = stand_in(
            dir.path(),
            "#!/bin/sh\necho 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4\n",
        );
        let found = gh.repositories(
            dir.path(),
            CloneProtocol::Https,
            200,
            Duration::from_secs(10),
        );
        assert!(!found.answered);
        assert!(found.logged_out);
        assert!(found.repositories.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_search_sends_its_text_as_a_parameter_and_the_protocol_as_a_setting() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0/repos-search.json");
        let gh = stand_in(
            dir.path(),
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncase \"$*\" in\n\
                 *git_protocol*) echo ssh ;;\n\
                 *) cat '{}' ;;\nesac\n",
                fixture.display()
            ),
        );
        assert_eq!(gh.git_protocol(dir.path()), CloneProtocol::Ssh);
        let found = gh
            .search_repositories(
                dir.path(),
                " weather ",
                CloneProtocol::Ssh,
                Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].clone_url, "git@github.com:o/weather-cli.git");
        let asked = asked(dir.path());
        assert_eq!(asked[0], "config get git_protocol");
        assert_eq!(
            asked[1],
            "api -X GET search/repositories -f q=weather in:name -F per_page=20"
        );
    }
}

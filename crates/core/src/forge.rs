//! The forge: where a pushed branch becomes a pull request, and where CI says what it thinks.
//!
//! Two levels, because only one of them can be relied on:
//!
//! - **The remote URL** is always there. Parsed into a host, an owner and a repository name it
//!   gives a *compare* page — a link that opens the forge's own "open a pull request" form with
//!   the branches already filled in. No account, no token, no extra program: this is the floor,
//!   and it works for GitHub, GitLab, Bitbucket and the Gitea family.
//! - **[`gh`](Gh)**, when the user has it and has logged in, does the rest: it opens the pull
//!   request without leaving Yardsort, and it can say what the pull request and its checks are
//!   doing afterwards.
//!
//! Yardsort holds no forge credentials of its own. `gh` already keeps the user's, in the way
//! they expect and can revoke; a second copy in our credential store would be one more secret to
//! leak. It follows that everything here degrades to the link when `gh` is absent or logged out,
//! and that is a supported state rather than an error to report.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::env::ShellEnv;
use crate::program::Program;

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("gh is not installed, or not on PATH")]
    NotInstalled,
    #[error("`gh {command}` failed: {stderr}")]
    Failed { command: String, stderr: String },
    #[error("could not read what gh printed: {0}")]
    Unreadable(String),
    #[error("could not run gh: {0}")]
    Io(#[from] std::io::Error),
}

impl ForgeError {
    /// The stable code the webview switches on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled => "gh_not_installed",
            Self::Failed { .. } => "gh_failed",
            Self::Unreadable(_) => "gh_unreadable",
            Self::Io(_) => "gh_io",
        }
    }

    /// Whether this is `gh` saying "log in first" rather than anything being broken.
    pub fn is_logged_out(&self) -> bool {
        match self {
            Self::Failed { stderr, .. } => {
                let said = stderr.to_ascii_lowercase();
                said.contains("gh auth login")
                    || said.contains("authentication required")
                    || said.contains("not logged into")
            }
            _ => false,
        }
    }
}

pub type ForgeResult<T> = Result<T, ForgeError>;

/// Which forge a host is, as far as the shape of its URLs goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    GitHub,
    GitLab,
    Bitbucket,
    /// Gitea, Forgejo, and anything else self-hosted: assumed to speak GitHub's URL shapes,
    /// which the Gitea family does. A wrong guess costs a link that 404s, not data.
    Unknown,
}

/// A repository on a forge, as read out of a remote URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub host: String,
    /// The owner, which on GitLab may itself contain `/` for subgroups.
    pub owner: String,
    pub name: String,
    pub kind: ForgeKind,
}

impl Repo {
    /// The forge's "open a pull request" form, with both branches filled in.
    pub fn compare_url(&self, base: &str, head: &str) -> String {
        let Self {
            host, owner, name, ..
        } = self;
        let base = encode(base);
        let head = encode(head);
        match self.kind {
            ForgeKind::GitLab => format!(
                "https://{host}/{owner}/{name}/-/merge_requests/new\
                 ?merge_request%5Bsource_branch%5D={head}\
                 &merge_request%5Btarget_branch%5D={base}"
            ),
            ForgeKind::Bitbucket => {
                format!("https://{host}/{owner}/{name}/pull-requests/new?source={head}&dest={base}")
            }
            // Gitea and Forgejo use GitHub's compare path too.
            ForgeKind::GitHub | ForgeKind::Unknown => {
                format!("https://{host}/{owner}/{name}/compare/{base}...{head}?expand=1")
            }
        }
    }
}

/// Percent-encode the characters a branch name may legally contain that a URL may not.
/// Git already forbids most of what would matter; `#`, `?` and `%` are the ones left.
fn encode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '%' => "%25".to_owned(),
            '#' => "%23".to_owned(),
            '?' => "%3F".to_owned(),
            '&' => "%26".to_owned(),
            ' ' => "%20".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

fn kind_of(host: &str) -> ForgeKind {
    let host = host.to_ascii_lowercase();
    if host == "github.com" || host.starts_with("github.") || host.ends_with(".github.com") {
        ForgeKind::GitHub
    } else if host.contains("gitlab") {
        ForgeKind::GitLab
    } else if host.contains("bitbucket") {
        ForgeKind::Bitbucket
    } else {
        ForgeKind::Unknown
    }
}

/// Read a git remote URL as a repository on a forge.
///
/// Handles the three shapes git hands out — `git@host:owner/repo.git`,
/// `ssh://git@host/owner/repo.git` and `https://host/owner/repo.git` — and returns `None` for
/// anything else, a local path most of all.
pub fn parse_remote(url: &str) -> Option<Repo> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, rest)| rest) {
        // scheme://[user@]host[:port]/owner/repo
        let rest = rest.split_once('@').map_or(rest, |(_, rest)| rest);
        let (authority, path) = rest.split_once('/')?;
        (
            authority.split_once(':').map_or(authority, |(h, _)| h),
            path,
        )
    } else {
        // [user@]host:owner/repo — git's scp-like form. A Windows path (`C:\…`) has no `@`
        // and no `/`, so it falls out at the split below rather than being taken for a host.
        let rest = url.split_once('@').map_or(url, |(_, rest)| rest);
        let (host, path) = rest.split_once(':')?;
        (host, path)
    };

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.rsplit_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(Repo {
        host: host.to_owned(),
        owner: owner.trim_matches('/').to_owned(),
        name: name.to_owned(),
        kind: kind_of(host),
    })
}

/// What CI says about a pull request's head commit, rolled up into the one thing a row can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Checks {
    /// Nothing is configured, or nothing has reported yet.
    None,
    Running,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}

/// A pull request, as much of it as a workspace row and the panel need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u32,
    pub url: String,
    pub title: String,
    /// The branch it would merge, which is how a workspace finds its own.
    pub branch: String,
    pub state: PullRequestState,
    pub draft: bool,
    pub checks: Checks,
}

/// The `gh` command line, found on the user's `PATH`.
pub struct Gh {
    program: Program,
}

impl Gh {
    pub fn find(env: &ShellEnv) -> Option<Self> {
        Program::find(env, "gh").map(|program| Self { program })
    }

    pub fn path(&self) -> &Path {
        self.program.path()
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> ForgeResult<String> {
        let output = self
            .program
            .command(cwd)
            .args(args)
            // gh dresses its output up for a terminal otherwise, and asks questions.
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(ForgeError::Failed {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Every pull request `gh` will tell us about for the repository at `root`, newest first.
    ///
    /// One call answers for every workspace in the project, which is why it is a list and not a
    /// lookup per branch: a project with a dozen workspaces would otherwise be a dozen round
    /// trips every time the window came back into focus.
    pub fn pull_requests(&self, root: &Path, limit: u32) -> ForgeResult<Vec<PullRequest>> {
        let limit = limit.to_string();
        let out = self.run(
            root,
            &[
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                &limit,
                "--json",
                "number,url,title,headRefName,state,isDraft,statusCheckRollup",
            ],
        )?;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
        let rows = parsed
            .as_array()
            .ok_or_else(|| ForgeError::Unreadable("expected a list of pull requests".to_owned()))?;
        Ok(rows.iter().filter_map(pull_request).collect())
    }

    /// Open a pull request for the branch checked out at `root`, and return its URL.
    pub fn create_pull_request(
        &self,
        root: &Path,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> ForgeResult<String> {
        let mut args = vec![
            "pr", "create", "--base", base, "--title", title, "--body", body,
        ];
        if draft {
            args.push("--draft");
        }
        let out = self.run(root, &args)?;
        // gh prints the URL on the last line, after whatever else it felt like saying.
        Ok(out
            .lines()
            .rev()
            .find(|line| line.starts_with("http"))
            .unwrap_or(&out)
            .trim()
            .to_owned())
    }
}

fn pull_request(row: &serde_json::Value) -> Option<PullRequest> {
    Some(PullRequest {
        number: u32::try_from(row.get("number")?.as_u64()?).ok()?,
        url: row.get("url")?.as_str()?.to_owned(),
        title: row
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned(),
        branch: row.get("headRefName")?.as_str()?.to_owned(),
        state: match row.get("state").and_then(|s| s.as_str()).unwrap_or("OPEN") {
            "MERGED" => PullRequestState::Merged,
            "CLOSED" => PullRequestState::Closed,
            _ => PullRequestState::Open,
        },
        draft: row
            .get("isDraft")
            .and_then(|d| d.as_bool())
            .unwrap_or(false),
        checks: roll_up(row.get("statusCheckRollup")),
    })
}

/// Reduce every check on the head commit to one verdict.
///
/// One failure outranks everything — a green summary hiding a red check is the one answer that
/// would make this worse than not showing it at all. Anything still running outranks success,
/// so "passing" always means *finished* and passing.
fn roll_up(rollup: Option<&serde_json::Value>) -> Checks {
    let Some(checks) = rollup.and_then(|value| value.as_array()) else {
        return Checks::None;
    };
    if checks.is_empty() {
        return Checks::None;
    }
    let mut running = false;
    let mut any = false;
    for check in checks {
        // A check run reports `status` then `conclusion`; a commit status only has `state`.
        let status = check.get("status").and_then(|s| s.as_str());
        let verdict = check
            .get("conclusion")
            .and_then(|c| c.as_str())
            .or_else(|| check.get("state").and_then(|s| s.as_str()))
            .unwrap_or("")
            .to_ascii_uppercase();
        let finished = status.is_none_or(|status| status.eq_ignore_ascii_case("COMPLETED"));
        match verdict.as_str() {
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => return Checks::Failing,
            "SUCCESS" | "NEUTRAL" | "SKIPPED" if finished => any = true,
            _ => running = true,
        }
    }
    if running {
        Checks::Running
    } else if any {
        Checks::Passing
    } else {
        Checks::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shapes_git_hands_out() {
        let expected = Repo {
            host: "github.com".into(),
            owner: "joaoh82".into(),
            name: "yardsort".into(),
            kind: ForgeKind::GitHub,
        };
        for url in [
            "git@github.com:joaoh82/yardsort.git",
            "git@github.com:joaoh82/yardsort",
            "ssh://git@github.com/joaoh82/yardsort.git",
            "https://github.com/joaoh82/yardsort.git",
            "https://github.com/joaoh82/yardsort",
            "https://joaoh82@github.com/joaoh82/yardsort.git",
        ] {
            assert_eq!(parse_remote(url).as_ref(), Some(&expected), "{url}");
        }
    }

    #[test]
    fn keeps_a_gitlab_subgroup_in_the_owner() {
        let repo = parse_remote("git@gitlab.com:group/sub/thing.git").expect("parsed");
        assert_eq!(repo.owner, "group/sub");
        assert_eq!(repo.name, "thing");
        assert_eq!(repo.kind, ForgeKind::GitLab);
    }

    #[test]
    fn a_local_remote_is_not_a_forge() {
        for url in [
            "/srv/git/thing.git",
            "../sibling",
            r"C:\repos\thing",
            "file:///srv/git/thing.git",
        ] {
            assert_eq!(parse_remote(url), None, "{url}");
        }
    }

    #[test]
    fn a_self_hosted_host_gets_githubs_urls() {
        let repo = parse_remote("git@git.example.org:team/thing.git").expect("parsed");
        assert_eq!(repo.kind, ForgeKind::Unknown);
        assert_eq!(
            repo.compare_url("main", "ys/fix"),
            "https://git.example.org/team/thing/compare/main...ys/fix?expand=1"
        );
    }

    #[test]
    fn each_forge_gets_its_own_form() {
        let github = parse_remote("git@github.com:o/r.git").expect("parsed");
        assert_eq!(
            github.compare_url("main", "ys/fix"),
            "https://github.com/o/r/compare/main...ys/fix?expand=1"
        );
        let gitlab = parse_remote("git@gitlab.com:o/r.git").expect("parsed");
        assert!(gitlab
            .compare_url("main", "ys/fix")
            .contains("merge_request%5Bsource_branch%5D=ys/fix"));
        let bitbucket = parse_remote("git@bitbucket.org:o/r.git").expect("parsed");
        assert_eq!(
            bitbucket.compare_url("main", "ys/fix"),
            "https://bitbucket.org/o/r/pull-requests/new?source=ys/fix&dest=main"
        );
    }

    #[test]
    fn a_branch_name_cannot_break_out_of_the_query() {
        let repo = parse_remote("git@github.com:o/r.git").expect("parsed");
        assert_eq!(
            repo.compare_url("main", "ys/fix#1"),
            "https://github.com/o/r/compare/main...ys/fix%231?expand=1"
        );
    }

    fn rollup(json: &str) -> Checks {
        roll_up(Some(&serde_json::from_str(json).expect("json")))
    }

    #[test]
    fn one_failure_outranks_every_success() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},
                    {"status":"COMPLETED","conclusion":"FAILURE"}]"#
            ),
            Checks::Failing
        );
    }

    #[test]
    fn anything_still_running_outranks_success() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},
                    {"status":"IN_PROGRESS","conclusion":null}]"#
            ),
            Checks::Running
        );
    }

    #[test]
    fn a_skipped_check_still_counts_as_passing() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SKIPPED"},
                    {"status":"COMPLETED","conclusion":"SUCCESS"}]"#
            ),
            Checks::Passing
        );
    }

    #[test]
    fn a_commit_status_reports_state_instead() {
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"SUCCESS"}]"#),
            Checks::Passing
        );
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"PENDING"}]"#),
            Checks::Running
        );
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"ERROR"}]"#),
            Checks::Failing
        );
    }

    #[test]
    fn no_checks_at_all_says_nothing() {
        assert_eq!(rollup("[]"), Checks::None);
        assert_eq!(roll_up(None), Checks::None);
        assert_eq!(roll_up(Some(&serde_json::Value::Null)), Checks::None);
    }

    #[test]
    fn reads_what_gh_prints() {
        let rows: serde_json::Value = serde_json::from_str(
            r#"[{"number":42,"url":"https://github.com/o/r/pull/42","title":"Fix login",
                 "headRefName":"ys/fix-login","state":"OPEN","isDraft":true,
                 "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}]}]"#,
        )
        .expect("json");
        let pr = pull_request(&rows[0]).expect("a pull request");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.branch, "ys/fix-login");
        assert_eq!(pr.state, PullRequestState::Open);
        assert!(pr.draft);
        assert_eq!(pr.checks, Checks::Passing);
    }

    /// Against the real `gh`, the real network and this repository — so the field names above
    /// are checked against what GitHub actually returns rather than against a fixture that was
    /// right once. Ignored by default: CI has no network and no login.
    ///
    /// `cargo test -p yardsort-core -- --ignored --nocapture reads_this_repositorys`
    #[test]
    #[ignore = "needs gh, a login and the network"]
    fn reads_this_repositorys_own_pull_requests() {
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::find(&env).expect("gh on PATH");
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = gh.pull_requests(here, 5).expect("gh answered");

        assert!(!found.is_empty(), "this repository has pull requests");
        for pr in &found {
            println!(
                "#{} {:?} {:?} {}",
                pr.number, pr.state, pr.checks, pr.branch
            );
            assert!(pr.number > 0);
            assert!(pr.url.starts_with("https://"));
            assert!(!pr.branch.is_empty());
        }
        assert!(
            found.iter().any(|pr| pr.checks != Checks::None),
            "at least one of them ran checks, so the rollup really was read"
        );
    }

    #[test]
    fn logged_out_is_told_apart_from_broken() {
        let logged_out = ForgeError::Failed {
            command: "pr list".into(),
            stderr: "To get started with GitHub CLI, please run: gh auth login".into(),
        };
        assert!(logged_out.is_logged_out());
        let broken = ForgeError::Failed {
            command: "pr list".into(),
            stderr: "could not resolve host".into(),
        };
        assert!(!broken.is_logged_out());
    }
}

//! Pull requests a workspace follows in repositories other than its project's.
//!
//! A project's own pull requests are found by asking its repository and matching branches and
//! commits to workspaces (`crate::forge::pull_requests_from`). A project whose checkout holds
//! other repositories — nested clones its agents go into and open pull requests from — has pull
//! requests that leave no such trace, so they are written down: by an agent through
//! `ys pr track`, by the user in the app, or by an adapter that saw one being created (below,
//! from [`link_in`] on). See `docs/design/38-pull-requests-elsewhere.md`.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

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

/// The nested clone of `repo` in the project's checkout at `root`: a folder directly inside it
/// with a `.git` of its own and a remote that is that repository. Where a tracked pull request's
/// commits are fetched for its diff (slice 4 of docs/design/38-pull-requests-elsewhere.md):
/// the project's own repository has none of its history.
pub fn clone_of(git: &Git, root: &Path, repo: &Repo) -> Option<std::path::PathBuf> {
    let clones: Vec<(std::path::PathBuf, Vec<Repo>)> = nested_repositories(root)
        .into_iter()
        .map(|name| root.join(name))
        .map(|clone| {
            let repos = crate::forge::repos_at(git, &clone);
            (clone, repos)
        })
        .collect();
    let named = |here: &Repo| {
        here.owner.eq_ignore_ascii_case(&repo.owner) && here.name.eq_ignore_ascii_case(&repo.name)
    };
    // On its own host first; then by owner and name alone, for a remote reached through an SSH
    // host alias (`git@github-work:o/lib`), which names no forge's host.
    clones
        .iter()
        .find(|(_, repos)| {
            repos
                .iter()
                .any(|here| named(here) && here.host.eq_ignore_ascii_case(&repo.host))
        })
        .or_else(|| clones.iter().find(|(_, repos)| repos.iter().any(named)))
        .map(|(clone, _)| clone.clone())
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
pub fn launch_section(project_root: &Path, workspace: &Path) -> Option<String> {
    let nested = nested_repositories(project_root);
    if nested.is_empty() {
        return None;
    }
    // Each by its whole path: a name alone reads as a folder of the agent's own worktree, which
    // is a worktree of the parent and has none of them (seen on the Linux hands-on pass, where
    // an agent went looking for `lib` beside it).
    let mut named = nested
        .iter()
        .take(NAMED_REPOSITORIES)
        .map(|name| code(&project_root.join(name).display().to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    if nested.len() > NAMED_REPOSITORIES {
        named.push_str(&format!(
            ", and {} more in {}",
            nested.len() - NAMED_REPOSITORIES,
            code(&project_root.display().to_string())
        ));
    }
    // A project's local workspace is its checkout itself, where the clones are folders of its own.
    let inside = crate::git::normalize(workspace) == crate::git::normalize(project_root);
    let place = if inside {
        String::new()
    } else {
        ", which are not in this workspace's folder".to_owned()
    };
    Some(format!(
        "## Other repositories\n\n\
         This project's checkout holds other git repositories{place}: {named}. Every workspace of \
         the project shares them, so to work on a branch of one, make a worktree of it for \
         yourself (`git -C <the clone> worktree add <a new folder outside it> -b <branch>`) \
         rather than changing the clone's own checkout. When you open a pull request in one of \
         them, run `ys pr track <its url>` so Yardsort follows it with this workspace."
    ))
}

/// `text` as inline code, with a fence longer than any run of backticks in it: a folder name may
/// hold one.
fn code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

// Seen being created. An adapter that sees a tool call open a pull request keeps its link, and
// nothing else of the call, as `pullRequestUrl` on the `tool.completed` event; draining the
// inbox hands those links to [`track_seen`]. The rules are here, not in each adapter, so every
// agent's calls are judged alike.

/// The first link to a GitHub pull request in `text`, rebuilt as [`parse`] rebuilds one. A
/// branch's "open a pull request" link (`/pull/new/<branch>`) and an API address
/// (`/repos/…/pulls/7`) are not one.
pub fn link_in(text: &str) -> Option<String> {
    links_in(text).next()
}

/// The last such link: what a command printed ends with the link `gh pr create` printed, after
/// whatever the commands before it in the same call printed — notes that name other pull
/// requests, say.
pub fn last_link_in(text: &str) -> Option<String> {
    links_in(text).last()
}

fn links_in(text: &str) -> impl Iterator<Item = String> + '_ {
    let mut rest = text;
    std::iter::from_fn(move || {
        while let Some(at) = rest.find("http") {
            let candidate = &rest[at..];
            let end = candidate
                .find(|c: char| c.is_whitespace() || "\"'`<>()[]{},\\".contains(c))
                .unwrap_or(candidate.len());
            rest = &candidate[end.max(4)..];
            let link = candidate[..end].trim_end_matches(['.', ';', ':', '!', '?']);
            if link.starts_with("https://") || link.starts_with("http://") {
                if let Ok(found) = parse(link) {
                    return Some(found.url);
                }
            }
        }
        None
    })
}

/// Whether a shell command runs `gh pr create`, as a command of its own anywhere in it: after a
/// `cd … &&`, behind a variable set for it or `env`, by a path to `gh`, with `-R owner/name`
/// between. `gh` as an argument — `echo gh pr create`, a commit message — is not running it, and
/// neither is text in quotes or in a here-document, whatever separators it holds.
pub fn runs_pr_create(command: &str) -> bool {
    /// Programs that run the command after them, and so leave it in command position.
    const WRAPPERS: &[&str] = &[
        "command", "exec", "env", "time", "sudo", "nohup", "nice", "timeout", "xargs",
    ];
    /// Words of the shell's own that a command follows: `if gh pr create; then …`, `! gh …`.
    const RESERVED: &[&str] = &["if", "then", "elif", "else", "do", "while", "until", "!"];
    simple_commands(command).iter().any(|part| {
        let mut words = part.iter().map(String::as_str).peekable();
        // The wrapper the words now belong to, and whether `timeout` has had its duration.
        let mut wrapper: Option<&str> = None;
        let mut timed = false;
        while let Some(&word) = words.peek() {
            let assignment = !word.starts_with('-') && word.contains('=');
            if RESERVED.contains(&word) || assignment {
                words.next();
            } else if WRAPPERS.contains(&word) {
                wrapper = Some(word);
                timed = false;
                words.next();
            } else if let (Some(by), true) = (wrapper, word.starts_with('-')) {
                words.next();
                // `env -u NAME`, `sudo -u user`, `nice -n 10`: the flag's value is not the
                // command.
                let valued = match by {
                    "env" => matches!(word, "-u" | "--unset" | "-C" | "--chdir" | "-S"),
                    "sudo" => matches!(word, "-u" | "-g" | "-C" | "-D" | "-h" | "-p" | "-U"),
                    "nice" => word == "-n",
                    "timeout" => matches!(word, "-s" | "--signal" | "-k" | "--kill-after"),
                    "xargs" => {
                        matches!(word, "-I" | "-n" | "-L" | "-P" | "-d" | "-E" | "-s" | "-a")
                    }
                    _ => false,
                };
                if valued {
                    words.next();
                }
            } else if wrapper == Some("timeout") && !timed {
                // `timeout 60 gh …`: the duration first.
                timed = true;
                words.next();
            } else {
                break;
            }
        }
        let Some(program) = words.next() else {
            return false;
        };
        let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
        if !program.eq_ignore_ascii_case("gh") && !program.eq_ignore_ascii_case("gh.exe") {
            return false;
        }
        // What `gh` was asked, past its flags and the repository one of them names.
        let mut asked = Vec::new();
        while let Some(word) = words.next() {
            if word == "-R" || word == "--repo" {
                words.next();
            } else if !word.starts_with('-') {
                asked.push(word);
                if asked.len() == 2 {
                    break;
                }
            }
        }
        asked == ["pr", "create"]
    })
}

/// The commands in a shell command line, each as its words, split the way a shell splits them:
/// quotes and backslashes keep what they hold inside one word, and comments and here-document
/// bodies are left out. Separators — `;`, `&`, `|`, a newline, parentheses, braces, backticks —
/// end a command only outside quotes. Text in double quotes stays text, even a `$(…)` in it: a
/// command missed is only a link left for the launch note, while one invented tracks the wrong
/// pull request.
fn simple_commands(line: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = line.chars().collect();
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word: Option<String> = None;
    // Here-documents opened on this line, by delimiter, and whether `<<-` strips its tabs.
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    let end_word = |word: &mut Option<String>, words: &mut Vec<String>| {
        if let Some(done) = word.take() {
            words.push(done);
        }
    };
    let end_command = |words: &mut Vec<String>, commands: &mut Vec<Vec<String>>| {
        if !words.is_empty() {
            commands.push(std::mem::take(words));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                let text = word.get_or_insert_with(String::new);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    text.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                let text = word.get_or_insert_with(String::new);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() && "\"\\$`".contains(chars[i + 1]) {
                        i += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                // An escaped newline joins two lines; anything else escaped is itself.
                if let Some(&next) = chars.get(i + 1) {
                    if next != '\n' {
                        word.get_or_insert_with(String::new).push(next);
                    }
                }
                i += 1;
            }
            '#' if word.is_none() => {
                while i + 1 < chars.len() && chars[i + 1] != '\n' {
                    i += 1;
                }
            }
            '<' if chars.get(i + 1) == Some(&'<') && chars.get(i + 2) != Some(&'<') => {
                end_word(&mut word, &mut words);
                i += 2;
                let strip_tabs = chars.get(i) == Some(&'-');
                if strip_tabs {
                    i += 1;
                }
                while matches!(chars.get(i), Some(' ' | '\t')) {
                    i += 1;
                }
                let mut delimiter = String::new();
                while let Some(&d) = chars.get(i) {
                    if d.is_whitespace() || ";&|()<>".contains(d) {
                        break;
                    }
                    if !"'\"\\".contains(d) {
                        delimiter.push(d);
                    }
                    i += 1;
                }
                heredocs.push((delimiter, strip_tabs));
                continue;
            }
            '\n' => {
                end_word(&mut word, &mut words);
                end_command(&mut words, &mut commands);
                // The bodies of the here-documents this line opened follow it, one after another.
                for (delimiter, strip_tabs) in heredocs.drain(..) {
                    loop {
                        i += 1;
                        if i >= chars.len() {
                            return commands;
                        }
                        let end = chars[i..]
                            .iter()
                            .position(|&c| c == '\n')
                            .map_or(chars.len(), |at| i + at);
                        let body: String = chars[i..end].iter().collect();
                        let body = if strip_tabs {
                            body.trim_start_matches('\t')
                        } else {
                            &body
                        };
                        i = end;
                        if body == delimiter {
                            break;
                        }
                    }
                }
            }
            ';' | '&' | '|' | '(' | ')' | '`' | '{' | '}' => {
                end_word(&mut word, &mut words);
                end_command(&mut words, &mut commands);
            }
            c if c.is_whitespace() => end_word(&mut word, &mut words),
            c => word.get_or_insert_with(String::new).push(c),
        }
        i += 1;
    }
    end_word(&mut word, &mut words);
    end_command(&mut words, &mut commands);
    commands
}

/// Whether an MCP tool is a GitHub server's `create_pull_request`: `mcp__<server>__<tool>`, as
/// Claude Code names them, from a server whose name says GitHub.
pub fn creates_pull_request_tool(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("mcp__") else {
        return false;
    };
    let Some((server, tool)) = rest.rsplit_once("__") else {
        return false;
    };
    is_create_pull_request(server, tool)
}

/// Whether a tool, by its server's name and its own, creates a GitHub pull request: one whose
/// name ends `create_pull_request`, where GitHub is in the server's name or the tool's. Each
/// agent names MCP tools its own way — Codex keeps the two apart (`github`,
/// `create_pull_request`, or its own `codex_apps`, `github_create_pull_request`); OpenCode joins
/// them (`github_create_pull_request`), so it passes an empty server.
pub fn is_create_pull_request(server: &str, tool: &str) -> bool {
    tool.ends_with("create_pull_request")
        && (server.to_ascii_lowercase().contains("github")
            || tool.to_ascii_lowercase().contains("github"))
}

/// The pull request a shell command created: the last link in what the command printed, when
/// the command runs `gh pr create` ([`runs_pr_create`]). The rule every adapter applies to a
/// shell tool; what the command printed is read for the link alone.
pub fn created_by_command(command: &str, output: &str) -> Option<String> {
    if runs_pr_create(command) {
        last_link_in(output)
    } else {
        None
    }
}

/// The new pull request's link in what a create-pull-request tool answered, which is GitHub's
/// JSON, often as text inside the tool's content blocks. Its `html_url` when it has one — a full
/// answer names other pull requests too, in its description — then a `url` that is a pull
/// request's, then the first link anywhere.
pub fn link_in_tool_result(result: &Value) -> Option<String> {
    let mut strings = Vec::new();
    collect(result, None, &mut strings);
    let keyed = |key: &str| {
        strings
            .iter()
            .filter(|(at, _)| at.as_deref() == Some(key))
            .find_map(|(_, text)| link_in(text))
    };
    keyed("html_url")
        .or_else(|| keyed("url"))
        .or_else(|| strings.iter().find_map(|(_, text)| link_in(text)))
}

/// Every string in `value` with the key it is under, looking inside strings that are JSON.
fn collect(value: &Value, key: Option<&str>, into: &mut Vec<(Option<String>, String)>) {
    match value {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(inner @ (Value::Object(_) | Value::Array(_))) => collect(&inner, key, into),
            _ => into.push((key.map(str::to_owned), text.clone())),
        },
        Value::Array(items) => items.iter().for_each(|item| collect(item, key, into)),
        Value::Object(fields) => fields
            .iter()
            .for_each(|(name, field)| collect(field, Some(name), into)),
        _ => {}
    }
}

/// A pull request an agent was seen creating, from an event drained just now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub url: String,
}

/// Track the pull requests agents were seen creating, by `hook`. One in the project's own
/// repository is left alone — its list has it already — and so is anything else [`track`]
/// refuses; the event stays either way. Without `git` the project's own repository cannot be
/// told apart, so nothing is tracked and the shortfall is counted. Returns the workspaces that
/// gained a pull request.
pub fn track_seen(store: &Store, git: Option<&Git>, seen: &[Seen]) -> BTreeSet<String> {
    let mut gained = BTreeSet::new();
    if seen.is_empty() {
        return gained;
    }
    let Some(git) = git else {
        let _ = store.bump_diagnostic("pull_request_seen_no_git", Some(&seen.len().to_string()));
        return gained;
    };
    for one in seen {
        match track(
            store,
            git,
            &one.workspace_id,
            &one.url,
            one.session_id.as_deref(),
            TrackedBy::Hook,
        ) {
            Ok((_, true)) => {
                gained.insert(one.workspace_id.clone());
            }
            Ok((_, false)) => {}
            Err(error) if error.code == "own_pull_request" => {}
            Err(error) => {
                let _ = store.bump_diagnostic("pull_request_seen_refused", Some(&error.message));
            }
        }
    }
    gained
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
    fn a_pull_requests_nested_clone_is_found_by_its_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (name, remote) in [
            ("app", "git@github.com:o/app.git"),
            ("lib", "https://github.com/O/Lib.git"),
            ("fork", "git@github.example.com:o/lib.git"),
        ] {
            let clone = root.join(name);
            std::fs::create_dir_all(&clone).unwrap();
            git().run(&clone, &["init", "-q"]).unwrap();
            git()
                .run(&clone, &["remote", "add", "origin", remote])
                .unwrap();
        }
        let repo = |host: &str, owner: &str, name: &str| crate::forge::Repo {
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
            kind: ForgeKind::GitHub,
        };
        assert_eq!(
            clone_of(&git(), root, &repo("github.com", "o", "lib")),
            Some(root.join("lib")),
            "by its remote, whatever the case"
        );
        assert_eq!(
            clone_of(&git(), root, &repo("github.example.com", "o", "lib")),
            Some(root.join("fork")),
            "the host counts"
        );
        assert_eq!(
            clone_of(&git(), root, &repo("github.com", "o", "other")),
            None
        );
        // A remote through an SSH host alias names no forge's host: found by its name.
        let alias = root.join("alias");
        std::fs::create_dir_all(&alias).unwrap();
        git().run(&alias, &["init", "-q"]).unwrap();
        git()
            .run(
                &alias,
                &["remote", "add", "origin", "git@github-work:o/tool.git"],
            )
            .unwrap();
        assert_eq!(
            clone_of(&git(), root, &repo("github.com", "o", "tool")),
            Some(alias)
        );
    }

    #[test]
    fn nested_clones_are_named_in_the_launch_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("parent");
        let worktree = dir.path().join("worktrees/fix");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        assert_eq!(
            launch_section(&root, &worktree),
            None,
            "nothing nested, nothing said"
        );
        std::fs::create_dir_all(root.join("lib/.git")).unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/.git"), "gitdir: elsewhere").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join(".hidden/.git")).unwrap();
        std::fs::write(root.join("file"), "").unwrap();
        assert_eq!(nested_repositories(&root), ["app", "lib"]);
        let paths = format!(
            "`{}`, `{}`.",
            root.join("app").display(),
            root.join("lib").display()
        );

        // From a worktree, which is elsewhere and has none of them: each by its whole path, said
        // to be outside, with a worktree of its own to work in rather than the shared clone.
        let section = launch_section(&root, &worktree).unwrap();
        assert!(section.starts_with("## Other repositories\n\n"));
        assert!(
            section.contains(&format!(
                "which are not in this workspace's folder: {paths}"
            )),
            "{section}"
        );
        assert!(
            section.contains("make a worktree of it for yourself"),
            "{section}"
        );
        assert!(section.contains("`ys pr track <its url>`"));

        // From the project's own checkout — its local workspace — they are folders of its own.
        let local = launch_section(&root, &root).unwrap();
        assert!(!local.contains("not in this workspace's folder"), "{local}");
        assert!(local.contains(&format!("repositories: {paths}")), "{local}");

        for n in 0..12 {
            std::fs::create_dir_all(root.join(format!("r{n:02}/.git"))).unwrap();
        }
        assert!(
            launch_section(&root, &worktree)
                .unwrap()
                .contains(&format!(", and 4 more in `{}`.", root.display())),
            "the rest are still somewhere to look"
        );
    }

    #[test]
    fn a_path_with_a_backtick_stays_one_code_span() {
        assert_eq!(code("/home/u/repo"), "`/home/u/repo`");
        assert_eq!(code("/home/u/my`repo"), "``/home/u/my`repo``");
        assert_eq!(code("`odd"), "`` `odd ``");
    }

    #[test]
    fn a_link_is_found_in_what_a_command_printed() {
        let found = |text: &str| link_in(text);
        assert_eq!(
            found("Creating pull request for bump into main in o/lib\n\nhttps://github.com/o/lib/pull/41\n")
                .as_deref(),
            Some("https://github.com/o/lib/pull/41")
        );
        assert_eq!(
            found("Opened (https://github.com/o/lib/pull/41/files).").as_deref(),
            Some("https://github.com/o/lib/pull/41")
        );
        assert_eq!(
            found("see \"https://github.example.com/t/svc/pull/3\"").as_deref(),
            Some("https://github.example.com/t/svc/pull/3")
        );
        for nothing in [
            "",
            "remote: Create a pull request for 'bump' on GitHub by visiting:\nremote:   https://github.com/o/lib/pull/new/bump",
            "https://api.github.com/repos/o/lib/pulls/41",
            "https://github.com/o/lib/issues/41",
            "http",
            "httpx://github.com/o/lib/pull/41",
        ] {
            assert_eq!(found(nothing), None, "{nothing}");
        }
    }

    #[test]
    fn a_command_that_runs_gh_pr_create_is_told_apart() {
        for yes in [
            "gh pr create --fill",
            "cd lib && gh pr create --title \"Bump\" --body \"x\"",
            "GH_REPO=o/lib gh pr create --fill",
            "/usr/local/bin/gh pr create",
            "gh -R o/lib pr create",
            "gh pr --repo o/lib create --fill",
            "git push -u origin bump; gh pr create --fill | tee out",
            "(cd lib\ngh pr create --draft)",
            "'C:\\tools\\gh.exe' pr create",
            "C:/tools/gh.exe pr create",
            "env -u GH_TOKEN gh pr create",
            "sudo -E gh pr create",
            "if gh pr create --fill; then echo ok; fi",
            "! gh pr create --fill",
            "timeout 60 gh pr create --fill",
            "timeout -k 5 60s gh pr create --fill",
            "nice -n 10 gh pr create",
            "sudo -n gh pr create",
            "while true; do gh pr create --fill && break; done",
            // Separators in quotes are the title's, not the shell's.
            "gh pr create --title 'Bump; then | more' --body \"a && b\"",
            "gh pr \\\n  create --fill",
            "gh pr create --body-file - <<'EOF'\nFixes https://github.com/o/lib/pull/7\nEOF",
            "cat <<EOF > body.md\nnotes; gh pr view 7\nEOF\ngh pr create --body-file body.md",
            "cat <<-EOF > body.md\n\tnotes\n\tEOF\ngh pr create --fill",
        ] {
            assert!(runs_pr_create(yes), "{yes}");
        }
        for no in [
            "gh pr view 7",
            "gh pr list --search create",
            "gh issue create",
            "echo gh pr create",
            "ghq pr create",
            "git commit -m 'gh pr create later'",
            "xargs -I{} echo gh pr create",
            "",
            // Text that only mentions it, whatever separators it holds.
            "printf '%s\\n' 'example; gh pr create --body https://github.com/o/lib/pull/7'",
            "echo \"done && gh pr create\"",
            "cat <<'EOF'\nRun: gh pr create --body https://github.com/o/lib/pull/7\nEOF",
            "cat <<EOF\nfirst\ngh pr create --fill\nEOF\necho done",
            "git status # then; gh pr create",
            "timeout 60 echo gh pr create",
            "if echo gh pr create; then :; fi",
        ] {
            assert!(!runs_pr_create(no), "{no}");
        }
    }

    #[test]
    fn what_a_command_printed_ends_with_the_link_it_created() {
        let printed = "Follows https://github.com/o/lib/pull/3\n\nCreating pull request\n\nhttps://github.com/o/lib/pull/41\n";
        assert_eq!(
            last_link_in(printed).as_deref(),
            Some("https://github.com/o/lib/pull/41")
        );
        assert_eq!(
            link_in(printed).as_deref(),
            Some("https://github.com/o/lib/pull/3")
        );
        assert_eq!(last_link_in("nothing here"), None);
    }

    #[test]
    fn only_a_github_servers_create_pull_request_tool_counts() {
        assert!(creates_pull_request_tool(
            "mcp__github__create_pull_request"
        ));
        assert!(creates_pull_request_tool(
            "mcp__claude_ai_GitHub__create_pull_request"
        ));
        assert!(!creates_pull_request_tool(
            "mcp__github__update_pull_request"
        ));
        assert!(!creates_pull_request_tool(
            "mcp__gitea__create_pull_request"
        ));
        assert!(!creates_pull_request_tool("create_pull_request"));
        assert!(!creates_pull_request_tool("mcp__github"));
        // The other agents' names: Codex's server and tool apart, OpenCode's joined.
        assert!(is_create_pull_request("github", "create_pull_request"));
        assert!(is_create_pull_request(
            "codex_apps",
            "github_create_pull_request"
        ));
        assert!(is_create_pull_request("", "github_create_pull_request"));
        assert!(!is_create_pull_request("", "create_pull_request"));
        assert!(!is_create_pull_request(
            "github",
            "create_pull_request_review"
        ));
        assert!(!is_create_pull_request("gitlab", "create_pull_request"));
    }

    #[test]
    fn a_command_creating_one_gives_the_link_it_printed() {
        let printed = "\nCreating pull request for bump into main in o/lib\n\nhttps://github.com/o/lib/pull/41\n";
        assert_eq!(
            created_by_command("gh pr create --fill", printed).as_deref(),
            Some("https://github.com/o/lib/pull/41")
        );
        assert_eq!(created_by_command("gh pr view 41", printed), None);
        assert_eq!(
            created_by_command("gh pr create --fill", "no git remotes found"),
            None
        );
    }

    #[test]
    fn a_tools_answer_gives_its_own_link_before_any_it_mentions() {
        let minimal = serde_json::json!([{ "type": "text",
            "text": "{\"id\":\"1\",\"url\":\"https://github.com/o/lib/pull/42\"}" }]);
        assert_eq!(
            link_in_tool_result(&minimal).as_deref(),
            Some("https://github.com/o/lib/pull/42")
        );
        let full = serde_json::json!({
            "body": "Supersedes https://github.com/o/lib/pull/3",
            "url": "https://api.github.com/repos/o/lib/pulls/9",
            "html_url": "https://github.com/o/lib/pull/9"
        });
        assert_eq!(
            link_in_tool_result(&full).as_deref(),
            Some("https://github.com/o/lib/pull/9")
        );
        let prose = serde_json::json!("Created https://github.com/o/lib/pull/5 for you.");
        assert_eq!(
            link_in_tool_result(&prose).as_deref(),
            Some("https://github.com/o/lib/pull/5")
        );
        assert_eq!(
            link_in_tool_result(&serde_json::json!({ "ok": true })),
            None
        );
    }

    #[test]
    fn what_was_seen_is_tracked_by_hook_except_the_projects_own() {
        let (store, ws, _dir) = project_on("git@github.com:o/parent.git");
        let seen = |url: &str| Seen {
            workspace_id: ws.id.clone(),
            session_id: Some("no-such-record".into()),
            url: url.into(),
        };
        let all = [
            seen("https://github.com/o/lib/pull/7"),
            seen("https://github.com/o/parent/pull/3"),
            seen("https://github.com/o/lib/pull/7"),
        ];

        // No git: the project's own cannot be told apart, so nothing is written.
        assert!(track_seen(&store, None, &all).is_empty());
        assert!(store.tracked_pull_requests(&ws.id).unwrap().is_empty());

        let gained = track_seen(&store, Some(&git()), &all);
        assert_eq!(gained, std::collections::BTreeSet::from([ws.id.clone()]));
        let rows = store.tracked_pull_requests(&ws.id).unwrap();
        assert_eq!(rows.len(), 1, "once, and not the project's own");
        assert_eq!(rows[0].url, "https://github.com/o/lib/pull/7");
        assert_eq!(rows[0].tracked_by, "hook");
        assert_eq!(
            rows[0].session_id, None,
            "a record not of this workspace is dropped"
        );

        // Seen again: nothing new.
        assert!(track_seen(&store, Some(&git()), &all[..1]).is_empty());
    }
}

//! Git on this computer, as its agents will use it: a name for their commits, and the GitHub
//! CLI signed in, which carries both their pushes and their pull requests.
//!
//! Everything is `git` and `gh` themselves, run as the user's login shell would find them. The
//! dashboard keeps nothing of its own: a token goes to `gh` on its input and is `gh`'s to keep.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use yardsort_core::env::ShellEnv;
use yardsort_core::program::Program;

use crate::jobs::Step;

/// The one forge the pull request features speak to.
const GITHUB: &str = "github.com";
/// How long `git` or `gh` gets to answer a question. `gh auth status` asks GitHub, and a
/// computer with no network must not hold the page.
const ANSWER_WITHIN: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Git {
    /// `git version 2.43.0`, or `None` when there is no git.
    pub version: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub github: GitHub,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHub {
    /// `gh version 2.60.0 (2024-10-23)`, or `None` when there is no GitHub CLI.
    pub version: Option<String>,
    pub signed_in: bool,
    /// What `gh auth status` said, as it said it: the account, how it signs in, what the token
    /// may do. `gh` never prints the token itself there.
    pub detail: String,
}

/// How Git stands for this user.
pub fn read(env: &ShellEnv) -> Git {
    let home = home(env);
    let git = Program::find(env, "git");
    let ask = |args: &[&str]| -> Option<String> {
        let mut command = git.as_ref()?.command(&home);
        command.args(args);
        answer(command).filter(|a| a.ok).map(|a| a.text)
    };
    let github = match Program::find(env, "gh") {
        None => GitHub {
            version: None,
            signed_in: false,
            detail: String::new(),
        },
        Some(gh) => {
            let run = |args: &[&str]| {
                let mut command = quiet(gh.command(&home));
                command.args(args);
                answer(command)
            };
            let status = run(&["auth", "status", "--hostname", GITHUB]);
            GitHub {
                version: run(&["--version"])
                    .filter(|a| a.ok)
                    .and_then(|a| a.text.lines().next().map(str::to_owned)),
                signed_in: status.as_ref().is_some_and(|a| a.ok),
                detail: status.map(|a| a.text).unwrap_or_else(|| {
                    "The GitHub CLI did not answer in time. Is GitHub reachable from here?".into()
                }),
            }
        }
    };
    Git {
        version: ask(&["--version"]),
        name: ask(&["config", "--global", "--get", "user.name"]).filter(|s| !s.is_empty()),
        email: ask(&["config", "--global", "--get", "user.email"]).filter(|s| !s.is_empty()),
        github,
    }
}

/// Set the name and email this user's commits carry.
pub fn set_identity(env: &ShellEnv, name: &str, email: &str) -> Result<(), String> {
    let (name, email) = (name.trim(), email.trim());
    for (what, value) in [("name", name), ("email", email)] {
        if value.is_empty() {
            return Err(format!("A commit needs a {what}."));
        }
        if value.len() > 200 || value.chars().any(char::is_control) {
            return Err(format!("That {what} is not one Git can keep."));
        }
    }
    let git = Program::find(env, "git").ok_or("There is no git on this user's PATH.")?;
    let home = home(env);
    for (key, value) in [("user.name", name), ("user.email", email)] {
        let mut command = git.command(&home);
        command.args(["config", "--global", key, value]);
        match answer(command) {
            Some(answer) if answer.ok => {}
            Some(answer) => return Err(format!("git could not keep it: {}", answer.text)),
            None => return Err("git did not answer.".into()),
        }
    }
    Ok(())
}

/// Sign the GitHub CLI in from a browser on any computer: it prints a one-time code and an
/// address, and waits until the code is typed there. Git is told to push through it.
pub fn sign_in(env: &ShellEnv) -> Result<Vec<Step>, String> {
    let gh = gh(env)?;
    let mut command = quiet(gh.command(&home(env)));
    command.args([
        "auth",
        "login",
        "--hostname",
        GITHUB,
        "--git-protocol",
        "https",
        "--web",
    ]);
    // There is no browser to open here, or it is not the one the person is looking at.
    command.env("BROWSER", "false");
    Ok(vec![Step::new(command).shown(format!(
        "gh auth login --hostname {GITHUB} --git-protocol https --web"
    ))])
}

/// Sign the GitHub CLI in with a token made at GitHub, which goes in on its input and into no
/// command line, and tell Git to push through it.
pub fn sign_in_with_token(env: &ShellEnv, token: &str) -> Result<Vec<Step>, String> {
    let token = token.trim();
    if token.is_empty()
        || token.len() > 400
        || token.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("That is not a token: paste it as GitHub showed it, alone.".into());
    }
    let gh = gh(env)?;
    let home = home(env);
    let mut login = quiet(gh.command(&home));
    login.args(["auth", "login", "--hostname", GITHUB, "--with-token"]);
    let mut setup = quiet(gh.command(&home));
    setup.args(["auth", "setup-git", "--hostname", GITHUB]);
    Ok(vec![
        Step::new(login)
            .shown(format!("gh auth login --hostname {GITHUB} --with-token"))
            .input(format!("{token}\n")),
        Step::new(setup).shown(format!("gh auth setup-git --hostname {GITHUB}")),
    ])
}

/// Sign the GitHub CLI out here. The token is still good at GitHub until it is deleted there.
pub fn sign_out(env: &ShellEnv) -> Result<Vec<Step>, String> {
    let gh = gh(env)?;
    let mut command = quiet(gh.command(&home(env)));
    command.args(["auth", "logout", "--hostname", GITHUB]);
    Ok(vec![
        Step::new(command).shown(format!("gh auth logout --hostname {GITHUB}"))
    ])
}

fn gh(env: &ShellEnv) -> Result<Program, String> {
    Program::find(env, "gh").ok_or_else(|| {
        "There is no GitHub CLI (gh) on this user's PATH. Install it as \
         https://github.com/cli/cli#installation says, then look again."
            .into()
    })
}

fn home(env: &ShellEnv) -> std::path::PathBuf {
    env.home_dir().unwrap_or_else(std::env::temp_dir)
}

/// `gh` told that nobody is there to answer it, and to print plain text.
fn quiet(mut command: Command) -> Command {
    command
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0");
    command
}

struct Answer {
    ok: bool,
    /// What it printed, its errors after its output, trimmed.
    text: String,
}

/// Run `command` for what it prints. `None` when it could not be run, or did not end in time,
/// when it is ended.
fn answer(mut command: Command) -> Option<Answer> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let since = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if since.elapsed() < ANSWER_WITHIN => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let (out, err) = (out.join().ok()?, err.join().ok()?);
    let text = [out.trim(), err.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Answer {
        ok: status.success(),
        text,
    })
}

/// A program's version, for the agents list: the first line it prints for `--version`.
pub fn version_of(env: &ShellEnv, program: &Path) -> Option<String> {
    let mut command = Program::at(program.to_path_buf(), env).command(&home(env));
    command.arg("--version");
    answer(command)
        .filter(|a| a.ok)
        .and_then(|a| a.text.lines().next().map(|line| line.trim().to_owned()))
        .filter(|line| !line.is_empty())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use yardsort_core::env::EnvSource;
    use yardsort_core::program::testing::executable;

    /// A home of its own, with the system's git and whatever `bin` holds.
    fn env(home: &Path, bin: &Path) -> ShellEnv {
        ShellEnv {
            vars: BTreeMap::from([
                ("HOME".to_owned(), home.display().to_string()),
                (
                    "PATH".to_owned(),
                    format!("{}:/usr/bin:/bin", bin.display()),
                ),
                // Nothing of the machine's own git setup reaches the test, nor the other way.
                ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
                (
                    "XDG_CONFIG_HOME".to_owned(),
                    home.join(".config").display().to_string(),
                ),
            ]),
            source: EnvSource::Process,
            warning: None,
        }
    }

    #[test]
    fn a_new_home_has_no_identity_and_setting_one_is_what_git_then_says() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let env = env(dir.path(), &bin);
        let before = read(&env);
        assert!(before
            .version
            .as_deref()
            .is_some_and(|v| v.starts_with("git version")));
        assert_eq!((before.name, before.email), (None, None));
        // Whether there is a GitHub CLI is the machine's to say here (a CI runner has one in
        // /usr/bin); in a new home, nobody is signed in to it either way.
        assert!(!before.github.signed_in);

        set_identity(&env, "  Build Box ", "box@example.invalid").unwrap();
        let after = read(&env);
        assert_eq!(after.name.as_deref(), Some("Build Box"));
        assert_eq!(after.email.as_deref(), Some("box@example.invalid"));
        // In this home's own file, and nowhere else.
        let kept = std::fs::read_to_string(dir.path().join(".gitconfig")).unwrap();
        assert!(kept.contains("name = Build Box"), "{kept}");
    }

    #[test]
    fn an_identity_git_could_not_keep_is_refused_before_git_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), dir.path());
        assert!(set_identity(&env, "", "a@example.invalid")
            .unwrap_err()
            .contains("needs a name"));
        assert!(set_identity(&env, "A", " ")
            .unwrap_err()
            .contains("needs a email"));
        assert!(set_identity(&env, "A\nB", "a@example.invalid").is_err());
        assert!(!dir.path().join(".gitconfig").exists());
    }

    #[test]
    fn the_github_cli_is_asked_whether_it_is_signed_in_and_its_words_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        executable(
            bin.join("gh"),
            "#!/bin/sh\ncase \"$*\" in\n  --version) echo 'gh version 9.9.9 (stand-in)'; echo more;;\n  \
             'auth status --hostname github.com') test -e \"$HOME/signed-in\" || { echo 'You are not logged into any GitHub hosts.' >&2; exit 1; }\n    \
             echo 'github.com'; echo '  Logged in to github.com account build-box';;\n  *) exit 9;;\nesac\n",
        );
        let env = env(dir.path(), &bin);
        let out = read(&env).github;
        assert_eq!(out.version.as_deref(), Some("gh version 9.9.9 (stand-in)"));
        assert!(!out.signed_in);
        assert!(out.detail.contains("not logged into"), "{}", out.detail);
        std::fs::write(dir.path().join("signed-in"), "").unwrap();
        let signed_in = read(&env).github;
        assert!(signed_in.signed_in);
        assert!(
            signed_in.detail.contains("account build-box"),
            "{}",
            signed_in.detail
        );
    }

    #[test]
    fn a_token_is_given_on_the_input_and_is_in_no_argument() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        executable(bin.join("gh"), "#!/bin/sh\nexit 0\n");
        let env = env(dir.path(), &bin);
        let steps = sign_in_with_token(&env, " ghp_example123 \n").unwrap();
        let [login, setup] = steps.as_slice() else {
            panic!("the login, then git's setup");
        };
        assert_eq!(login.input.as_deref(), Some("ghp_example123\n"));
        let arguments: Vec<_> = login
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            ["auth", "login", "--hostname", "github.com", "--with-token"]
        );
        assert!(!login.shown.contains("ghp_"));
        assert!(setup.shown.contains("setup-git"));
        for not_a_token in ["", "two words", "line\nbreak"] {
            assert!(
                sign_in_with_token(&env, not_a_token).is_err(),
                "{not_a_token:?}"
            );
        }
    }

    #[test]
    fn without_the_github_cli_there_is_none_to_read_and_signing_in_says_where_to_get_it() {
        let dir = tempfile::tempdir().unwrap();
        let empty = ShellEnv {
            vars: BTreeMap::from([("PATH".to_owned(), dir.path().display().to_string())]),
            source: EnvSource::Process,
            warning: None,
        };
        let there = read(&empty);
        assert_eq!((there.version, there.github.version), (None, None));
        assert!(!there.github.signed_in);
        let Err(why) = sign_in(&empty) else {
            panic!("no gh there");
        };
        assert!(why.contains("github.com/cli/cli"), "{why}");
    }
}

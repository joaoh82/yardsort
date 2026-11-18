//! The agents a host can be given: where each comes from, and how it is installed into the
//! operator's `~/.local`, as the operator.
//!
//! Yardsort hands on a copy only of a harness whose licence lets anyone redistribute it; the full
//! image carries those (`crates/host/agents/`). For the others it runs the vendor's own installer,
//! so that whoever installs one accepts that vendor's terms there and then. The ones the image
//! carries can be brought up to date between releases the same way, from npm, into `~/.local`,
//! which comes first on the image's `PATH`.
//!
//! `yardsort-host install-agent` and the dashboard's **Install** are the same steps, made here:
//! one prints them at a terminal, the other into a job.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where a harness comes from.
pub enum Source {
    /// The vendor's installer script, run with `bash` as it says to.
    Installer {
        url: &'static str,
        vendor: &'static str,
        /// Where the vendor's terms are, to be named before the installer runs.
        terms: &'static str,
        /// Told to the installer, so that it installs where `~/.local/bin` is.
        env: &'static [(&'static str, &'static str)],
    },
    /// An npm package whose licence allows redistribution; the full image ships it already.
    Npm { package: &'static str },
}

pub struct Harness {
    /// Yardsort's id for it, as in Settings and `--harness`.
    pub id: &'static str,
    pub name: &'static str,
    /// What the installer leaves in `~/.local/bin`.
    pub command: &'static str,
    pub source: Source,
}

pub const HARNESSES: &[Harness] = &[
    Harness {
        id: "claude",
        name: "Claude Code",
        command: "claude",
        source: Source::Installer {
            url: "https://claude.ai/install.sh",
            vendor: "Anthropic",
            terms: "https://www.anthropic.com/legal/commercial-terms",
            env: &[],
        },
    },
    Harness {
        id: "cursor",
        name: "Cursor's CLI",
        command: "cursor-agent",
        source: Source::Installer {
            url: "https://cursor.com/install",
            vendor: "Cursor",
            terms: "https://cursor.com/terms-of-service",
            env: &[],
        },
    },
    Harness {
        id: "grok",
        name: "Grok",
        command: "grok",
        source: Source::Installer {
            url: "https://x.ai/cli/install.sh",
            vendor: "xAI",
            terms: "https://x.ai/cli",
            // It tries /usr/local/bin too, which only root may write in the image.
            env: &[("GROK_BIN_DIR", "$HOME/.local/bin")],
        },
    },
    Harness {
        id: "codex",
        name: "Codex CLI",
        command: "codex",
        source: Source::Npm {
            package: "@openai/codex",
        },
    },
    Harness {
        id: "opencode",
        name: "OpenCode",
        command: "opencode",
        source: Source::Npm {
            package: "@opencode/cli",
        },
    },
    Harness {
        id: "pi",
        name: "Pi",
        command: "pi",
        source: Source::Npm {
            package: "@earendil-works/pi-coding-agent",
        },
    },
    Harness {
        id: "omp",
        name: "Oh My Pi",
        command: "omp",
        source: Source::Npm {
            package: "@oh-my-pi/pi-coding-agent",
        },
    },
];

pub fn find(id: &str) -> Result<&'static Harness, String> {
    HARNESSES.iter().find(|h| h.id == id).ok_or_else(|| {
        let ids: Vec<&str> = HARNESSES.iter().map(|h| h.id).collect();
        format!("No harness called {id}. One of: {}.", ids.join(", "))
    })
}

/// One program of an install, as an argv array.
pub struct Run {
    pub program: &'static str,
    pub args: Vec<OsString>,
    /// Added to the environment it runs in.
    pub env: Vec<(String, String)>,
    /// What to say when the program cannot be started at all.
    pub needs: String,
    /// What to say when it ran and failed, with `{status}` for how it ended.
    pub failed: String,
}

impl Run {
    /// The line a shell would have been given, for showing. Never run.
    pub fn shown(&self) -> String {
        std::iter::once(self.program.to_owned())
            .chain(
                self.args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned()),
            )
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How one harness is installed: what to say first, the programs to run in turn, and what they
/// leave that must not stay.
pub struct Install {
    pub harness: &'static Harness,
    /// Said before anything runs. For a vendor's installer it names the vendor and their terms.
    pub notice: String,
    pub runs: Vec<Run>,
    /// The installer that was downloaded, to delete once it has run.
    pub downloaded: Option<PathBuf>,
    /// `~/.local/bin`, where the harness lands.
    pub bin: PathBuf,
}

/// Plan installing `id` into `home`'s `.local`. With `installer`, that file is run instead of
/// the vendor's being downloaded: one fetched and read beforehand, or a mirror's. Nothing is
/// made or fetched here but `~/.local/bin` itself.
pub fn install(id: &str, home: &Path, installer: Option<PathBuf>) -> Result<Install, String> {
    let harness = find(id)?;
    if !cfg!(unix) {
        return Err(format!(
            "{} is installed here as its own guide says: the installers Yardsort runs are for \
             Linux and macOS.",
            harness.name
        ));
    }
    if is_root() {
        return Err(format!(
            "Run it as the user that runs Yardsort, not as root: {} is installed into that \
             user's home, where the agents it starts will look for it.",
            harness.name
        ));
    }
    // Before anything is made or fetched: an installer asked for and not used would be the
    // network instead of what the operator read.
    if let Some(path) = &installer {
        if let Source::Npm { package } = harness.source {
            return Err(format!(
                "{} comes from npm, as {package}, not from an installer script: --installer is \
                 for claude, cursor and grok.",
                harness.name
            ));
        }
        if !path.is_file() {
            return Err(format!("No installer at {}.", path.display()));
        }
    }
    let local = home.join(".local");
    let bin = local.join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;

    Ok(match &harness.source {
        Source::Installer {
            url,
            vendor,
            terms,
            env,
        } => {
            let env: Vec<(String, String)> = env
                .iter()
                .map(|(key, value)| {
                    (
                        (*key).to_owned(),
                        value.replace("$HOME", &home.to_string_lossy()),
                    )
                })
                .collect();
            let bash = |script: &Path| Run {
                program: "bash",
                args: vec![script.into()],
                env: env.clone(),
                needs: "Running the installer needs bash".into(),
                failed: format!("{vendor}'s installer failed ({{status}}); its output is above."),
            };
            match installer {
                Some(path) => Install {
                    harness,
                    notice: format!(
                        "Installing {} with the installer {}.",
                        harness.name,
                        path.display()
                    ),
                    runs: vec![bash(&path)],
                    downloaded: None,
                    bin,
                },
                None => {
                    let path = std::env::temp_dir().join(format!(
                        "yardsort-install-{}-{}-{}.sh",
                        harness.id,
                        std::process::id(),
                        yardsort_core::store::now_ms()
                    ));
                    Install {
                        harness,
                        notice: format!(
                            "Installing {} with {vendor}'s own installer, {url}. It is {vendor}'s \
                             software, under {vendor}'s terms ({terms}): installing it is \
                             accepting them.",
                            harness.name
                        ),
                        runs: vec![
                            Run {
                                program: "curl",
                                args: ["-fsSL", "--proto", "=https", "--tlsv1.2", "-o"]
                                    .into_iter()
                                    .map(OsString::from)
                                    .chain([path.clone().into_os_string(), (*url).into()])
                                    .collect(),
                                env: Vec::new(),
                                needs: "Downloading the installer needs curl".into(),
                                failed: format!("Could not download {url} ({{status}})."),
                            },
                            bash(&path),
                        ],
                        downloaded: Some(path),
                        bin,
                    }
                }
            }
        }
        Source::Npm { package } => Install {
            harness,
            notice: format!(
                "Installing {} from npm, {package}, into {}.",
                harness.name,
                local.display()
            ),
            runs: vec![Run {
                program: "npm",
                args: ["install", "--global", "--no-audit", "--no-fund", "--prefix"]
                    .into_iter()
                    .map(OsString::from)
                    .chain([local.into_os_string(), format!("{package}@latest").into()])
                    .collect(),
                env: Vec::new(),
                needs: format!(
                    "Installing {} needs npm, from Node.js 22 or later",
                    harness.name
                ),
                failed: "npm failed ({status}); its output is above.".into(),
            }],
            downloaded: None,
            bin,
        },
    })
}

/// What to say once an install has run: what was installed, and whether agents will find it.
/// `path` is the `PATH` agents are started with, and `version` runs a program for its version.
pub fn report(
    install: &Install,
    path: Option<&std::ffi::OsStr>,
    version: impl FnOnce(&Path) -> Option<String>,
) -> Vec<String> {
    let harness = install.harness;
    let program = install.bin.join(harness.command);
    if !program.exists() {
        return vec![format!(
            "Done, but there is no {}: the installer put it somewhere else. Its output above says where.",
            program.display()
        )];
    }
    let mut lines = vec![match version(&program) {
        Some(version) => format!("Installed {}: {version}.", program.display()),
        None => format!("Installed {}.", program.display()),
    }];
    let on_path =
        path.is_some_and(|path| std::env::split_paths(path).any(|dir| dir == install.bin));
    if !on_path {
        lines.push(format!(
            "{} is not on PATH here. Agents are started from your login shell's environment: add \
             it in its profile, as Ubuntu and Debian do once the folder exists.",
            install.bin.display()
        ));
    }
    lines.push(format!(
        "Sign {} in as its own guide says, as this user, before starting it with `ys`.",
        harness.name
    ));
    lines
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_harness_is_one_yardsort_knows() {
        for harness in HARNESSES {
            let known = yardsort_core::harness::find(harness.id, &[])
                .unwrap_or_else(|| panic!("{} is not a harness", harness.id));
            assert_eq!(known.command, harness.command, "{}", harness.id);
        }
    }

    #[test]
    fn an_installer_is_the_vendors_own_over_https() {
        for harness in HARNESSES {
            let Source::Installer { url, terms, .. } = &harness.source else {
                continue;
            };
            let domain = match harness.id {
                "claude" => "https://claude.ai/",
                "cursor" => "https://cursor.com/",
                "grok" => "https://x.ai/",
                other => panic!("an installer for {other}, from whom?"),
            };
            assert!(url.starts_with(domain), "{url}");
            assert!(terms.starts_with("https://"), "{terms}");
        }
    }

    #[test]
    fn an_unknown_harness_names_the_known_ones() {
        let Err(why) = find("aider") else {
            panic!("aider is not one");
        };
        assert!(why.contains("claude") && why.contains("omp"), "{why}");
    }

    #[cfg(unix)]
    #[test]
    fn a_vendors_installer_is_downloaded_over_https_and_then_run_and_its_terms_are_named() {
        if is_root() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let plan = install("grok", home.path(), None).unwrap();
        assert!(
            plan.notice.contains("xAI's terms (https://x.ai/cli)"),
            "{}",
            plan.notice
        );
        let [download, run] = plan.runs.as_slice() else {
            panic!("a download, then the installer");
        };
        let script = plan.downloaded.as_ref().expect("a downloaded installer");
        assert_eq!(download.program, "curl");
        assert!(
            download.shown().contains("--proto =https"),
            "{}",
            download.shown()
        );
        assert!(download.shown().ends_with("https://x.ai/cli/install.sh"));
        assert_eq!(run.program, "bash");
        assert_eq!(run.args, [script.clone().into_os_string()]);
        assert_eq!(
            run.env,
            [(
                "GROK_BIN_DIR".to_owned(),
                format!("{}/.local/bin", home.path().display())
            )]
        );
        assert!(home.path().join(".local/bin").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn an_npm_harness_goes_into_local_and_takes_no_installer() {
        if is_root() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let plan = install("codex", home.path(), None).unwrap();
        let [npm] = plan.runs.as_slice() else {
            panic!("npm alone");
        };
        assert_eq!(
            npm.shown(),
            format!(
                "npm install --global --no-audit --no-fund --prefix {}/.local @openai/codex@latest",
                home.path().display()
            )
        );
        let installer = home.path().join("installer.sh");
        std::fs::write(&installer, "exit 0\n").unwrap();
        let Err(why) = install("codex", home.path(), Some(installer)) else {
            panic!("an installer for an npm harness");
        };
        assert!(why.contains("comes from npm"), "{why}");
    }
}

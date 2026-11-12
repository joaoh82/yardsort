//! `yardsort-host install-agent`: a harness installed, or brought up to date, into the operator's
//! `~/.local`, as the operator.
//!
//! Yardsort hands on a copy only of a harness whose licence lets anyone redistribute it; the full
//! image carries those (`crates/host/agents/`). For the others it runs the vendor's own installer,
//! so that whoever installs one accepts that vendor's terms there and then. The ones the image
//! carries can be brought up to date between releases the same way, from npm, into `~/.local`,
//! which comes first on the image's `PATH`.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Install `id` into `~/.local`. With `installer`, run that file instead of downloading the
/// vendor's: one fetched and read beforehand, or a mirror's.
pub fn run(id: &str, installer: Option<PathBuf>) -> Result<(), String> {
    let harness = find(id)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return Err(format!(
            "Run it as the user that runs yardsort-host, not as root: {} is installed into that \
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
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set, so there is no ~/.local to install into.")?;
    let local = home.join(".local");
    let bin = local.join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;

    match &harness.source {
        Source::Installer {
            url,
            vendor,
            terms,
            env,
        } => {
            let (script, downloaded) = match installer {
                Some(path) => {
                    println!(
                        "Installing {} with the installer {}.",
                        harness.name,
                        path.display()
                    );
                    (path, false)
                }
                None => {
                    println!(
                        "Installing {} with {vendor}'s own installer, {url}. It is {vendor}'s \
                         software, under {vendor}'s terms ({terms}): installing it is accepting them.",
                        harness.name
                    );
                    let path = std::env::temp_dir().join(format!(
                        "yardsort-install-{}-{}.sh",
                        harness.id,
                        std::process::id()
                    ));
                    download(url, &path)?;
                    (path, true)
                }
            };
            let mut command = Command::new("bash");
            command.arg(&script);
            for (key, value) in env.iter() {
                command.env(key, value.replace("$HOME", &home.to_string_lossy()));
            }
            let status = command.status();
            if downloaded {
                let _ = std::fs::remove_file(&script);
            }
            let status = status.map_err(|e| format!("Could not run bash: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "{vendor}'s installer failed ({status}); its output is above."
                ));
            }
        }
        Source::Npm { package } => {
            println!(
                "Installing {} from npm, {package}, into {}.",
                harness.name,
                local.display()
            );
            let status = Command::new("npm")
                .args(["install", "--global", "--no-audit", "--no-fund", "--prefix"])
                .arg(&local)
                .arg(format!("{package}@latest"))
                .status()
                .map_err(|e| {
                    format!(
                        "Installing {} needs npm, from Node.js 22 or later: {e}",
                        harness.name
                    )
                })?;
            if !status.success() {
                return Err(format!("npm failed ({status}); its output is above."));
            }
        }
    }
    report(harness, &bin);
    Ok(())
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|e| format!("Downloading the installer needs curl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Could not download {url} ({status})."))
    }
}

/// Say what was installed, and whether agents will find it.
fn report(harness: &Harness, bin: &Path) {
    let program = bin.join(harness.command);
    if !program.exists() {
        println!(
            "Done, but there is no {}: the installer put it somewhere else. Its output above says where.",
            program.display()
        );
        return;
    }
    let version = Command::new(&program)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|text| !text.is_empty());
    match version {
        Some(version) => println!("Installed {}: {version}.", program.display()),
        None => println!("Installed {}.", program.display()),
    }
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir == bin));
    if !on_path {
        println!(
            "{} is not on PATH here. Agents are started from your login shell's environment: add \
             it in its profile, as Ubuntu and Debian do once the folder exists.",
            bin.display()
        );
    }
    println!(
        "Sign {} in as its own guide says, as this user, before starting it with `ys`.",
        harness.name
    );
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

    /// What npm installs is what the image ships, and nothing the image ships is installed by a
    /// vendor's script instead.
    #[test]
    fn the_npm_harnesses_are_the_images() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../agents/package.json"))
                .expect("agents/package.json");
        let mut shipped: Vec<&str> = manifest["dependencies"]
            .as_object()
            .expect("dependencies")
            .keys()
            .map(String::as_str)
            .collect();
        let mut npm: Vec<&str> = HARNESSES
            .iter()
            .filter_map(|h| match h.source {
                Source::Npm { package } => Some(package),
                Source::Installer { .. } => None,
            })
            .collect();
        shipped.sort_unstable();
        npm.sort_unstable();
        assert_eq!(shipped, npm);
    }

    #[test]
    fn an_unknown_harness_names_the_known_ones() {
        let Err(why) = find("aider") else {
            panic!("aider is not one");
        };
        assert!(why.contains("claude") && why.contains("omp"), "{why}");
    }
}

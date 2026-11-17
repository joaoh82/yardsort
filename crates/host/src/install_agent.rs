//! `yardsort-host install-agent`: a harness installed, or brought up to date, into the operator's
//! `~/.local`, as the operator.
//!
//! Where each harness comes from, and the programs that install it, are `host_dashboard::agents`:
//! the dashboard's **Install** runs the same ones. Here they run at the operator's terminal, with
//! their output going straight to it.

use std::path::{Path, PathBuf};
use std::process::Command;

use host_dashboard::agents;

/// Install `id` into `~/.local`. With `installer`, run that file instead of downloading the
/// vendor's: one fetched and read beforehand, or a mirror's.
pub fn run(id: &str, installer: Option<PathBuf>) -> Result<(), String> {
    // Looked up before `HOME`, so a name nobody knows is said to be that, whatever else is wrong.
    agents::find(id)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set, so there is no ~/.local to install into.")?;
    let plan = agents::install(id, &home, installer)?;
    println!("{}", plan.notice);
    let done = plan.runs.iter().try_for_each(|run| {
        let status = Command::new(run.program)
            .args(&run.args)
            .envs(run.env.iter().cloned())
            .status()
            .map_err(|e| format!("{}: {e}", run.needs))?;
        if status.success() {
            Ok(())
        } else {
            Err(run.failed.replace("{status}", &status.to_string()))
        }
    });
    if let Some(script) = &plan.downloaded {
        let _ = std::fs::remove_file(script);
    }
    done?;
    let path = std::env::var_os("PATH");
    for line in agents::report(&plan, path.as_deref(), version) {
        println!("{line}");
    }
    Ok(())
}

fn version(program: &Path) -> Option<String> {
    Command::new(program)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use host_dashboard::agents::{Source, HARNESSES};

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
}

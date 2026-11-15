//! `yardsort-host`: Yardsort on a computer with no screen, as a host that phones reach.
//!
//! The same host runtime as the desktop app's (`remote_host::account`, `yardsort_core::runtime`),
//! without a window: it keeps the profile's terminal daemon company and its records true, keeps
//! the link to a Yardsort server's relay, and lets the operator, at a terminal on this computer,
//! approve a device. Agents are started with `ys`. See `docs/guide/headless-host.md`.
//!
//! Linux is what it is for. It builds on macOS too; on Windows it is only a stub, so the workspace
//! builds everywhere.

#[cfg(unix)]
mod control;
#[cfg(unix)]
mod guard;
#[cfg(unix)]
mod home;
#[cfg(unix)]
mod install_agent;
#[cfg(unix)]
mod memory;
#[cfg(unix)]
mod pair;
#[cfg(unix)]
mod profile;
#[cfg(unix)]
mod serve;
#[cfg(unix)]
mod sign_in;
#[cfg(unix)]
mod sync;

fn main() {
    // Each of these can take over the process entirely: this executable doubles as the
    // environment probe, as the terminal daemon, and as the hook an agent runs to report what it
    // is doing, as the app's does. So a host's daemon is the same build as the host.
    yardsort_core::env::print_env_and_exit_if_asked();
    yardsort_core::daemon::run_daemon_and_exit_if_asked();
    yardsort_core::activity::hook::run_hook_and_exit_if_asked();
    #[cfg(unix)]
    std::process::exit(cli::main());
    #[cfg(not(unix))]
    {
        eprintln!("yardsort-host runs on Linux and macOS. On Windows, use the Yardsort app.");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod cli {
    use std::path::PathBuf;

    use clap::{Parser, Subcommand};

    use crate::control::{self, Ask, Reply};
    use crate::profile::Profile;

    #[derive(Parser)]
    #[command(
        name = "yardsort-host",
        version,
        about = "Yardsort on a computer with no screen, as a host your phone reaches",
        long_about = "Yardsort on a computer with no screen, as a host your phone reaches.\n\n\
                      Sign in once, then keep `yardsort-host serve` running. Start agents with \
                      `ys`; watch and type into them from a paired phone. Only someone at a \
                      terminal on this computer can pair a device, with `yardsort-host pair`."
    )]
    struct Cli {
        /// Which Yardsort profile to use. Defaults to `YARDSORT_DATA_DIR`, then the app's own.
        #[arg(long, global = true, value_name = "DIR")]
        data_dir: Option<PathBuf>,

        #[command(subcommand)]
        command: Command,
    }

    #[derive(Subcommand)]
    enum Command {
        /// Sign this computer in to a Yardsort server as a host, from a browser on any computer.
        ///
        /// Prints an address to open. Once signed in there, the browser is sent to an address on
        /// 127.0.0.1 that goes nowhere on that computer: paste it here. Or forward the port it
        /// prints over SSH, and the sign-in finishes by itself. Run it while `serve` is stopped.
        SignIn {
            /// The server, such as https://remote.yardsort.sh.
            server: String,
            /// What phones call this computer. Defaults to its host name.
            #[arg(long)]
            name: Option<String>,
        },
        /// Run the host: keep the terminals' records, and the link to the server. Until stopped.
        Serve,
        /// Show a code for a device to pair with, and answer it when it asks. Only from a
        /// terminal on this computer that no agent runs in.
        Pair,
        /// The devices let in here.
        Devices,
        /// Take a device's access away, by the id or name `devices` shows.
        Revoke { device: String },
        /// Whether the host is running, signed in and linked, and its terminals.
        Status,
        /// Sign this computer out at the server, which removes it there, and forget its sign-in.
        SignOut,
        /// Sync projects with your other computers: what syncs, and turning it on or off.
        #[command(subcommand)]
        Sync(Sync),
        /// A project's memory: its notes for agents, and your decisions about them.
        #[command(subcommand)]
        Memory(Memory),
        /// Install an agent, or bring it up to date, into ~/.local, as you.
        ///
        /// Claude Code, Cursor's CLI and Grok come from each vendor's own installer, under the
        /// vendor's terms, which it names before it runs. Codex, OpenCode, Pi and Oh My Pi come from
        /// npm; the full image already has them. Run it as the user that runs `serve`.
        InstallAgent {
            /// claude, cursor, grok, codex, opencode, pi or omp.
            harness: String,
            /// Run this installer file instead of downloading the vendor's: one you fetched and
            /// read yourself, or a mirror's.
            #[arg(long, value_name = "FILE")]
            installer: Option<PathBuf>,
        },
    }

    /// Every one but `status` only from a terminal on this computer that no agent runs in.
    #[derive(Subcommand)]
    enum Sync {
        /// What syncs here, how the last pass went, and the synced projects you could link.
        Status,
        /// Turn sync on for a project, by name or id: its name and its remote's address, without
        /// any user or password, go to the server. Shows them first and asks.
        On {
            project: String,
            /// Do not ask.
            #[arg(long)]
            yes: bool,
        },
        /// Link a project here to one your other computer syncs, by the id or name `sync status`
        /// shows. Nothing is linked by itself.
        Link { project: String, synced: String },
        /// Stop syncing a project here. Its records stay on the server for your other computers.
        Stop { project: String },
        /// Check out here a workspace one of your other computers has, by the name `ys sync
        /// status` shows: its branch, fetched from the project's remote, and a workspace on it.
        /// A branch of that name here with commits the remote's has not is left alone, and
        /// refused.
        Checkout { project: String, workspace: String },
        /// Remove a project's records from the server, for every computer. Each keeps its own
        /// copy of the project.
        Remove {
            project: String,
            /// Do not ask.
            #[arg(long)]
            yes: bool,
        },
    }

    /// Only from a terminal on this computer that no agent runs in, so an agent never approves
    /// its own proposal, nor reads more than `ys memory` gives it. Notes are named by the id
    /// `list` shows.
    #[derive(Subcommand)]
    enum Memory {
        /// A project's notes: waiting, approved, set aside, and any two computers changed at once.
        List { project: String },
        /// Write a note, approved as you write it.
        Write { project: String, text: String },
        /// Approve a proposal an agent made, or a note you rejected.
        Approve { note: String },
        /// Set a proposal aside. It is kept, so the same proposal is known when it comes back.
        Reject { note: String },
        /// Stop giving a note to agents, here and on every computer it syncs with.
        Revoke { note: String },
        /// Give a revoked note to agents again.
        Restore { note: String },
        /// Rewrite a note.
        Edit { note: String, text: String },
        /// Settle a note two computers changed at once: keep the text that stands, the other
        /// computer's, or write a third.
        Settle {
            note: String,
            /// Keep the text that stands, the one agents get now.
            #[arg(long, conflicts_with_all = ["other", "text"])]
            standing: bool,
            /// Keep the other computer's text.
            #[arg(long, conflicts_with = "text")]
            other: bool,
            /// Write a third.
            #[arg(long)]
            text: Option<String>,
        },
    }

    pub fn main() -> i32 {
        let cli = Cli::parse();
        // Needs no profile, and no runtime: it runs the vendor's installer and waits.
        if let Command::InstallAgent { harness, installer } = &cli.command {
            return match crate::install_agent::run(harness, installer.clone()) {
                Ok(()) => 0,
                Err(why) => failed(&why),
            };
        }
        let profile = match Profile::resolve(cli.data_dir) {
            Ok(profile) => profile,
            Err(why) => return failed(&why),
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a Tokio runtime");
        let done = runtime.block_on(async move {
            match cli.command {
                Command::SignIn { server, name } => {
                    crate::sign_in::run(profile, server, name).await
                }
                Command::Serve => crate::serve::run(profile).await,
                Command::Pair => crate::pair::run(&profile).await,
                Command::Devices => devices(&profile).await,
                Command::Revoke { device } => revoke(&profile, device).await,
                Command::Status => status(&profile).await,
                Command::SignOut => crate::sign_in::sign_out(profile).await,
                Command::Sync(command) => sync(&profile, command).await,
                Command::Memory(command) => memory(&profile, command).await,
                Command::InstallAgent { .. } => unreachable!("handled before the profile"),
            }
        });
        match done {
            Ok(()) => 0,
            Err(why) => failed(&why),
        }
    }

    fn failed(why: &str) -> i32 {
        eprintln!("yardsort-host: {why}");
        1
    }

    /// After an upgrade, while the terminal daemon of the release before still runs agents.
    fn earlier_daemon(version: Option<&str>) -> String {
        let release = match version {
            Some(version) => format!("Yardsort {version}"),
            None => "an earlier release".into(),
        };
        format!(
            "The terminal daemon is from {release}, kept for the agents running in it. \
             Phones cannot watch its terminals, new ones included, until it is replaced: once \
             those agents have ended, restart `serve`."
        )
    }

    async fn devices(profile: &Profile) -> Result<(), String> {
        let Reply::Devices { devices } = control::ask_once(profile, Ask::Devices).await? else {
            return Err("The host answered something else.".into());
        };
        if devices.is_empty() {
            println!("No devices are let in. Pair one with `yardsort-host pair`.");
            return Ok(());
        }
        for device in devices {
            println!(
                "{}  {}  {}{}",
                &device.grant_id[..8.min(device.grant_id.len())],
                device.name,
                if device.control {
                    "watches and types"
                } else {
                    "watches"
                },
                match device.projects {
                    None => String::new(),
                    Some(names) => format!(", in {}", names.join(", ")),
                },
            );
        }
        Ok(())
    }

    async fn revoke(profile: &Profile, device: String) -> Result<(), String> {
        match control::ask_once(profile, Ask::Revoke { device }).await? {
            Reply::Revoked { name } => {
                println!("{name} is no longer let in.");
                Ok(())
            }
            _ => Err("The host answered something else.".into()),
        }
    }

    /// Ask, and print what the host says it did.
    async fn done(profile: &Profile, ask: Ask) -> Result<(), String> {
        match control::ask_once(profile, ask).await? {
            Reply::Done { message } => {
                println!("{message}");
                Ok(())
            }
            Reply::Failed { message } => Err(message),
            _ => Err("The host answered something else.".into()),
        }
    }

    /// A yes from the person at this terminal.
    fn confirmed(question: &str) -> bool {
        use std::io::{BufRead, Write};
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    }

    async fn sync(profile: &Profile, command: Sync) -> Result<(), String> {
        match command {
            Sync::Status => sync_status(profile).await,
            Sync::On { project, yes } => {
                let preview = match control::ask_once(
                    profile,
                    Ask::SyncPreview {
                        project: project.clone(),
                    },
                )
                .await?
                {
                    Reply::Preview(preview) => preview,
                    Reply::Failed { message } => return Err(message),
                    _ => return Err("The host answered something else.".into()),
                };
                println!("Turning sync on sends this to the server:");
                println!("  Name:   {}", preview.name);
                println!(
                    "  Remote: {}",
                    preview
                        .remote
                        .as_deref()
                        .unwrap_or("none (no remote, or one on this disk)")
                );
                println!(
                    "  Notes:  {} approved, each with its text and history",
                    preview.notes
                );
                println!(
                    "  Workspaces: {}, by name, branch and task; never their files",
                    preview.workspaces
                );
                println!(
                    "The server keeps synced records in plain text, readable by whoever runs it. \
                     Never the project's path or files."
                );
                if !yes && !confirmed("Turn sync on?") {
                    return Err("Nothing was turned on.".into());
                }
                done(profile, Ask::SyncOn { project }).await
            }
            Sync::Link { project, synced } => {
                done(profile, Ask::SyncLink { project, synced }).await
            }
            Sync::Stop { project } => done(profile, Ask::SyncStop { project }).await,
            Sync::Checkout { project, workspace } => {
                done(profile, Ask::SyncCheckout { project, workspace }).await
            }
            Sync::Remove { project, yes } => {
                if !yes
                    && !confirmed(
                        "Remove this project's records from the server, for every computer?",
                    )
                {
                    return Err("Nothing was removed.".into());
                }
                done(profile, Ask::SyncRemove { project }).await
            }
        }
    }

    async fn memory(profile: &Profile, command: Memory) -> Result<(), String> {
        let decide = |entry: String, decision: &str| Ask::MemoryDecide {
            entry,
            decision: decision.into(),
        };
        let ask = match command {
            Memory::List { project } => return memory_list(profile, project).await,
            Memory::Write { project, text } => Ask::MemoryWrite { project, text },
            Memory::Approve { note } => decide(note, "approve"),
            Memory::Reject { note } => decide(note, "reject"),
            Memory::Revoke { note } => decide(note, "revoke"),
            Memory::Restore { note } => decide(note, "restore"),
            Memory::Edit { note, text } => Ask::MemoryEdit { entry: note, text },
            Memory::Settle {
                note,
                standing,
                other,
                text,
            } => Ask::MemorySettle {
                entry: note,
                keep: match (standing, other, &text) {
                    (true, _, _) => "standing",
                    (_, true, _) => "other",
                    (_, _, Some(_)) => "text",
                    _ => return Err("Say which: --standing, --other or --text \"…\".".into()),
                }
                .into(),
                text,
            },
        };
        done(profile, ask).await
    }

    async fn memory_list(profile: &Profile, project: String) -> Result<(), String> {
        let Reply::Memory { notes } =
            control::ask_once(profile, Ask::MemoryList { project }).await?
        else {
            return Err("The host answered something else.".into());
        };
        if notes.is_empty() {
            println!("No notes yet. Write one with `yardsort-host memory write <project> <text>`.");
            return Ok(());
        }
        let torn: Vec<_> = notes.iter().filter(|n| n.conflict.is_some()).collect();
        if !torn.is_empty() {
            println!("Changed on two computers at once (`yardsort-host memory settle`):");
            for note in torn {
                let (text, from) = note.conflict.as_ref().expect("filtered");
                println!("  {}  stands now: {}", note.id, note.text);
                println!("            from {from}: {text}");
            }
        }
        for (state, heading) in [
            (
                "candidate",
                "Waiting for you (`yardsort-host memory approve <id>`):",
            ),
            ("approved", "Approved:"),
            ("rejected", "Rejected:"),
            ("revoked", "Revoked:"),
        ] {
            let these: Vec<_> = notes
                .iter()
                .filter(|n| n.state == state && n.conflict.is_none())
                .collect();
            if these.is_empty() {
                continue;
            }
            println!("{heading}");
            for note in these {
                println!("  {}  {}  (from {})", note.id, note.text, note.from);
            }
        }
        Ok(())
    }

    async fn sync_status(profile: &Profile) -> Result<(), String> {
        let Reply::Sync(status) = control::ask_once(profile, Ask::SyncStatus).await? else {
            return Err("The host answered something else.".into());
        };
        if !status.remote {
            println!("Remote is off, so nothing syncs. Sign in with `yardsort-host sign-in`.");
            return Ok(());
        }
        println!("Sync: {}", status.state);
        match status.synced_at {
            Some(at) => println!("Last synced: {}", ago(at)),
            None => println!("Last synced: never"),
        }
        if let Some(problem) = status.problem {
            println!("Last problem: {problem}");
        }
        if status.waiting > 0 {
            println!("Waiting to go up: {}", status.waiting);
        }
        if status.projects.is_empty() {
            println!("Syncing: nothing. Turn a project on with `yardsort-host sync on <project>`.");
        } else {
            println!("Syncing:");
            for project in status.projects {
                println!("  {}  {}", project.name, project.state);
            }
        }
        if let Some(problem) = status.offered_problem {
            println!("Could not ask the server what else syncs: {problem}");
        } else if !status.offered.is_empty() {
            println!(
                "Synced elsewhere, not linked here (`yardsort-host sync link <project> <id>`):"
            );
            for offer in status.offered {
                println!(
                    "  {}  {}  {}{}",
                    offer.id,
                    offer.name,
                    offer.remote.as_deref().unwrap_or("no remote"),
                    match offer.matches {
                        Some(name) => format!("  (matches {name} here)"),
                        None => String::new(),
                    }
                );
            }
        }
        Ok(())
    }

    /// How long ago `at` was, roughly.
    fn ago(at: i64) -> String {
        let seconds = (yardsort_core::store::now_ms() - at).max(0) / 1000;
        match seconds {
            0..=59 => format!("{seconds}s ago"),
            60..=3599 => format!("{}m ago", seconds / 60),
            3600..=86_399 => format!("{}h ago", seconds / 3600),
            _ => format!("{}d ago", seconds / 86_400),
        }
    }

    async fn status(profile: &Profile) -> Result<(), String> {
        println!("Profile: {}", profile.data_dir.display());
        println!("Sign-in kept in: {}", profile.credentials_dir.display());
        let reply = match control::ask_once(profile, Ask::Status).await {
            Ok(reply) => reply,
            Err(why) if control::not_serving(&why) => {
                println!("Host: not running. Start it with `yardsort-host serve`.");
                return Ok(());
            }
            Err(why) => return Err(why),
        };
        let Reply::Status(status) = reply else {
            return Err("The host answered something else.".into());
        };
        println!("Host: running (pid {})", status.pid);
        match status.remote {
            None => println!("Remote: off. Sign in with `yardsort-host sign-in <server>`."),
            Some(remote) => {
                println!("Remote: on, as {} at {}", remote.name, remote.server_url);
                println!("Link: {}", remote.link);
                println!("Devices let in: {}", remote.devices);
            }
        }
        println!("Terminals: {}", status.terminals);
        if let Some(earlier) = status.earlier_daemon {
            println!("{}", earlier_daemon(earlier.version.as_deref()));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_earlier_daemon_that_gave_no_version_is_still_named() {
            assert!(earlier_daemon(Some("0.22.2")).contains("from Yardsort 0.22.2, kept"));
            assert!(earlier_daemon(None).contains("from an earlier release, kept"));
        }
    }
}

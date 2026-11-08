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
mod pair;
#[cfg(unix)]
mod profile;
#[cfg(unix)]
mod serve;
#[cfg(unix)]
mod sign_in;

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
    }

    pub fn main() -> i32 {
        let cli = Cli::parse();
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

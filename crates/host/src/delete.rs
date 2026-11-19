//! `yardsort-host account delete`: the operator's side. Says what goes and what stays, takes
//! `delete` typed, then gives the address of a sign-in again, opened in a browser on any computer,
//! and sends back the address that browser lands on, as `sign-in` does. The running host holds the
//! sign-in's verifier and deletes (`serve.rs`); nothing is deleted until that sign-in comes back.

use std::io::{BufRead, IsTerminal};

use tokio::sync::mpsc;

use crate::control::{self, Ask, Reply};
use crate::profile::Profile;

pub async fn run(profile: &Profile) -> Result<(), String> {
    // The host checks this itself, by what the system says of this process; said here first only
    // so the reason is the plainest one.
    if !std::io::stdin().is_terminal() {
        return Err(
            "Deleting the account needs a person at a terminal: run `yardsort-host account \
             delete` in one."
                .into(),
        );
    }
    println!("Deleting your account removes everything the server keeps of it, for every device:");
    println!("  Every phone and computer signed in to it, each of them signed out");
    println!("  The devices each computer lets in, and which phones get notifications");
    println!("  Every synced project, note and workspace name");
    println!(
        "What stays: everything on this computer and on your other devices. Projects, workspaces \
         and agents carry on. Remote turns off here."
    );
    println!(
        "It cannot be undone. Export your account first to keep a copy: `yardsort-host account \
         export`. To confirm it is you, you sign in again in a browser.\n"
    );
    let mut typed = typed();
    eprint!("Type delete to confirm: ");
    let confirmed = typed.recv().await;
    if confirmed.map(|t| t.trim().to_lowercase()).as_deref() != Some("delete") {
        return Err("Nothing was deleted.".into());
    }
    let mut line = control::connect(profile).await?;
    line.send(&Ask::AccountDelete).await?;
    converse(&mut line, &mut typed).await
}

/// Once the host has given the confirming sign-in, the browser may bring it back at any moment,
/// by the forwarded port or a paste, and the host then deletes before it answers. So a host that
/// goes quiet after that may have deleted: it is said that the outcome is not known.
const UNKNOWN: &str = "The host stopped answering after it gave the sign-in to confirm with, so \
                       whether the account was deleted is not known. It may have been: run \
                       `yardsort-host status` once the host is running again, or look on another \
                       device.";

/// The conversation with the host, from the ask to its answer.
async fn converse(
    line: &mut control::Line,
    typed: &mut mpsc::UnboundedReceiver<String>,
) -> Result<(), String> {
    let mut confirming = false;
    let lost = |confirming: bool, why: String| {
        if confirming {
            UNKNOWN.to_owned()
        } else {
            format!("{why} Nothing was deleted.")
        }
    };
    loop {
        let reply = tokio::select! {
            reply = line.next::<Reply>() => reply,
            pasted = typed.recv() => match pasted {
                Some(address) if address.trim().is_empty() => {
                    eprint!("Address: ");
                    continue;
                }
                Some(address) => {
                    if let Err(why) = line.send(&Ask::Pasted { address }).await {
                        return Err(lost(confirming, why));
                    }
                    continue;
                }
                // No more input: the forwarded port may still bring the browser back.
                None => line.next::<Reply>().await,
            },
        };
        let reply = match reply {
            Ok(Some(reply)) => reply,
            Ok(None) => return Err(lost(confirming, "The host closed the connection.".into())),
            Err(why) => return Err(lost(confirming, why)),
        };
        match reply {
            Reply::Confirm { url, port } => {
                confirming = true;
                println!("Open this address in a browser, on any computer, and sign in again:\n");
                println!("  {url}\n");
                println!(
                    "The browser is then sent to an address starting \
                     http://127.0.0.1:{port}/signed-in, which goes nowhere on that computer. \
                     Copy the whole address from its address bar, paste it here, and press \
                     Enter.\n"
                );
                println!(
                    "Or forward the port from that computer first, and it finishes by itself:\n  \
                     ssh -L {port}:127.0.0.1:{port} <this computer>\n"
                );
                eprint!("Address: ");
            }
            Reply::NotThatAddress { message } => {
                eprintln!("{message}");
                eprint!("Address: ");
            }
            Reply::Deleted {
                devices,
                hosts,
                records,
            } => {
                println!(
                    "\nYour account was deleted at the server, with {}, {} and {}. Every device \
                     was signed out. Remote is off on this computer, and everything on it is \
                     still here.",
                    counted(devices, "device"),
                    counted(hosts, "computer"),
                    counted(records, "synced record")
                );
                return Ok(());
            }
            Reply::Failed { message } => return Err(message),
            _ => return Err("The host answered something else.".into()),
        }
    }
}

/// Each line typed at the terminal. Read on a thread of its own, not Tokio's: a read still
/// waiting once the account is gone would keep the runtime, and so the command, from ending.
fn typed() -> mpsc::UnboundedReceiver<String> {
    let (send, receive) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if send.send(line).is_err() {
                return;
            }
        }
    });
    receive
}

/// `1 device`, `3 devices`.
fn counted(n: u32, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host on the other end of a socket pair, which answers the ask as `host` says, then goes.
    async fn against<F, Fut>(host: F, typed: &[&str]) -> Result<(), String>
    where
        F: FnOnce(control::Line) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let (here, there) = tokio::net::UnixStream::pair().unwrap();
        let hosting = tokio::spawn(async move {
            let mut there = control::Line::new(there);
            assert!(matches!(
                there.next::<Ask>().await.unwrap(),
                Some(Ask::AccountDelete)
            ));
            host(there).await;
        });
        let (send, mut lines) = mpsc::unbounded_channel();
        for line in typed {
            send.send((*line).to_owned()).unwrap();
        }
        let mut here = control::Line::new(here);
        here.send(&Ask::AccountDelete).await.unwrap();
        let said = converse(&mut here, &mut lines).await;
        hosting.await.unwrap();
        said
    }

    fn confirm() -> Reply {
        Reply::Confirm {
            url: "https://remote.example.test/v1/login?purpose=confirm".into(),
            port: 45999,
        }
    }

    #[tokio::test]
    async fn a_host_gone_before_it_gave_the_sign_in_deleted_nothing() {
        let said = against(|_gone| async {}, &[]).await.unwrap_err();
        assert_eq!(said, "The host closed the connection. Nothing was deleted.");
    }

    #[tokio::test]
    async fn a_host_gone_after_the_address_was_pasted_may_have_deleted_and_says_it_is_not_known() {
        let said = against(
            |mut there| async move {
                there.send(&confirm()).await.unwrap();
                // The address arrives, the server deletes, and the answer never comes back.
                assert!(matches!(
                    there.next::<Ask>().await.unwrap(),
                    Some(Ask::Pasted { .. })
                ));
            },
            &["http://127.0.0.1:45999/signed-in?code=the-code"],
        )
        .await
        .unwrap_err();
        assert_eq!(said, UNKNOWN);
        assert!(!said.contains("Nothing was deleted"));
    }

    #[tokio::test]
    async fn a_host_gone_after_giving_the_sign_in_may_have_deleted_by_the_forwarded_port() {
        let said = against(
            |mut there| async move {
                there.send(&confirm()).await.unwrap();
            },
            &[],
        )
        .await
        .unwrap_err();
        assert_eq!(said, UNKNOWN);
    }

    #[tokio::test]
    async fn the_hosts_word_that_it_deleted_is_success() {
        let said = against(
            |mut there| async move {
                there.send(&confirm()).await.unwrap();
                there
                    .send(&Reply::Deleted {
                        devices: 2,
                        hosts: 1,
                        records: 3,
                    })
                    .await
                    .unwrap();
            },
            &[],
        )
        .await;
        assert_eq!(said, Ok(()));
    }
}

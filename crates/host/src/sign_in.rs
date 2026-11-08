//! `yardsort-host sign-in` and `sign-out`.
//!
//! Signing in is OIDC with PKCE, as everywhere else: the server sends the browser back to a
//! loopback address here. This computer has no browser, so the operator signs in on another
//! computer's, then pastes back the address it was sent to, which went nowhere there. Or they
//! forward the port over SSH, and it arrives by itself. The code in that address is worth nothing
//! without the verifier this process keeps.

use remote_client::SIGN_IN_TIMEOUT;
use remote_host::account;
use remote_proto::account::{DeviceKind, NewDevice};
use remote_proto::Platform;
use tokio::io::{AsyncBufReadExt, BufReader};
use yardsort_core::presence::{AppLock, Holder};

use crate::control::{self, Ask, Reply};
use crate::home::HostHome;
use crate::profile::Profile;

/// The profile's lock, for a command that holds the sign-in while the host is not serving.
fn hold(profile: &Profile, doing: &str) -> Result<AppLock, String> {
    AppLock::take_as(&profile.data_dir, Holder::Host)
        .map_err(|e| format!("Cannot use {}: {e}", profile.data_dir.display()))?
        .ok_or_else(|| {
            format!(
                "Yardsort is running on {}: the app, or `yardsort-host serve`. Stop it, {doing}, \
                 then start it again. Two programs holding one sign-in get it revoked.",
                profile.data_dir.display()
            )
        })
}

pub async fn run(profile: Profile, server: String, name: Option<String>) -> Result<(), String> {
    let _lock = hold(&profile, "sign in")?;
    let name = match name {
        Some(name) => name.trim().to_owned(),
        None => host_name().ok_or("Name this computer with --name.")?,
    };
    let (client, described) = account::prepare(&server, &name)
        .await
        .map_err(|f| f.message)?;
    let sign_in = client
        .begin_sign_in()
        .await
        .map_err(|e| account::failed(e).message)?;
    let port = sign_in.redirect_port();
    println!("Open this address in a browser, on any computer, and sign in:\n");
    println!("  {}\n", sign_in.url());
    println!(
        "The browser is then sent to an address starting http://127.0.0.1:{port}/signed-in, \
         which goes nowhere on that computer. Copy the whole address from its address bar, \
         paste it here, and press Enter.\n"
    );
    println!(
        "Or forward the port from that computer first, and the sign-in finishes by itself:\n  \
         ssh -L {port}:127.0.0.1:{port} <this computer>\n"
    );
    let pasted = async {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            eprint!("Address: ");
            match lines.next_line().await {
                Ok(Some(line)) if line.trim().is_empty() => continue,
                Ok(Some(line)) => match sign_in.pasted(&line) {
                    Ok(code) => return Ok(code),
                    Err(error) => eprintln!("{error}"),
                },
                // No terminal to paste into: wait for the browser instead.
                Ok(None) | Err(_) => std::future::pending::<()>().await,
            }
        }
    };
    let code = tokio::select! {
        returned = sign_in.returned(SIGN_IN_TIMEOUT) => returned,
        pasted = pasted => pasted,
    }
    .map_err(|e| account::failed(e).message)?;
    let device = NewDevice {
        name: name.clone(),
        kind: DeviceKind::Host,
        platform: Platform::current(),
    };
    let signed_in = sign_in
        .redeem(&client, device, code)
        .await
        .map_err(|e| account::failed(e).message)?;
    let home = HostHome::open(profile, None)?;
    let row = account::enable(&home, client, &described, &name, signed_in)
        .await
        .map_err(|f| f.message)?;
    println!(
        "\nSigned in as {} at {}. The sign-in is kept in {}.",
        row.name,
        row.server_url,
        home.profile.credentials_dir.display()
    );
    println!(
        "Start the host with `yardsort-host serve`, then pair a phone with `yardsort-host pair`."
    );
    Ok(())
}

pub async fn sign_out(profile: Profile) -> Result<(), String> {
    match control::ask_once(&profile, Ask::SignOut).await {
        Ok(Reply::SignedOut) => {}
        Ok(_) => return Err("The host answered something else.".into()),
        // Not serving: do it here, holding the profile meanwhile.
        Err(why) if control::not_serving(&why) => {
            let _lock = hold(&profile, "sign out")?;
            let home = HostHome::open(profile, None)?;
            account::disable(home.as_ref())
                .await
                .map_err(|f| f.message)?;
        }
        Err(why) => return Err(why),
    }
    println!(
        "Signed out. This computer is no longer a host at its server, and no device reaches it."
    );
    Ok(())
}

/// This computer's name, as phones would first see it.
fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for its length, and `gethostname` writes within it.
    let got = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if got != 0 {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

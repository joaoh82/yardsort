//! Sign a pretend device in to a Yardsort server, as the app will: the browser goes to the server
//! and on to its identity provider, comes back to a listener on this machine's loopback address,
//! and the one-time code it brings is redeemed with this run's PKCE verifier.
//!
//! ```text
//! cargo run -p yardsort-server --example sign-in -- [SERVER] [--keep | --wait-for-revoke]
//!     [--no-browser | --zitadel-api]
//! ```
//!
//! `SERVER` is `http://127.0.0.1:8787` unless given (`just server-run`). After signing in it lists
//! the account's devices, refreshes once, and revokes itself unless `--keep` is given. With
//! `--wait-for-revoke` it stays signed in until something else revokes it (`yardsort-server
//! revoke-account`), then checks that its access and refresh credentials are both refused.
//!
//! `--zitadel-api` signs in without a browser, for automated runs against a ZITADEL: it does what
//! ZITADEL's own login pages do, through its session API, as `YARDSORT_SIGN_IN_AS` with
//! `YARDSORT_SIGN_IN_PASSWORD`, using the login client's token in `ZITADEL_LOGIN_CLIENT_TOKEN`. It
//! prints the ZITADEL user id, which is the subject `revoke-account` takes.
//!
//! Credentials are never printed; nothing is saved.

use std::time::Duration;

use remote_proto::account::{
    Credentials, DeviceKind, DevicePage, NewDevice, Secret, TokenRequest, CHALLENGE_METHOD,
};
use remote_proto::{Platform, RemoteError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;
use yardsort_server::{oidc, tokens};

mod common;
use common::{open, through_zitadel_api};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let keep = flag("--keep");
    let wait_for_revoke = flag("--wait-for-revoke");
    let zitadel_api = flag("--zitadel-api");
    let browser = !flag("--no-browser") && !zitadel_api;
    let server = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map_or("http://127.0.0.1:8787", String::as_str);
    let server = Url::parse(server).map_err(|e| format!("{server}: {e}"))?;
    // The server's own client settings: no redirects followed, a timeout, rustls.
    let http = oidc::http_client_builder()
        .build()
        .map_err(|e| e.to_string())?;

    let verifier = tokens::random();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://127.0.0.1:{}/signed-in",
        listener.local_addr().map_err(|e| e.to_string())?.port()
    );
    let mut login = server.join("v1/login").map_err(|e| e.to_string())?;
    login
        .query_pairs_mut()
        .append_pair("challenge", &tokens::s256(&verifier))
        .append_pair("challengeMethod", CHALLENGE_METHOD)
        .append_pair("redirect", &redirect);

    let code = if zitadel_api {
        // Both at once: the last step of the API's sign-in is the redirect to the listener.
        let (code, signed_in) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(60), returned(&listener)),
            through_zitadel_api(&http, login),
        );
        signed_in?;
        code.map_err(|_| "no sign-in came back within a minute".to_owned())??
    } else {
        println!("Sign in, in a browser:\n\n  {login}\n");
        if browser {
            open(login.as_str());
        }
        tokio::time::timeout(Duration::from_secs(600), returned(&listener))
            .await
            .map_err(|_| "no sign-in came back within ten minutes".to_owned())??
    };

    let credentials: Credentials = call(
        http.post(server.join("v1/token").map_err(|e| e.to_string())?)
            .json(&TokenRequest::SignIn {
                code: Secret::new(code),
                verifier: Secret::new(verifier),
                device: NewDevice {
                    name: "Sign-in tool".into(),
                    kind: DeviceKind::Computer,
                    platform: Platform::current(),
                },
            }),
    )
    .await?;
    println!("Signed in as device {}.", credentials.device);

    let devices = |access: &Secret| {
        http.get(server.join("v1/devices").unwrap())
            .bearer_auth(access.expose())
    };
    let page: DevicePage = call(devices(&credentials.access)).await?;
    println!("\nThis account's devices, newest first:");
    for d in &page.devices {
        println!(
            "  {} {:<24} {:?} on {:?}{}",
            if d.current { "*" } else { " " },
            d.name,
            d.kind,
            d.platform,
            if d.revoked_at.is_some() {
                "  (revoked)"
            } else {
                ""
            }
        );
    }

    let renewed: Credentials = call(http.post(server.join("v1/token").unwrap()).json(
        &TokenRequest::Refresh {
            refresh: credentials.refresh.clone(),
        },
    ))
    .await?;
    let _: DevicePage = call(devices(&renewed.access)).await?;
    println!("\nRefreshed: the new access credential works.");

    if keep {
        println!("Kept signed in (--keep). Nothing was saved, so this device is now unreachable.");
        return Ok(());
    }
    if wait_for_revoke {
        println!("Waiting for this device to be revoked elsewhere, for up to ten minutes.");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            let response = devices(&renewed.access)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                return Err("not revoked within ten minutes".into());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let refreshed = http
            .post(server.join("v1/token").unwrap())
            .json(&TokenRequest::Refresh {
                refresh: renewed.refresh.clone(),
            })
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if refreshed.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Err(format!(
                "revoked, but refreshing got {} rather than 401",
                refreshed.status()
            ));
        }
        println!("Revoked elsewhere: its access and refresh credentials now both get 401.");
        return Ok(());
    }
    let revoke = server
        .join(&format!("v1/devices/{}/revoke", renewed.device))
        .unwrap();
    let response = http
        .post(revoke)
        .bearer_auth(renewed.access.expose())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("revoking: {}", response.status()));
    }
    let after = devices(&renewed.access)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "Revoked this device; its access credential now gets {}.",
        after.status()
    );
    Ok(())
}

/// The one-time code the browser brings back to the loopback listener.
async fn returned(listener: &TcpListener) -> Result<String, String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 8192];
        let read = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
        let request = String::from_utf8_lossy(&buffer[..read]);
        let Some(target) = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        else {
            continue;
        };
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        if url.path() != "/signed-in" {
            let _ = socket
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await;
            continue;
        }
        let query = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let (body, outcome) = match (query("code"), query("error")) {
            (Some(code), _) => ("Signed in. You can close this window.", Ok(code)),
            (None, Some(error)) => (
                "Not signed in. You can close this window.",
                Err(format!("the server answered {error}")),
            ),
            (None, None) => continue,
        };
        let _ = socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        return outcome;
    }
}

async fn call<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, String> {
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    if status.is_success() {
        return response.json().await.map_err(|e| e.to_string());
    }
    match response.json::<RemoteError>().await {
        Ok(error) => Err(format!("{status}: {} ({:?})", error.message, error.code)),
        Err(_) => Err(status.to_string()),
    }
}

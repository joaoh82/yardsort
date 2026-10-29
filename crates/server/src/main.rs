//! `yardsort-server`: the control service on PostgreSQL, over HTTP. Configured by its environment;
//! see [`yardsort_server::config`].
//!
//! `yardsort-server revoke-account --subject SUBJECT [--issuer ISSUER]` signs every device of one
//! account out instead, for an operator: the provider's subject, at the provider the server is
//! configured with unless `--issuer` names another. It needs only `DATABASE_URL`.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;
use yardsort_server::config::{AdminSettings, Settings};
use yardsort_server::http::{self, now_ms};
use yardsort_server::postgres::PgRecords;
use yardsort_server::records::{Records, RevokeReason, Revoked};
use yardsort_server::relay::Relay;
use yardsort_server::{oidc, ControlService, ServerConfig};

const USAGE: &str = "usage: yardsort-server [revoke-account --subject SUBJECT [--issuer ISSUER]]";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        // Colours for a person at a terminal; plain text for a platform's log collector.
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => serve().await,
        Some("revoke-account") => revoke_account(&args[1..]).await,
        Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<(), String> {
    let settings = Settings::from_env().map_err(|e| e.to_string())?;
    let records = PgRecords::connect(settings.database_url.expose())
        .await
        .map_err(|e| e.to_string())?;
    let server = records
        .server_id(now_ms())
        .await
        .map_err(|e| e.to_string())?;
    let http_client = oidc::http_client().map_err(|e| e.to_string())?;
    let service = ControlService::new(
        ServerConfig {
            server,
            name: settings.name,
            public_url: settings.public_url,
            provider: settings.provider,
            device_redirects: settings.device_redirects,
        },
        records,
        http_client,
    )
    .map_err(|e| e.to_string())?
    .with_relay(Relay::new(settings.instance, settings.routing));
    tracing::info!(
        server = %service.describe().server,
        issuer = %service.describe().issuer,
        callback = %service.callback_url(),
        "the callback is the redirect URI to register at the identity provider",
    );

    let listener = tokio::net::TcpListener::bind(settings.listen)
        .await
        .map_err(|e| format!("listening on {}: {e}", settings.listen))?;
    tracing::info!(
        instance = %service.relay().instance(),
        routing = ?service.relay().routing(),
        "listening on {}",
        settings.listen
    );
    let relay = service.relay().clone();
    let app = http::router(
        service,
        http::Options {
            client_ip_header: settings.client_ip_header,
            ..http::Options::default()
        },
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    // The relay's sockets stay open until told to close, and a graceful stop waits for them.
    .with_graceful_shutdown(async move {
        stopped().await;
        relay.shutdown();
    })
    .await
    .map_err(|e| e.to_string())
}

/// Revoke every device of one account, as its owner could one at a time. The devices stop on their
/// next request. Signing in again is still possible, if the provider lets the person in.
async fn revoke_account(args: &[String]) -> Result<(), String> {
    let mut subject = None;
    let mut issuer = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--subject" => &mut subject,
            "--issuer" => &mut issuer,
            other => return Err(format!("unexpected {other:?}\n{USAGE}")),
        };
        *slot = Some(
            args.next()
                .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?
                .clone(),
        );
    }
    let subject = subject.ok_or_else(|| format!("--subject is required\n{USAGE}"))?;
    let settings = AdminSettings::from_env().map_err(|e| e.to_string())?;
    let issuer = issuer
        .or(settings.issuer)
        .ok_or("give --issuer, or set YARDSORT_OIDC_ISSUER")?;
    let records = PgRecords::connect(settings.database_url.expose())
        .await
        .map_err(|e| e.to_string())?;
    let revoked = Revoked {
        at: now_ms(),
        reason: RevokeReason::ByOperator,
    };
    match records
        .revoke_account(&issuer, &subject, revoked)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(count) => {
            let devices = if count == 1 { "device" } else { "devices" };
            println!("Revoked {count} {devices} of the account for {subject} at {issuer}.");
            Ok(())
        }
        None => Err(format!(
            "no account for {subject} at {issuer}: that subject has never signed in here"
        )),
    }
}

/// Ctrl-C, or the `SIGTERM` a platform sends before it replaces an instance.
async fn stopped() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    tracing::info!("stopping");
}

//! `yardsort-server`: the control service on PostgreSQL, over HTTP. Configured by its environment;
//! see [`yardsort_server::config`].

use std::net::SocketAddr;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;
use yardsort_server::config::Settings;
use yardsort_server::http::{self, now_ms};
use yardsort_server::postgres::PgRecords;
use yardsort_server::{oidc, ControlService, ServerConfig};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
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
    .map_err(|e| e.to_string())?;
    tracing::info!(
        server = %service.describe().server,
        issuer = %service.describe().issuer,
        callback = %service.callback_url(),
        "the callback is the redirect URI to register at the identity provider",
    );

    let listener = tokio::net::TcpListener::bind(settings.listen)
        .await
        .map_err(|e| format!("listening on {}: {e}", settings.listen))?;
    tracing::info!("listening on {}", settings.listen);
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
    .with_graceful_shutdown(stopped())
    .await
    .map_err(|e| e.to_string())
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

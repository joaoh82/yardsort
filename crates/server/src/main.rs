//! `yardsort-server`: the control service on PostgreSQL, over HTTP. Configured by its environment;
//! see [`yardsort_server::config`].
//!
//! `yardsort-server revoke-account --subject SUBJECT [--issuer ISSUER]` signs every device of one
//! account out instead, for an operator: the provider's subject, at the provider the server is
//! configured with unless `--issuer` names another. It needs only `DATABASE_URL`, and signals the
//! running servers through it, which close those devices' live sockets at once.
//!
//! `yardsort-server usage [--from DATE] [--to DATE]` prints what each account used a day at a
//! time, `YYYY-MM-DD` in UTC, the last seven days unless told otherwise (`crate::usage`). It too
//! needs only `DATABASE_URL`.
//!
//! `yardsort-server reconcile` brings a database restored from a backup up to date with the
//! journal (`yardsort_server::journal`): every revocation and deletion made since the backup is
//! made again. It needs `DATABASE_URL` and the journal's settings, and tells the running servers,
//! which refuse every request until it has run.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;
use yardsort_server::config::{AdminSettings, Settings};
use yardsort_server::entitlements::Entitlements;
use yardsort_server::http::{self, now_ms};
use yardsort_server::journal::{self, Journal, Standing};
use yardsort_server::postgres::PgRecords;
use yardsort_server::records::{Records, RevokeReason, Revoked, Signal};
use yardsort_server::relay::Relay;
use yardsort_server::{oidc, ControlService, ServerConfig};

const USAGE: &str = "usage: yardsort-server [revoke-account --subject SUBJECT [--issuer ISSUER] \
                     | usage [--from YYYY-MM-DD] [--to YYYY-MM-DD] | reconcile]";

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
        Some("usage") => usage(&args[1..]).await,
        Some("reconcile") if args.len() == 1 => reconcile().await,
        Some("reconcile") => Err(format!("reconcile takes nothing more\n{USAGE}")),
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
    let push = std::sync::Arc::new(yardsort_server::push::ConfiguredPush::new(
        http_client.clone(),
        settings.push.apns,
        settings.push.fcm,
    ));
    let service = ControlService::new(
        ServerConfig {
            server,
            name: settings.name,
            public_url: settings.public_url,
            provider: settings.provider,
            device_redirects: settings.device_redirects,
        },
        records,
        http_client.clone(),
    )
    .map_err(|e| e.to_string())?
    .with_relay(Relay::new(settings.instance, settings.routing))
    .with_push(push)
    .with_entitlements(match settings.entitlements {
        Some(service) => Entitlements::service(service.url, service.token, http_client.clone()),
        None => Entitlements::none(),
    })
    .with_journal(match settings.journal {
        Some(journal) => Journal::new(journal.store(http_client.clone())),
        None => Journal::none(),
    });
    match service.journal().store() {
        Some(store) => tracing::info!(
            "revocations and deletions are journaled in {}",
            store.describe()
        ),
        None => tracing::warn!(
            "no journal (YARDSORT_JOURNAL_DIR or a bucket): a restore of the database from a \
             backup can let revoked devices back in and bring deleted accounts back"
        ),
    }
    // Before serving anything: records restored from a backup answer nothing until reconciled.
    if service.journal().check(service.records()).await == Standing::Behind {
        tracing::error!(
            "every request is refused with 503 until `yardsort-server reconcile` has run against \
             this database"
        );
    }
    tracing::info!(
        entitlements = service.entitlements_configured(),
        "entitlements are asked when configured; unset, everything is allowed"
    );
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
    let kept = service.records().clone();
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
    .with_graceful_shutdown({
        let relay = relay.clone();
        async move {
            stopped().await;
            relay.shutdown();
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    // What was counted since the last minute, once every socket has closed.
    let counted = relay.meter().take();
    if let Err(error) = kept.add_usage(&counted, now_ms()).await {
        tracing::error!("usage: records: {error}");
    }
    Ok(())
}

/// Replay the journal into restored records, after sending them what they hold that it lacks.
async fn reconcile() -> Result<(), String> {
    let settings = AdminSettings::from_env().map_err(|e| e.to_string())?;
    let store = settings
        .journal
        .ok_or("no journal is configured: set YARDSORT_JOURNAL_DIR, or the bucket's settings")?
        .store(oidc::http_client().map_err(|e| e.to_string())?);
    let records = PgRecords::connect(settings.database_url.expose())
        .await
        .map_err(|e| e.to_string())?;
    println!("Reconciling the records with {}.", store.describe());
    let done = journal::reconcile(&store, &records, now_ms()).await?;
    if done.sent > 0 {
        println!("Sent {} the journal did not have yet.", entries(done.sent));
    }
    if done.replayed == 0 {
        println!("Nothing to replay: the records hold every entry of the journal.");
    } else {
        println!(
            "Replayed {} the records did not have: revoked {} again, revoked {} again, removed {} \
             again, deleted {} again.{}",
            entries(done.replayed),
            counted(done.devices, "device", "devices"),
            counted(done.grants, "grant", "grants"),
            counted(done.hosts, "host", "hosts"),
            counted(done.accounts, "account", "accounts"),
            match done.replayed - done.changed {
                0 => String::new(),
                n => format!(" {} closed already.", entries(n)),
            },
        );
    }
    if !done.unreadable.is_empty() {
        return Err(format!(
            "{} written by a newer server, which this one cannot read: run that server's \
             `reconcile`. Until then the records stay behind, and the servers keep refusing \
             requests: {}",
            entries(done.unreadable.len()),
            done.unreadable.join(", ")
        ));
    }
    println!("Running servers serve again within moments.");
    Ok(())
}

fn entries(n: usize) -> String {
    counted(n, "entry", "entries")
}

fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Print each account's days, from `--from` to `--to`.
async fn usage(args: &[String]) -> Result<(), String> {
    let today = yardsort_server::usage::day_of(now_ms());
    let (mut from, mut to) = (today - 6, today);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--from" => &mut from,
            "--to" => &mut to,
            other => return Err(format!("unexpected {other:?}\n{USAGE}")),
        };
        let value = args
            .next()
            .ok_or_else(|| format!("{arg} needs a date\n{USAGE}"))?;
        *slot = yardsort_server::usage::parse_date(value)
            .ok_or_else(|| format!("{arg} {value:?}: a date as YYYY-MM-DD"))?;
    }
    if from > to {
        return Err("--from is after --to".into());
    }
    let settings = AdminSettings::from_env().map_err(|e| e.to_string())?;
    let records = PgRecords::connect(settings.database_url.expose())
        .await
        .map_err(|e| e.to_string())?;
    let days = records.usage(from, to).await.map_err(|e| e.to_string())?;
    print!("{}", yardsort_server::usage::table(&days));
    Ok(())
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
            // Into the journal now, rather than at a running server's next send.
            if let Some(journal) = settings.journal {
                let store = journal.store(oidc::http_client().map_err(|e| e.to_string())?);
                if let Err(error) = Journal::new(store).flush(&records).await {
                    println!("Running servers write it to the journal within moments ({error}).");
                }
            }
            // Running servers close the devices' live sockets now, not at their next check.
            match records.signal(Signal::RecheckAll).await {
                Ok(()) => println!("Running servers close their live connections now."),
                Err(error) => println!(
                    "Running servers close their live connections within five seconds ({error})."
                ),
            }
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

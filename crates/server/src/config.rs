//! The server's settings, from its environment. Each secret can come from a file instead
//! (`…_FILE`), as container platforms mount them; a secret never comes from the command line, where
//! any process on the machine can read it.
//!
//! - `YARDSORT_PUBLIC_URL`: the server's own address, as browsers reach it.
//! - `YARDSORT_OIDC_ISSUER`: the identity provider, as its issuer URL.
//! - `YARDSORT_OIDC_CLIENT_ID`: this server's client id at the provider.
//! - `YARDSORT_OIDC_CLIENT_SECRET` (or `_FILE`): its client secret. Unset for a public PKCE client.
//! - `DATABASE_URL` (or `_FILE`): PostgreSQL, as `postgres://…`. A direct connection, not a pooler
//!   that hands out one per transaction: the server listens for its instances' signals on it.
//! - `YARDSORT_LISTEN`: the address to listen on; [`DEFAULT_LISTEN`] unless set.
//! - `YARDSORT_SERVER_NAME`: what people see it called; [`DEFAULT_NAME`] unless set.
//! - `YARDSORT_DEVICE_REDIRECTS`: comma-separated; [`DEFAULT_DEVICE_REDIRECTS`] unless set.
//! - `YARDSORT_CLIENT_IP_HEADER`: behind a proxy, the header it names the client's address in
//!   (`Fly-Client-IP`), for rate limits.
//! - `YARDSORT_ROUTING`: `fly-replay` behind Fly.io's proxy, which lets the server run as several
//!   instances: the relay sends a request on to the instance it belongs to. Unset, or `single`,
//!   the server must run as one instance.
//! - `YARDSORT_INSTANCE`: this instance's name, as that proxy knows it. On Fly.io it is the
//!   Machine's id, which Fly.io sets as `FLY_MACHINE_ID` and is used when this is unset; anywhere
//!   else, a name made up at each start.
//!
//! Entitlements are optional (`crate::entitlements`). Unset, everything is allowed up to the
//! server's own limits, as a self-hosted server runs.
//! - `YARDSORT_ENTITLEMENTS_URL`: the service asked before another device, a connection to a host
//!   or more synced data. HTTPS, or plain HTTP to a loopback address.
//! - `YARDSORT_ENTITLEMENTS_TOKEN` (or `_FILE`): sent to it as a bearer credential.
//!
//! The journal is optional (`crate::journal`), and a server run for others should have one: without
//! it, a database restored from a backup can let revoked devices back in and bring deleted
//! accounts back. One of:
//! - `YARDSORT_JOURNAL_DIR`: a directory, kept apart from the database and its backups.
//! - `YARDSORT_JOURNAL_BUCKET`, or `BUCKET_NAME` as `fly storage create` sets it: a bucket of an
//!   S3-compatible store, at `AWS_ENDPOINT_URL_S3`, in `AWS_REGION` (`auto` unless set), with
//!   `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (or `_FILE`). The bucket is named in the
//!   host (`<bucket>.<endpoint>`), as AWS and Tigris expect, or in the path for an endpoint at an
//!   IP address or `localhost`; `YARDSORT_JOURNAL_ADDRESSING` (`virtual` or `path`) says otherwise.
//!
//! Push is optional. With none of these set, notices wait until a provider is configured.
//! The design is `docs/design/40-mobile-notifications.md`.
//! - `YARDSORT_APNS_KEY_ID`, `YARDSORT_APNS_TEAM_ID`, `YARDSORT_APNS_BUNDLE_ID`
//!   (`dev.yardsort.mobile` for a release build, `dev.yardsort.mobile.development` for a
//!   development build), and `YARDSORT_APNS_KEY` or `YARDSORT_APNS_KEY_FILE` (the `.p8` PEM).
//!   `YARDSORT_APNS_SANDBOX=1` sends to the sandbox.
//! - `YARDSORT_FCM_CREDENTIALS` or `YARDSORT_FCM_CREDENTIALS_FILE` (a service account JSON).
//!   `YARDSORT_FCM_PROJECT_ID` overrides the project id in that file.

use std::net::SocketAddr;

use axum::http::HeaderName;
use remote_proto::account::Secret;
use url::Url;

use crate::oidc::ProviderConfig;
use crate::relay::{Instance, Routing};

pub const DEFAULT_LISTEN: &str = "127.0.0.1:8787";
pub const DEFAULT_NAME: &str = "Yardsort";
/// The app's own scheme, and a computer's loopback listener on any port.
pub const DEFAULT_DEVICE_REDIRECTS: &[&str] =
    &["yardsort://signed-in", "http://127.0.0.1/signed-in"];

pub struct Settings {
    pub listen: SocketAddr,
    pub name: String,
    pub public_url: Url,
    pub provider: ProviderConfig,
    pub device_redirects: Vec<String>,
    pub database_url: Secret,
    /// Where a trusted proxy names the client's address, for rate limits. Unset, the connection's
    /// own address is used.
    pub client_ip_header: Option<HeaderName>,
    pub routing: Routing,
    pub instance: Instance,
    pub push: PushSettings,
    pub entitlements: Option<EntitlementsSettings>,
    pub journal: Option<JournalSettings>,
}

/// Where the journal is: a directory, or a bucket.
#[derive(Debug)]
pub enum JournalSettings {
    Dir(std::path::PathBuf),
    Bucket {
        endpoint: Url,
        bucket: String,
        addressing: crate::journal::Addressing,
        region: String,
        access_key: String,
        secret_key: Secret,
    },
}

impl JournalSettings {
    pub fn store(self, http: reqwest::Client) -> crate::journal::Store {
        match self {
            Self::Dir(root) => crate::journal::Store::Dir(crate::journal::Dir::new(root)),
            Self::Bucket {
                endpoint,
                bucket,
                addressing,
                region,
                access_key,
                secret_key,
            } => crate::journal::Store::S3(crate::journal::S3::new(
                endpoint,
                bucket,
                addressing,
                region,
                access_key,
                secret_key.expose().to_owned(),
                http,
            )),
        }
    }
}

/// The entitlements service, when the operator configured one.
pub struct EntitlementsSettings {
    pub url: Url,
    pub token: Option<Secret>,
}

/// APNs and FCM, when the operator configured them. A missing piece leaves that provider off.
pub struct PushSettings {
    pub apns: Option<crate::push::Apns>,
    pub fcm: Option<crate::push::Fcm>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("listen", &self.listen)
            .field("name", &self.name)
            .field("public_url", &self.public_url.as_str())
            .field("issuer", &self.provider.issuer)
            .field("client_id", &self.provider.client_id)
            .field("client_secret", &self.provider.client_secret.is_some())
            .field("device_redirects", &self.device_redirects)
            .field("client_ip_header", &self.client_ip_header)
            .field("routing", &self.routing)
            .field("instance", &self.instance.as_str())
            .field("apns", &self.push.apns.is_some())
            .field("fcm", &self.push.fcm.is_some())
            .field(
                "entitlements",
                &self.entitlements.as_ref().map(|e| e.url.as_str()),
            )
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl Settings {
    /// From the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// From any lookup, for tests.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |name: &str| get(name).filter(|value| !value.trim().is_empty());
        let required =
            |name: &str| get(name).ok_or_else(|| ConfigError(format!("{name} is not set")));
        let secret = |name: &str| secret(&get, name);

        let listen = get("YARDSORT_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.into());
        let listen = listen
            .parse()
            .map_err(|e| ConfigError(format!("YARDSORT_LISTEN {listen:?}: {e}")))?;
        let public_url = required("YARDSORT_PUBLIC_URL")?;
        let public_url = Url::parse(&public_url)
            .map_err(|e| ConfigError(format!("YARDSORT_PUBLIC_URL {public_url:?}: {e}")))?;
        let device_redirects = match get("YARDSORT_DEVICE_REDIRECTS") {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned)
                .collect(),
            None => DEFAULT_DEVICE_REDIRECTS
                .iter()
                .map(|r| (*r).to_owned())
                .collect(),
        };
        let client_ip_header = get("YARDSORT_CLIENT_IP_HEADER")
            .map(|name| {
                HeaderName::from_bytes(name.trim().as_bytes())
                    .map_err(|e| ConfigError(format!("YARDSORT_CLIENT_IP_HEADER {name:?}: {e}")))
            })
            .transpose()?;
        let routing = match get("YARDSORT_ROUTING").as_deref().map(str::trim) {
            None | Some("single") => Routing::Single,
            Some("fly-replay") => Routing::FlyReplay,
            Some(other) => {
                return Err(ConfigError(format!(
                    "YARDSORT_ROUTING {other:?}: single or fly-replay"
                )))
            }
        };
        let instance = match get("YARDSORT_INSTANCE").or_else(|| get("FLY_MACHINE_ID")) {
            Some(name) => Instance::new(name.trim())
                .map_err(|e| ConfigError(format!("YARDSORT_INSTANCE: {e}")))?,
            None => Instance::random(),
        };
        Ok(Self {
            client_ip_header,
            routing,
            instance,
            listen,
            name: get("YARDSORT_SERVER_NAME").unwrap_or_else(|| DEFAULT_NAME.into()),
            public_url,
            provider: ProviderConfig {
                issuer: required("YARDSORT_OIDC_ISSUER")?,
                client_id: required("YARDSORT_OIDC_CLIENT_ID")?,
                client_secret: secret("YARDSORT_OIDC_CLIENT_SECRET")?,
            },
            device_redirects,
            database_url: database_url(&get)?,
            push: push_settings(&get),
            entitlements: entitlements_settings(&get)?,
            journal: journal_settings(&get)?,
        })
    }
}

fn journal_settings(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<JournalSettings>, ConfigError> {
    let dir = get("YARDSORT_JOURNAL_DIR");
    let bucket = get("YARDSORT_JOURNAL_BUCKET").or_else(|| get("BUCKET_NAME"));
    match (dir, bucket) {
        (Some(_), Some(_)) => Err(ConfigError(
            "set YARDSORT_JOURNAL_DIR or a bucket (YARDSORT_JOURNAL_BUCKET or BUCKET_NAME), not \
             both"
                .into(),
        )),
        (Some(dir), None) => Ok(Some(JournalSettings::Dir(dir.trim().into()))),
        (None, None) => Ok(None),
        (None, Some(bucket)) => {
            let need = |name: &str| {
                get(name).ok_or_else(|| {
                    ConfigError(format!("the journal's bucket {bucket:?} needs {name}"))
                })
            };
            let endpoint = need("AWS_ENDPOINT_URL_S3")?;
            let endpoint = Url::parse(endpoint.trim())
                .map_err(|e| ConfigError(format!("AWS_ENDPOINT_URL_S3 {endpoint:?}: {e}")))?;
            let loopback = matches!(endpoint.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                || matches!(endpoint.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
            if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
                return Err(ConfigError(format!(
                    "AWS_ENDPOINT_URL_S3 {:?} is not HTTPS (plain HTTP is accepted on a loopback \
                     address only)",
                    endpoint.as_str()
                )));
            }
            let access_key = need("AWS_ACCESS_KEY_ID")?;
            let secret_key = secret(get, "AWS_SECRET_ACCESS_KEY")?.ok_or_else(|| {
                ConfigError(format!(
                    "the journal's bucket {bucket:?} needs AWS_SECRET_ACCESS_KEY"
                ))
            })?;
            let addressing = match get("YARDSORT_JOURNAL_ADDRESSING").as_deref().map(str::trim) {
                None => crate::journal::Addressing::for_endpoint(&endpoint),
                Some("virtual") => crate::journal::Addressing::Virtual,
                Some("path") => crate::journal::Addressing::Path,
                Some(other) => {
                    return Err(ConfigError(format!(
                        "YARDSORT_JOURNAL_ADDRESSING {other:?}: virtual or path"
                    )))
                }
            };
            Ok(Some(JournalSettings::Bucket {
                endpoint,
                bucket: bucket.trim().to_owned(),
                addressing,
                region: get("AWS_REGION").unwrap_or_else(|| "auto".into()),
                access_key: access_key.trim().to_owned(),
                secret_key,
            }))
        }
    }
}

fn entitlements_settings(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<EntitlementsSettings>, ConfigError> {
    let token = secret(get, "YARDSORT_ENTITLEMENTS_TOKEN")?;
    let Some(url) = get("YARDSORT_ENTITLEMENTS_URL") else {
        return match token {
            Some(_) => Err(ConfigError(
                "YARDSORT_ENTITLEMENTS_TOKEN is set without YARDSORT_ENTITLEMENTS_URL".into(),
            )),
            None => Ok(None),
        };
    };
    let url = Url::parse(url.trim())
        .map_err(|e| ConfigError(format!("YARDSORT_ENTITLEMENTS_URL {url:?}: {e}")))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => {
            return Err(ConfigError(format!(
                "YARDSORT_ENTITLEMENTS_URL {:?} is not HTTPS (plain HTTP is accepted on a \
                 loopback address only)",
                url.as_str()
            )))
        }
    }
    Ok(Some(EntitlementsSettings { url, token }))
}

fn push_settings(get: &impl Fn(&str) -> Option<String>) -> PushSettings {
    let apns = apns_settings(get);
    let fcm = fcm_settings(get);
    PushSettings { apns, fcm }
}

fn apns_settings(get: &impl Fn(&str) -> Option<String>) -> Option<crate::push::Apns> {
    let key_id = filled(get, "YARDSORT_APNS_KEY_ID");
    let team_id = filled(get, "YARDSORT_APNS_TEAM_ID");
    let bundle_id = filled(get, "YARDSORT_APNS_BUNDLE_ID");
    let key = push_secret(get, "YARDSORT_APNS_KEY");
    if key_id.is_none() && team_id.is_none() && bundle_id.is_none() && key.is_none() {
        return None;
    }
    let (Some(key_id), Some(team_id), Some(bundle_id), Some(key)) =
        (key_id, team_id, bundle_id, key)
    else {
        tracing::warn!(
            "push: APNs needs YARDSORT_APNS_KEY_ID, YARDSORT_APNS_TEAM_ID, \
             YARDSORT_APNS_BUNDLE_ID and YARDSORT_APNS_KEY"
        );
        return None;
    };
    let sandbox = matches!(
        get("YARDSORT_APNS_SANDBOX").as_deref(),
        Some("1" | "true" | "yes")
    );
    match crate::push::Apns::parse(key_id, team_id, bundle_id, sandbox, key.expose()) {
        Some(apns) => Some(apns),
        None => {
            tracing::warn!("push: the APNs key could not be read");
            None
        }
    }
}

fn fcm_settings(get: &impl Fn(&str) -> Option<String>) -> Option<crate::push::Fcm> {
    let credentials = push_secret(get, "YARDSORT_FCM_CREDENTIALS");
    let project = filled(get, "YARDSORT_FCM_PROJECT_ID");
    if credentials.is_none() && project.is_none() {
        return None;
    }
    let Some(credentials) = credentials else {
        tracing::warn!("push: FCM needs YARDSORT_FCM_CREDENTIALS");
        return None;
    };
    let Some(mut fcm) = crate::push::Fcm::parse(credentials.expose()) else {
        tracing::warn!("push: the FCM credentials could not be read");
        return None;
    };
    if let Some(project) = project {
        fcm.project_id = project;
    }
    Some(fcm)
}

/// What the operator's commands need: the database, the provider the server signs in with, and
/// the journal, if they are set.
pub struct AdminSettings {
    pub database_url: Secret,
    pub issuer: Option<String>,
    pub journal: Option<JournalSettings>,
}

impl AdminSettings {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |name: &str| get(name).filter(|value| !value.trim().is_empty());
        Ok(Self {
            database_url: database_url(&get)?,
            issuer: get("YARDSORT_OIDC_ISSUER"),
            journal: journal_settings(&get)?,
        })
    }
}

/// A push secret from the environment. A blank value is unset, so a compose file can pass
/// `${VAR:-}` and leave the provider off until the operator fills it in.
fn push_secret(get: &impl Fn(&str) -> Option<String>, name: &str) -> Option<Secret> {
    let value = filled(get, name);
    let file = filled(get, &format!("{name}_FILE"));
    match (value, file) {
        (None, None) => None,
        (Some(_), Some(_)) => {
            tracing::warn!("push: set {name} or {name}_FILE, not both");
            None
        }
        (Some(value), None) => Some(Secret::new(value)),
        (None, Some(path)) => match std::fs::read_to_string(path.trim()) {
            Ok(text) => Some(Secret::new(text.trim_end().to_owned())),
            Err(error) => {
                tracing::warn!("push: {name}_FILE: {error}");
                None
            }
        },
    }
}

fn filled(get: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    get(name).filter(|text| !text.trim().is_empty())
}

/// `name`, or the contents of the file `{name}_FILE` names.
fn secret(
    get: &impl Fn(&str) -> Option<String>,
    name: &str,
) -> Result<Option<Secret>, ConfigError> {
    let file = format!("{name}_FILE");
    match (get(name), get(&file)) {
        (Some(_), Some(_)) => Err(ConfigError(format!("set {name} or {file}, not both"))),
        (Some(value), None) => Ok(Some(Secret::new(value))),
        (None, Some(path)) => std::fs::read_to_string(path.trim())
            .map(|text| Some(Secret::new(text.trim_end())))
            .map_err(|e| ConfigError(format!("{file}: {e}"))),
        (None, None) => Ok(None),
    }
}

fn database_url(get: &impl Fn(&str) -> Option<String>) -> Result<Secret, ConfigError> {
    secret(get, "DATABASE_URL")?.ok_or_else(|| ConfigError("DATABASE_URL is not set".into()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Result<Settings, ConfigError> {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Settings::from_lookup(|name| env.get(name).cloned())
    }

    const MINIMAL: &[(&str, &str)] = &[
        ("YARDSORT_PUBLIC_URL", "https://remote.example.invalid"),
        ("YARDSORT_OIDC_ISSUER", "https://id.example.invalid"),
        ("YARDSORT_OIDC_CLIENT_ID", "client"),
        ("DATABASE_URL", "postgres://u:the-db-password@db/yardsort"),
    ];

    #[test]
    fn the_minimum_and_the_defaults() {
        let s = settings(MINIMAL).unwrap();
        assert_eq!(s.listen.to_string(), DEFAULT_LISTEN);
        assert_eq!(s.name, DEFAULT_NAME);
        assert_eq!(s.device_redirects, DEFAULT_DEVICE_REDIRECTS);
        assert!(s.provider.client_secret.is_none());
        for missing in MINIMAL {
            let without: Vec<_> = MINIMAL.iter().filter(|p| p != &missing).copied().collect();
            let error = settings(&without).unwrap_err().to_string();
            assert!(error.contains(missing.0), "{error}");
        }
    }

    #[test]
    fn entitlements_are_off_unless_a_secure_url_is_given() {
        assert!(settings(MINIMAL).unwrap().entitlements.is_none());
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut pairs = MINIMAL.to_vec();
            pairs.extend_from_slice(extra);
            settings(&pairs)
        };
        let on = with(&[
            (
                "YARDSORT_ENTITLEMENTS_URL",
                "https://plans.example.invalid/ask",
            ),
            ("YARDSORT_ENTITLEMENTS_TOKEN", "the-entitlements-token"),
        ])
        .unwrap();
        let entitlements = on.entitlements.as_ref().unwrap();
        assert_eq!(
            entitlements.url.as_str(),
            "https://plans.example.invalid/ask"
        );
        assert_eq!(
            entitlements.token.as_ref().unwrap().expose(),
            "the-entitlements-token"
        );
        assert!(!format!("{on:?}").contains("the-entitlements-token"));
        assert!(
            with(&[("YARDSORT_ENTITLEMENTS_URL", "http://127.0.0.1:9/ask")])
                .unwrap()
                .entitlements
                .is_some()
        );
        let plain = with(&[("YARDSORT_ENTITLEMENTS_URL", "http://plans.example.invalid/")]);
        assert!(plain.unwrap_err().to_string().contains("not HTTPS"));
        let alone = with(&[("YARDSORT_ENTITLEMENTS_TOKEN", "t")]);
        assert!(alone.unwrap_err().to_string().contains("without"));
    }

    #[test]
    fn a_secret_comes_from_a_file_and_is_never_printed() {
        let dir = std::env::temp_dir().join(format!("ys-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret");
        std::fs::write(&file, "the-client-secret\n").unwrap();
        let path = file.to_string_lossy().into_owned();
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("YARDSORT_OIDC_CLIENT_SECRET_FILE", &path));
        let s = settings(&pairs).unwrap();
        assert_eq!(
            s.provider.client_secret.as_ref().map(Secret::expose),
            Some("the-client-secret")
        );
        let printed = format!("{s:?}");
        assert!(!printed.contains("the-client-secret"), "{printed}");
        assert!(!printed.contains("the-db-password"), "{printed}");

        pairs.push(("YARDSORT_OIDC_CLIENT_SECRET", "another"));
        assert!(settings(&pairs).is_err(), "both ways at once is refused");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn routing_and_the_instance_name() {
        let s = settings(MINIMAL).unwrap();
        assert_eq!(s.routing, Routing::Single);
        assert_ne!(
            s.instance,
            settings(MINIMAL).unwrap().instance,
            "made up at each start"
        );

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("YARDSORT_ROUTING", "fly-replay"));
        pairs.push(("FLY_MACHINE_ID", "148e21eae69398"));
        let s = settings(&pairs).unwrap();
        assert_eq!(s.routing, Routing::FlyReplay);
        assert_eq!(
            s.instance.as_str(),
            "148e21eae69398",
            "Fly.io's name for it"
        );
        pairs.push(("YARDSORT_INSTANCE", "b"));
        assert_eq!(
            settings(&pairs).unwrap().instance.as_str(),
            "b",
            "named here first"
        );

        for (name, bad) in [
            ("YARDSORT_ROUTING", "round-robin"),
            ("YARDSORT_INSTANCE", "a/b"),
        ] {
            let mut pairs = MINIMAL.to_vec();
            pairs.push((name, bad));
            let error = settings(&pairs).unwrap_err().to_string();
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn redirects_are_a_list() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push((
            "YARDSORT_DEVICE_REDIRECTS",
            " yardsort://signed-in , http://127.0.0.1/cb,",
        ));
        assert_eq!(
            settings(&pairs).unwrap().device_redirects,
            ["yardsort://signed-in", "http://127.0.0.1/cb"]
        );
    }

    #[test]
    fn a_journal_is_a_directory_or_fly_storage_bucket() {
        assert!(settings(MINIMAL).unwrap().journal.is_none());

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("YARDSORT_JOURNAL_DIR", "/var/lib/yardsort/journal"));
        assert!(matches!(
            settings(&pairs).unwrap().journal,
            Some(JournalSettings::Dir(dir)) if dir == std::path::Path::new("/var/lib/yardsort/journal")
        ));

        // As `fly storage create` leaves the app's secrets.
        let mut fly = MINIMAL.to_vec();
        fly.extend([
            ("BUCKET_NAME", "yardsort-journal-staging"),
            ("AWS_ENDPOINT_URL_S3", "https://fly.storage.tigris.dev"),
            ("AWS_ACCESS_KEY_ID", "tid_key"),
            ("AWS_SECRET_ACCESS_KEY", "tsec_the-bucket-secret"),
        ]);
        let s = settings(&fly).unwrap();
        let Some(JournalSettings::Bucket {
            endpoint,
            bucket,
            addressing,
            region,
            access_key,
            secret_key,
        }) = &s.journal
        else {
            panic!("a bucket: {:?}", s.journal);
        };
        assert_eq!(endpoint.as_str(), "https://fly.storage.tigris.dev/");
        assert_eq!(bucket, "yardsort-journal-staging");
        assert_eq!(region, "auto");
        assert_eq!(*addressing, crate::journal::Addressing::Virtual);
        assert_eq!(access_key, "tid_key");
        assert_eq!(secret_key.expose(), "tsec_the-bucket-secret");
        let printed = format!("{s:?}");
        assert!(printed.contains("yardsort-journal-staging"), "{printed}");
        assert!(!printed.contains("tsec_the-bucket-secret"), "{printed}");

        // Its own name wins over Fly's.
        let mut named = fly.clone();
        named.push(("YARDSORT_JOURNAL_BUCKET", "another"));
        assert!(matches!(
            settings(&named).unwrap().journal,
            Some(JournalSettings::Bucket { bucket, .. }) if bucket == "another"
        ));

        for missing in [
            "AWS_ENDPOINT_URL_S3",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            let without: Vec<_> = fly.iter().filter(|(k, _)| *k != missing).copied().collect();
            let error = settings(&without).unwrap_err().to_string();
            assert!(error.contains(missing), "{error}");
        }
        let mut plain = fly.clone();
        plain.retain(|(k, _)| *k != "AWS_ENDPOINT_URL_S3");
        plain.push(("AWS_ENDPOINT_URL_S3", "http://storage.example.invalid"));
        assert!(settings(&plain)
            .unwrap_err()
            .to_string()
            .contains("not HTTPS"));
        plain.retain(|(k, _)| *k != "AWS_ENDPOINT_URL_S3");
        plain.push(("AWS_ENDPOINT_URL_S3", "http://127.0.0.1:9000"));
        assert!(
            matches!(
                settings(&plain).unwrap().journal,
                Some(JournalSettings::Bucket {
                    addressing: crate::journal::Addressing::Path,
                    ..
                })
            ),
            "plain HTTP to this machine, by path"
        );
        plain.push(("YARDSORT_JOURNAL_ADDRESSING", "virtual"));
        assert!(matches!(
            settings(&plain).unwrap().journal,
            Some(JournalSettings::Bucket {
                addressing: crate::journal::Addressing::Virtual,
                ..
            })
        ));
        plain.retain(|(k, _)| *k != "YARDSORT_JOURNAL_ADDRESSING");
        plain.push(("YARDSORT_JOURNAL_ADDRESSING", "sideways"));
        assert!(settings(&plain)
            .unwrap_err()
            .to_string()
            .contains("virtual or path"));

        let mut both = fly;
        both.push(("YARDSORT_JOURNAL_DIR", "/journal"));
        assert!(settings(&both)
            .unwrap_err()
            .to_string()
            .contains("not both"));
    }
}

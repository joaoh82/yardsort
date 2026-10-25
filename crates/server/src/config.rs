//! The server's settings, from its environment. Each secret can come from a file instead
//! (`…_FILE`), as container platforms mount them; a secret never comes from the command line, where
//! any process on the machine can read it.
//!
//! - `YARDSORT_PUBLIC_URL`: the server's own address, as browsers reach it.
//! - `YARDSORT_OIDC_ISSUER`: the identity provider, as its issuer URL.
//! - `YARDSORT_OIDC_CLIENT_ID`: this server's client id at the provider.
//! - `YARDSORT_OIDC_CLIENT_SECRET` (or `_FILE`): its client secret. Unset for a public PKCE client.
//! - `DATABASE_URL` (or `_FILE`): PostgreSQL, as `postgres://…`.
//! - `YARDSORT_LISTEN`: the address to listen on; [`DEFAULT_LISTEN`] unless set.
//! - `YARDSORT_SERVER_NAME`: what people see it called; [`DEFAULT_NAME`] unless set.
//! - `YARDSORT_DEVICE_REDIRECTS`: comma-separated; [`DEFAULT_DEVICE_REDIRECTS`] unless set.
//! - `YARDSORT_CLIENT_IP_HEADER`: behind a proxy, the header it names the client's address in
//!   (`Fly-Client-IP`), for rate limits.

use std::net::SocketAddr;

use axum::http::HeaderName;
use remote_proto::account::Secret;
use url::Url;

use crate::oidc::ProviderConfig;

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
        Ok(Self {
            client_ip_header,
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
        })
    }
}

/// What `yardsort-server revoke-account` needs: the database, and the provider the server signs
/// in with, if it is set.
pub struct AdminSettings {
    pub database_url: Secret,
    pub issuer: Option<String>,
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
        })
    }
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
}

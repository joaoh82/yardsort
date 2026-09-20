//! Where the TypeSafe API key lives.
//!
//! Never in `settings.toml`: that file is meant to be read, diffed, backed up and shared. The key
//! goes into the OS credential store — the Keychain on macOS, Credential Manager on Windows, the
//! Secret Service (GNOME Keyring, KWallet…) on Linux. `TYPESAFE_API_KEY` in the environment is
//! the fallback for systems without one, and the variable TypeSafe's own tools read.
//!
//! The key goes one way only: the webview can set it or forget it, never read it back.

use serde::Serialize;
use specta::Type;

const SERVICE: &str = "dev.yardsort.app";
const ACCOUNT: &str = "typesafe-api-key";
pub const ENV_VAR: &str = "TYPESAFE_API_KEY";

/// Somewhere a key can be kept. The real one is [`Keychain`]; tests use memory.
pub trait KeyStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, key: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

pub struct Keychain;

impl Keychain {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|error| error.to_string())
    }
}

impl KeyStore for Keychain {
    fn get(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, key: &str) -> Result<(), String> {
        Self::entry()?
            .set_password(key)
            .map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    None,
    /// Saved from Settings, in the OS credential store.
    Keychain,
    /// `TYPESAFE_API_KEY`, from the environment Yardsort runs in.
    Environment,
}

/// The key in force and where it came from. The credential store wins: a key typed into Settings
/// is the user's latest word.
pub fn resolve(
    store: &dyn KeyStore,
    env_value: Option<&str>,
) -> (Option<String>, KeySource, Option<String>) {
    let (stored, problem) = match store.get() {
        Ok(key) => (key, None),
        Err(error) => (None, Some(unavailable(&error))),
    };
    let usable = |key: Option<&str>| {
        key.map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_owned)
    };
    if let Some(key) = usable(stored.as_deref()) {
        return (Some(key), KeySource::Keychain, None);
    }
    match usable(env_value) {
        Some(key) => (Some(key), KeySource::Environment, problem),
        None => (None, KeySource::None, problem),
    }
}

pub fn unavailable(error: &str) -> String {
    format!(
        "The system credential store is not available ({error}). Start one (for example \
         gnome-keyring), or set {ENV_VAR} in your environment instead."
    )
}

/// The last four characters, so people can tell keys apart without the key being shown.
pub fn hint(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

#[cfg(test)]
pub struct MemoryStore(pub std::sync::Mutex<Result<Option<String>, String>>);

#[cfg(test)]
impl MemoryStore {
    pub fn empty() -> Self {
        Self(std::sync::Mutex::new(Ok(None)))
    }
    pub fn broken() -> Self {
        Self(std::sync::Mutex::new(Err("no secret service".into())))
    }
}

#[cfg(test)]
impl KeyStore for MemoryStore {
    fn get(&self) -> Result<Option<String>, String> {
        self.0.lock().unwrap().clone()
    }
    fn set(&self, key: &str) -> Result<(), String> {
        let mut slot = self.0.lock().unwrap();
        slot.clone()?;
        *slot = Ok(Some(key.to_owned()));
        Ok(())
    }
    fn delete(&self) -> Result<(), String> {
        let mut slot = self.0.lock().unwrap();
        slot.clone()?;
        *slot = Ok(None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_key_wins_over_the_environment() {
        let store = MemoryStore::empty();
        assert_eq!(resolve(&store, None), (None, KeySource::None, None));
        assert_eq!(
            resolve(&store, Some(" ts-env ")),
            (Some("ts-env".into()), KeySource::Environment, None)
        );
        store.set("ts-saved").unwrap();
        assert_eq!(
            resolve(&store, Some("ts-env")),
            (Some("ts-saved".into()), KeySource::Keychain, None)
        );
        store.delete().unwrap();
        assert_eq!(resolve(&store, Some("")).1, KeySource::None);
    }

    #[test]
    fn without_a_credential_store_the_environment_still_works_and_the_problem_is_named() {
        let store = MemoryStore::broken();
        let (key, source, problem) = resolve(&store, Some("ts-env"));
        assert_eq!(
            (key.as_deref(), source),
            (Some("ts-env"), KeySource::Environment)
        );
        assert!(problem.unwrap().contains(ENV_VAR));
    }

    #[test]
    fn a_hint_shows_only_the_end_of_the_key() {
        assert_eq!(hint("ts-live-abcdef1234"), "…1234");
        assert_eq!(hint("ab"), "…ab");
    }
}

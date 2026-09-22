//! Assist: small, typed judgments from TypeSafe's Jev model, used where ordinary code cannot
//! tell what a change *means*.
//!
//! Two rules shape everything here. Nothing leaves the machine unless the user has entered an API
//! key **and** switched the feature on — each feature says what it sends. And nothing here is
//! load-bearing: with Assist off, unreachable or rate-limited, Yardsort works exactly as before,
//! minus a few badges.
//!
//! What Jev is asked stays narrow (one property per question) and what its answers *mean* stays
//! in code: the thresholds live in `review` and `suggest`, never in the model.

pub mod commands;
pub mod jev;
pub mod key;
pub mod review;
pub mod suggest;

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use key::{KeySource, KeyStore};

// The thresholds are written into `settings.toml`, so they live in the core beside the settings
// that carry them.
pub use yardsort_core::assist::Thresholds;

/// Answers for inputs that have not changed, so watching an agent work does not re-ask the same
/// questions. Dropped whole once it grows past a few thousand entries — it is a cache, not a store.
pub struct Cache<T>(Mutex<HashMap<u64, T>>);

const MAX_CACHED: usize = 4000;

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

impl<T: Clone> Cache<T> {
    pub fn get(&self, key: u64) -> Option<T> {
        self.lock().get(&key).cloned()
    }

    pub fn put(&self, key: u64, value: T) {
        let mut entries = self.lock();
        if entries.len() >= MAX_CACHED {
            entries.clear();
        }
        entries.insert(key, value);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, T>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Everything Assist keeps between calls: where the key lives, and what has been answered.
///
/// The cache holds Jev's *answers*, not the badges they led to, so moving a threshold re-reads
/// what the model already said instead of asking it again.
pub struct Assist {
    store: Box<dyn KeyStore>,
    pub reviews: Cache<jev::Answers>,
}

impl Default for Assist {
    fn default() -> Self {
        Self::with_store(Box::new(key::Keychain))
    }
}

impl Assist {
    pub fn with_store(store: Box<dyn KeyStore>) -> Self {
        Self {
            store,
            reviews: Cache::default(),
        }
    }

    /// The key in force, where it came from, and what is wrong with the credential store if
    /// anything is. `env_key` is `TYPESAFE_API_KEY` from the environment Yardsort runs in.
    pub fn key(&self, env_key: Option<&str>) -> (Option<String>, KeySource, Option<String>) {
        key::resolve(self.store.as_ref(), env_key)
    }

    pub fn save_key(&self, value: &str) -> Result<(), String> {
        self.store
            .set(value.trim())
            .map_err(|e| key::unavailable(&e))
    }

    pub fn forget_key(&self) -> Result<(), String> {
        self.store.delete().map_err(|e| key::unavailable(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_answers_what_it_was_given_and_never_grows_without_bound() {
        let cache: Cache<u32> = Cache::default();
        assert_eq!(cache.get(1), None);
        cache.put(1, 42);
        assert_eq!(cache.get(1), Some(42));
        for n in 0..MAX_CACHED as u64 {
            cache.put(n, 1);
        }
        assert!(cache.lock().len() <= MAX_CACHED);
    }

    #[test]
    fn a_key_can_be_saved_and_forgotten() {
        let assist = Assist::with_store(Box::new(key::MemoryStore::empty()));
        assert_eq!(assist.key(None).1, KeySource::None);
        assist.save_key("  ts-live-1234  ").unwrap();
        assert_eq!(
            assist.key(Some("ts-env")),
            (Some("ts-live-1234".into()), KeySource::Keychain, None)
        );
        assist.forget_key().unwrap();
        assert_eq!(assist.key(None).0, None);
    }

    #[test]
    fn a_credential_store_that_is_not_there_is_explained_not_swallowed() {
        let assist = Assist::with_store(Box::new(key::MemoryStore::broken()));
        let problem = assist.save_key("ts-live-1234").unwrap_err();
        assert!(problem.contains("credential store"), "{problem}");
        assert!(problem.contains(key::ENV_VAR), "{problem}");
    }
}

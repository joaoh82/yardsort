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

use serde::{Deserialize, Serialize};

use key::{KeySource, KeyStore};

/// How sure Jev must be before Yardsort acts on an answer, as whole percentages — the numbers
/// that decide whether a probability becomes a badge or a suggestion.
///
/// They are settings, not constants, because the right values depend on the code being written
/// and on how much noise a person will put up with. Raw answers are cached, so moving a
/// threshold re-reads what Jev already said instead of asking again.
/// The names here are the ones written into `settings.toml`, so they stay snake_case; the
/// webview sees them through `commands::ThresholdsDto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    /// A risk question (secret, tests, checks) becomes a badge at or above this probability.
    pub flag_at_percent: u8,
    /// "Unrelated to the task" needs at least this much of the probability mass to be a badge.
    pub off_task_at_percent: u8,
    /// A composer suggestion is offered only when the model is at least this sure.
    pub suggest_at_percent: u8,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            flag_at_percent: 70,
            off_task_at_percent: 60,
            suggest_at_percent: 50,
        }
    }
}

impl Thresholds {
    /// The lowest and highest a threshold may be: certainty is never 0% or 100%, and a badge that
    /// always or never appears says nothing.
    pub const RANGE: std::ops::RangeInclusive<u8> = 5..=95;

    /// The message to show if these cannot be saved.
    pub fn validate(&self) -> Result<(), String> {
        let ok = |value: u8| Self::RANGE.contains(&value);
        if ok(self.flag_at_percent) && ok(self.off_task_at_percent) && ok(self.suggest_at_percent) {
            Ok(())
        } else {
            Err(format!(
                "Thresholds must be between {}% and {}%.",
                Self::RANGE.start(),
                Self::RANGE.end()
            ))
        }
    }

    pub fn flag_at(&self) -> f64 {
        f64::from(self.flag_at_percent) / 100.0
    }

    pub fn off_task_at(&self) -> f64 {
        f64::from(self.off_task_at_percent) / 100.0
    }

    pub fn suggest_at(&self) -> f64 {
        f64::from(self.suggest_at_percent) / 100.0
    }
}

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

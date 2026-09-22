//! Assist's certainty thresholds.
//!
//! They live here, away from the Jev client that uses them, because `settings.toml` carries
//! them and the settings file is part of the core every client reads.

use serde::{Deserialize, Serialize};

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

//! What a model's tokens would cost at its vendor's published API rate.
//!
//! This is an estimate for comparison, not a bill: most people run these agents on a
//! subscription, which is not charged per token at all. The table is the vendors' standard tier
//! in US dollars per million tokens, as published on the dates below. A model that is not here
//! has its tokens counted and its cost left unknown, never guessed.
//!
//! - Anthropic, 2026-09-25: input and output per model; cache reads per model (they differ);
//!   cache writes at 1.25× input for the five-minute cache and 2× for the hour-long one.
//! - OpenAI, 2026-10-01: input, cached input and output. Reasoning tokens are part of output.

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
    /// Writing to the five-minute cache, where the vendor charges for it separately.
    pub cache_write_5m: f64,
    /// Writing to the hour-long cache.
    pub cache_write_1h: f64,
}

const fn anthropic(input: f64, cache_read: f64, output: f64) -> Price {
    Price {
        input,
        cache_read,
        output,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
    }
}

/// OpenAI charges nothing extra to write its cache: those tokens are ordinary input.
const fn openai(input: f64, cache_read: f64, output: f64) -> Price {
    Price {
        input,
        cache_read,
        output,
        cache_write_5m: input,
        cache_write_1h: input,
    }
}

const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", anthropic(10.0, 0.25, 50.0)),
    ("claude-mythos-5-1", anthropic(10.0, 0.25, 50.0)),
    ("claude-fable-5", anthropic(10.0, 1.0, 50.0)),
    ("claude-mythos-5", anthropic(10.0, 1.0, 50.0)),
    ("claude-opus-5-5", anthropic(4.0, 0.20, 20.0)),
    ("claude-opus-5", anthropic(5.0, 0.50, 25.0)),
    ("claude-opus-4-8", anthropic(5.0, 0.50, 25.0)),
    ("claude-opus-4-7", anthropic(5.0, 0.50, 25.0)),
    ("claude-opus-4-6", anthropic(5.0, 0.50, 25.0)),
    ("claude-sonnet-5-5", anthropic(2.0, 0.20, 10.0)),
    ("claude-sonnet-5", anthropic(2.0, 0.20, 10.0)),
    ("claude-sonnet-4-6", anthropic(3.0, 0.30, 15.0)),
    ("claude-haiku-4-5", anthropic(1.0, 0.10, 5.0)),
    ("gpt-6-astra", openai(10.0, 1.0, 50.0)),
    ("gpt-6.1-sol", openai(2.0, 0.10, 10.0)),
    ("gpt-6-sol", openai(2.0, 0.20, 10.0)),
    ("gpt-6-luna", openai(0.10, 0.01, 0.50)),
    ("gpt-5.6-sol", openai(4.0, 0.40, 20.0)),
    ("gpt-5.6-terra", openai(2.0, 0.20, 12.0)),
    ("gpt-5.6-luna", openai(0.20, 0.02, 1.20)),
    ("gpt-5.5", openai(5.0, 0.50, 30.0)),
    ("gpt-5.4", openai(2.50, 0.25, 15.0)),
    ("gpt-5.4-mini", openai(0.75, 0.075, 4.50)),
    ("gpt-5.4-nano", openai(0.20, 0.02, 1.25)),
    ("gpt-5.3-codex", openai(1.75, 0.175, 14.0)),
    ("gpt-5.2", openai(1.75, 0.175, 14.0)),
    ("gpt-5.1", openai(1.25, 0.125, 10.0)),
    ("gpt-5", openai(1.25, 0.125, 10.0)),
    ("gpt-5-mini", openai(0.25, 0.025, 2.0)),
    ("gpt-5-nano", openai(0.05, 0.005, 0.40)),
];

/// The price of a model as an agent names it. Agents decorate the id — a provider in front
/// (`anthropic/…`), a date behind (`…-20251001`), a context size in brackets (`…[1m]`) — so
/// those are taken off; what is left must then match exactly. A near miss is not a match:
/// `gpt-5.4-pro` costs ten times `gpt-5.4`.
pub fn price(model: &str) -> Option<Price> {
    let id = normalize(model);
    PRICES
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, price)| *price)
}

fn normalize(model: &str) -> String {
    let mut id = model.trim().to_ascii_lowercase();
    if let Some((_, rest)) = id.rsplit_once('/') {
        id = rest.to_owned();
    }
    if let Some(open) = id.find('[') {
        id.truncate(open);
    }
    // A trailing `-YYYYMMDD` snapshot date.
    if let Some((base, date)) = id.rsplit_once('-') {
        if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) {
            id = base.to_owned();
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorated_ids_find_their_model() {
        assert_eq!(price("claude-opus-5-5"), price("anthropic/claude-opus-5-5"));
        assert_eq!(price("claude-haiku-4-5-20251001").unwrap().input, 1.0);
        assert_eq!(price("claude-opus-5-5[1m]").unwrap().output, 20.0);
        assert_eq!(price("GPT-6-Astra").unwrap().cache_read, 1.0);
    }

    #[test]
    fn a_near_miss_is_unknown_rather_than_priced_like_its_neighbour() {
        assert_eq!(price("gpt-5.4-pro"), None);
        assert_eq!(price("grok-4.7-build"), None);
        assert_eq!(price("claude-opus-5-5-fast"), None);
        assert_eq!(price(""), None);
    }

    #[test]
    fn anthropic_cache_writes_cost_more_than_input_and_openai_ones_do_not() {
        let claude = price("claude-sonnet-5-5").unwrap();
        assert_eq!(claude.cache_write_5m, 2.5);
        assert_eq!(claude.cache_write_1h, 4.0);
        let gpt = price("gpt-6-sol").unwrap();
        assert_eq!(gpt.cache_write_5m, gpt.input);
    }
}

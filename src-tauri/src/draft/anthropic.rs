//! The Anthropic Messages API, used only when no configured agent can do the writing.
//!
//! Raw HTTP rather than an SDK because there is no official Rust one, and because the one call
//! Yardsort makes is a single non-streaming request — the same shape `assist::jev` already uses
//! for TypeSafe.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

const BASE_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version header every request must carry.
const VERSION: &str = "2023-06-01";
const TIMEOUT: Duration = Duration::from_secs(60);

/// Deliberately short outputs: a commit message is a few lines and a pull request description a
/// few paragraphs. Nothing here streams, so this also keeps the request inside the timeout.
const MAX_TOKENS: u32 = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum AnthropicError {
    #[error("could not reach the Anthropic API: {0}")]
    Unreachable(String),
    #[error("the Anthropic API key was not accepted")]
    BadKey,
    #[error("the Anthropic API is rate limiting this key; try again shortly")]
    RateLimited,
    #[error("the Anthropic API said: {0}")]
    Failed(String),
    #[error("the model declined to write this{0}")]
    Refused(String),
    #[error("the Anthropic API sent something unexpected: {0}")]
    Unreadable(String),
}

impl AnthropicError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable(_) => "anthropic_unreachable",
            Self::BadKey => "anthropic_bad_key",
            Self::RateLimited => "anthropic_rate_limited",
            Self::Failed(_) => "anthropic_failed",
            Self::Refused(_) => "anthropic_refused",
            Self::Unreadable(_) => "anthropic_unreadable",
        }
    }
}

type Result<T> = std::result::Result<T, AnthropicError>;

/// One Messages API response, as much of it as one short non-streaming call needs.
#[derive(Debug, Deserialize)]
struct Reply {
    content: Vec<Block>,
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
}

#[derive(Debug, Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct StopDetails {
    #[serde(default)]
    category: Option<String>,
}

/// Ask the model to write something, and return what it wrote.
///
/// `effort` is `low`: this is a short writing task, and lowering effort is the documented way to
/// spend less on one — rather than turning thinking off, which on this family can put a tool call
/// into the visible text or leak reasoning tags into the answer.
pub async fn write(key: &str, model: &str, system: &str, prompt: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| AnthropicError::Unreachable(error.to_string()))?;

    let response = client
        .post(BASE_URL)
        .header("x-api-key", key)
        .header("anthropic-version", VERSION)
        .json(&json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "output_config": { "effort": "low" },
            "system": system,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|error| AnthropicError::Unreachable(error.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AnthropicError::Unreachable(error.to_string()))?;

    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => AnthropicError::BadKey,
            429 => AnthropicError::RateLimited,
            _ => AnthropicError::Failed(message_in(&body).unwrap_or_else(|| status.to_string())),
        });
    }

    let reply: Reply =
        serde_json::from_str(&body).map_err(|e| AnthropicError::Unreadable(e.to_string()))?;
    text_of(&reply)
}

/// The text the model wrote — checking first that it wrote any.
///
/// A refusal arrives as an ordinary 200 with `stop_reason: "refusal"` and empty content, so the
/// stop reason is read before the content rather than after.
fn text_of(reply: &Reply) -> Result<String> {
    if reply.stop_reason.as_deref() == Some("refusal") {
        let why = reply
            .stop_details
            .as_ref()
            .and_then(|details| details.category.as_deref())
            .map(|category| format!(" ({category})"))
            .unwrap_or_default();
        return Err(AnthropicError::Refused(why));
    }
    let written: String = reply
        .content
        .iter()
        .filter(|block| block.kind == "text")
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    if written.trim().is_empty() {
        return Err(AnthropicError::Unreadable("it wrote nothing".to_owned()));
    }
    Ok(written)
}

/// The human-readable half of an API error body, when there is one.
fn message_in(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    Some(parsed.get("error")?.get("message")?.as_str()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(json: &str) -> Reply {
        serde_json::from_str(json).expect("json")
    }

    #[test]
    fn the_text_blocks_are_joined_and_thinking_is_left_out() {
        let written = text_of(&reply(
            r#"{"content":[{"type":"thinking","thinking":"hmm"},
                           {"type":"text","text":"Fix the login redirect"},
                           {"type":"text","text":"\n\nBecause."}],
                "stop_reason":"end_turn"}"#,
        ))
        .expect("text");
        assert_eq!(written, "Fix the login redirect\n\nBecause.");
    }

    #[test]
    fn a_refusal_is_an_error_even_though_the_call_succeeded() {
        let error = text_of(&reply(
            r#"{"content":[],"stop_reason":"refusal",
                "stop_details":{"type":"refusal","category":"cyber"}}"#,
        ))
        .expect_err("refused");
        assert_eq!(error.code(), "anthropic_refused");
        assert!(error.to_string().contains("cyber"));
    }

    #[test]
    fn a_refusal_without_a_category_still_reads_as_a_sentence() {
        let error = text_of(&reply(r#"{"content":[],"stop_reason":"refusal"}"#)).expect_err("no");
        assert_eq!(error.to_string(), "the model declined to write this");
    }

    #[test]
    fn an_empty_answer_is_not_passed_off_as_a_message() {
        let error = text_of(&reply(
            r#"{"content":[{"type":"text","text":"  "}],"stop_reason":"end_turn"}"#,
        ))
        .expect_err("empty");
        assert_eq!(error.code(), "anthropic_unreadable");
    }

    #[test]
    fn an_error_body_is_quoted_back_when_it_has_something_to_say() {
        assert_eq!(
            message_in(
                r#"{"type":"error","error":{"type":"invalid_request_error",
                           "message":"max_tokens: must be greater than 0"}}"#
            ),
            Some("max_tokens: must be greater than 0".to_owned())
        );
        assert_eq!(message_in("not json"), None);
        assert_eq!(message_in(r#"{"error":{}}"#), None);
    }
}

//! A client for TypeSafe's System One API, the one place Yardsort talks to Jev.
//!
//! Jev answers typed questions about a piece of text (the *state*): a yes/no probability (Noul),
//! one of a set of options (Choice), or a position on ordered levels (Score). It never writes
//! text. See <https://docs.typesafe.ai/api>.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// Pinned rather than `jev-latest`: the thresholds in `review` and `suggest` were chosen for this
/// version, and an alias can move under them.
pub const MODEL: &str = "jev-1.13.0";
const BASE_URL: &str = "https://api.typesafe.ai";
const TIMEOUT: Duration = Duration::from_secs(20);
/// 429 (rate limited) and 529 (overloaded) are worth retrying, with backoff.
const ATTEMPTS: u32 = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        instructions: String,
    },
    Choice {
        instructions: String,
        /// Option → what it means.
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        /// Ordered levels, lowest first.
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        /// Probability that the answer is yes.
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        /// Level index (as a string) → probability.
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

impl Answer {
    /// A Noul's probability of yes.
    pub fn yes(&self) -> Option<f64> {
        match self {
            Self::Noul { noul } => Some(*noul),
            _ => None,
        }
    }

    /// The probability a Score put on one level.
    pub fn level(&self, index: usize) -> Option<f64> {
        match self {
            Self::Score { probabilities, .. } => Some(
                probabilities
                    .get(&index.to_string())
                    .copied()
                    .unwrap_or(0.0),
            ),
            _ => None,
        }
    }

    /// A Score's most likely level and its probability.
    pub fn likeliest_level(&self) -> Option<(usize, f64)> {
        match self {
            Self::Score { probabilities, .. } => probabilities
                .iter()
                .filter_map(|(level, p)| Some((level.parse().ok()?, *p)))
                .max_by(|a, b| a.1.total_cmp(&b.1)),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Response {
    model: String,
    answers: BTreeMap<String, Answer>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Answers {
    /// The versioned model that answered.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
}

impl Answers {
    pub fn get(&self, id: &str) -> Option<&Answer> {
        self.answers.get(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JevError {
    #[error("TypeSafe did not accept the API key. Check it in Settings → Assist.")]
    BadKey,
    #[error("TypeSafe is rate limiting this API key. Try again in a minute.")]
    RateLimited,
    #[error("Could not reach TypeSafe: {0}")]
    Unavailable(String),
    /// A request we built was refused: a bug on our side, not the user's.
    #[error("TypeSafe refused the request: {0}")]
    Rejected(String),
}

impl JevError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadKey => "assist_bad_key",
            Self::RateLimited => "assist_rate_limited",
            Self::Unavailable(_) => "assist_unavailable",
            Self::Rejected(_) => "assist_rejected",
        }
    }
}

pub struct Jev {
    client: reqwest::Client,
    base: String,
    key: String,
}

impl Jev {
    pub fn new(key: &str) -> Result<Self, JevError> {
        Self::build(key, BASE_URL, reqwest::Client::builder())
    }

    /// A client for a stand-in server. Proxies are bypassed so a test never depends on them.
    #[cfg(test)]
    pub fn at(base: &str, key: &str) -> Self {
        Self::build(key, base, reqwest::Client::builder().no_proxy()).unwrap()
    }

    fn build(key: &str, base: &str, builder: reqwest::ClientBuilder) -> Result<Self, JevError> {
        // reqwest brings rustls without a crypto provider; the updater installs the same one.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let client = builder
            .timeout(TIMEOUT)
            .user_agent(concat!("Yardsort/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| JevError::Unavailable(error.to_string()))?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        })
    }

    /// Ask every question about one state. They are answered independently and in parallel.
    pub async fn ask(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Answers, JevError> {
        let body = serde_json::json!({
            "model": MODEL,
            "state": state,
            "questions": questions,
        });
        let response: Response = self
            .send(|| {
                self.client
                    .post(format!("{}/v1/systemone", self.base))
                    .json(&body)
            })
            .await?
            .json()
            .await
            .map_err(|error| JevError::Unavailable(format!("unexpected answer: {error}")))?;
        Ok(Answers {
            model: response.model,
            answers: response.answers,
        })
    }

    /// Whether the key works: the cheapest authenticated call there is.
    pub async fn check_key(&self) -> Result<(), JevError> {
        self.send(|| self.client.get(format!("{}/v1/models", self.base)))
            .await
            .map(drop)
    }

    async fn send(
        &self,
        request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, JevError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let response = request()
                .bearer_auth(&self.key)
                .send()
                .await
                .map_err(|error| JevError::Unavailable(without_url(&error)))?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529;
            if retryable && attempt < ATTEMPTS {
                tokio::time::sleep(retry_wait(&response, attempt)).await;
                continue;
            }
            return Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => JevError::BadKey,
                StatusCode::TOO_MANY_REQUESTS => JevError::RateLimited,
                StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                    JevError::Rejected(response.text().await.unwrap_or_default())
                }
                _ => JevError::Unavailable(format!("the server answered {status}")),
            });
        }
    }
}

/// `retry-after` when the server gives one, else exponential backoff from half a second.
fn retry_wait(response: &reqwest::Response, attempt: u32) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok()?.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map_or_else(
            || Duration::from_millis(500 * 2u64.pow(attempt - 1)),
            Duration::from_secs_f64,
        )
        .min(MAX_RETRY_WAIT)
}

/// reqwest errors name the URL; ours carries nothing secret, but the message reads better without.
fn without_url(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    if let Some(url) = error.url() {
        message = message.replace(&format!(" ({url})"), "");
        message = message.replace(url.as_str(), "TypeSafe");
    }
    message
}

#[cfg(test)]
pub mod fake {
    //! A stand-in for the TypeSafe API: a real HTTP server on a loopback port that answers from
    //! a script and records what it was sent.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub struct Received {
        pub method: String,
        pub path: String,
        pub authorization: Option<String>,
        pub body: serde_json::Value,
    }

    pub struct FakeServer {
        pub url: String,
        pub received: Arc<Mutex<Vec<Received>>>,
    }

    /// Answer each request with `respond(request)`: a status and a JSON body.
    pub fn serve(
        respond: impl Fn(&Received) -> (u16, serde_json::Value) + Send + 'static,
    ) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let (mut length, mut authorization) = (0, None);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap(),
                        "authorization" => authorization = Some(value.trim().to_owned()),
                        _ => {}
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let request = Received {
                    method,
                    path,
                    authorization,
                    body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
                };
                let (status, reply) = respond(&request);
                log.lock().unwrap().push(request);
                let reply = reply.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nRetry-After: 0\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        FakeServer { url, received }
    }

    /// Answer every question the way `answer(question id, question)` says.
    pub fn answering(
        answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> FakeServer {
        serve(move |request| {
            let answers: serde_json::Map<_, _> = request.body["questions"]
                .as_object()
                .map(|questions| {
                    questions
                        .iter()
                        .map(|(id, question)| (id.clone(), answer(id, question)))
                        .collect()
                })
                .unwrap_or_default();
            (
                200,
                serde_json::json!({
                    "model": super::MODEL,
                    "answers": answers,
                    "usage": {"input_tokens": 100, "output_tokens": 10},
                }),
            )
        })
    }

    pub fn noul(p: f64) -> serde_json::Value {
        serde_json::json!({"type": "noul", "noul": p})
    }

    /// A Score answer with these probabilities per level.
    pub fn score(levels: &[f64]) -> serde_json::Value {
        let probabilities: serde_json::Map<_, _> = levels
            .iter()
            .enumerate()
            .map(|(i, p)| (i.to_string(), serde_json::json!(p)))
            .collect();
        let score: f64 = levels.iter().enumerate().map(|(i, p)| i as f64 * p).sum();
        serde_json::json!({"type": "score", "score": score, "legend": {},
                           "probabilities": probabilities, "confidence": 0.8})
    }

    pub fn choice(pick: &str, confidence: f64) -> serde_json::Value {
        serde_json::json!({"type": "choice", "choice": pick,
                           "probabilities": {pick: confidence}, "confidence": confidence})
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn questions_and_state_go_out_in_the_documented_shape_and_answers_come_back_typed() {
        let server = answering(|id, _| match id {
            "urgent" => noul(0.92),
            "team" => choice("technical", 0.85),
            _ => score(&[0.05, 0.3, 0.65]),
        });
        let jev = Jev::at(&server.url, "ts-secret");
        let questions = BTreeMap::from([
            (
                "urgent".to_owned(),
                Question::Noul {
                    instructions: "Does this convey urgency?".into(),
                },
            ),
            (
                "team".to_owned(),
                Question::Choice {
                    instructions: "Which team?".into(),
                    criteria: BTreeMap::from([
                        ("billing".to_owned(), "Payments".to_owned()),
                        ("technical".to_owned(), "Bugs".to_owned()),
                    ]),
                },
            ),
            (
                "anger".to_owned(),
                Question::Score {
                    instructions: "How angry?".into(),
                    criteria: vec!["Calm".into(), "Cross".into(), "Furious".into()],
                },
            ),
        ]);
        let answers = block(jev.ask(&serde_json::json!({"message": "Help!"}), &questions)).unwrap();

        assert_eq!(answers.model, MODEL);
        assert_eq!(answers.get("urgent").unwrap().yes(), Some(0.92));
        assert!(
            matches!(answers.get("team"), Some(Answer::Choice { choice, .. }) if choice == "technical")
        );
        assert_eq!(
            answers.get("anger").unwrap().likeliest_level(),
            Some((2, 0.65))
        );
        assert_eq!(answers.get("anger").unwrap().level(0), Some(0.05));

        let sent = server.received.lock().unwrap()[0].clone();
        assert_eq!(
            (sent.method.as_str(), sent.path.as_str()),
            ("POST", "/v1/systemone")
        );
        assert_eq!(sent.authorization.as_deref(), Some("Bearer ts-secret"));
        assert_eq!(sent.body["model"], MODEL);
        assert_eq!(sent.body["state"]["message"], "Help!");
        assert_eq!(sent.body["questions"]["urgent"]["type"], "noul");
        assert_eq!(
            sent.body["questions"]["team"]["criteria"]["billing"],
            "Payments"
        );
        assert_eq!(sent.body["questions"]["anger"]["criteria"][2], "Furious");
    }

    #[test]
    fn a_rejected_key_is_reported_as_such_and_not_retried() {
        let server = serve(|_| (401, serde_json::json!({"detail": "bad key"})));
        let jev = Jev::at(&server.url, "wrong");
        assert_eq!(block(jev.check_key()), Err(JevError::BadKey));
        let received = server.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].path, "/v1/models");
    }

    #[test]
    fn rate_limits_and_overload_are_retried_then_given_up_on() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let server = serve(move |_| {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match n {
                0 => (529, serde_json::json!({})),
                1 => (429, serde_json::json!({})),
                _ => (200, serde_json::json!({"models": []})),
            }
        });
        assert_eq!(block(Jev::at(&server.url, "k").check_key()), Ok(()));
        assert_eq!(server.received.lock().unwrap().len(), 3);

        let server = serve(|_| (429, serde_json::json!({})));
        assert_eq!(
            block(Jev::at(&server.url, "k").check_key()),
            Err(JevError::RateLimited)
        );
        assert_eq!(server.received.lock().unwrap().len(), ATTEMPTS as usize);
    }

    #[test]
    fn no_server_means_unavailable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let jev = Jev::at(&format!("http://127.0.0.1:{port}"), "k");
        assert!(matches!(
            block(jev.check_key()),
            Err(JevError::Unavailable(_))
        ));
    }
}

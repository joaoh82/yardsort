//! The agents' own session logs, read for the tokens each model call used.
//!
//! These are files the agents write for themselves, read where they are and never changed:
//!
//! - **Claude Code**: `projects/<folder>/<session>.jsonl` under `CLAUDE_CONFIG_DIR`, else
//!   `~/.claude` (and `~/.config/claude`, where some installs keep it). Every assistant message
//!   carries the API's `usage`; a message is written once per content block, and a resumed
//!   conversation copies earlier messages into its new file, so a message is counted once by
//!   its id and request id.
//! - **Codex**: `sessions/YYYY/MM/DD/rollout-*.jsonl` under `CODEX_HOME`, else `~/.codex`. A
//!   `token_usage_record` per model response; older versions only wrote running totals in
//!   `token_count` events, which are counted by their difference. `token_count` also carries the
//!   plan's rate limits as Codex last heard them.
//! - **Grok**: `sessions/<encoded folder>/<session>/usage.json` under `GROK_HOME`, else
//!   `~/.grok`, with tokens per turn and per model.
//!
//! None of this is the output of a terminal: it is structured data the agents keep on disk, the
//! same files the activity adapters already read (see `crate::activity`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use serde_json::Value;

use crate::activity::iso_to_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agent {
    Claude,
    Codex,
    Grok,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Grok];

    /// The built-in harness id, which is also what the frontend shows a name and icon for.
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Grok => "grok",
        }
    }
}

/// Tokens of one kind or another. `input` is the part that was neither read from nor written to
/// a cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.cache_read + self.cache_write_5m + self.cache_write_1h + self.output
    }

    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.output += other.output;
    }
}

/// One model call — or, for Grok, one turn of one model — as its agent logged it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub at_ms: i64,
    pub model: Arc<str>,
    /// The folder the agent ran in, which is how a call is matched to a workspace.
    pub cwd: Option<Arc<str>>,
    pub tokens: Tokens,
    /// The same call can be logged in more than one file; this says which calls are one.
    pub key: Option<u64>,
}

/// A plan's rate limits as the agent last heard them from its vendor.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub observed_at_ms: i64,
    pub plan: Option<String>,
    pub windows: Vec<LimitWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitWindow {
    pub minutes: Option<u32>,
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
}

/// What one file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    pub limits: Option<Limits>,
}

fn key_of(parts: &[&str]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

fn n(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn s<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Strings repeat on almost every line of a file; keep one copy of each.
#[derive(Default)]
struct Interner(HashMap<String, Arc<str>>);

impl Interner {
    fn get(&mut self, text: &str) -> Arc<str> {
        if let Some(found) = self.0.get(text) {
            return Arc::clone(found);
        }
        let shared: Arc<str> = Arc::from(text);
        self.0.insert(text.to_owned(), Arc::clone(&shared));
        shared
    }
}

/// A Claude Code transcript.
pub fn parse_claude(text: &str) -> Parsed {
    let mut strings = Interner::default();
    let mut entries: Vec<Entry> = Vec::new();
    // A message is written once per content block, each line with the usage so far; keep the
    // line that saw the most output.
    let mut seen: HashMap<u64, usize> = HashMap::new();
    for line in text.lines() {
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s(&value, "type") != Some("assistant") {
            continue;
        }
        let message = &value["message"];
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let Some(model) = s(message, "model").filter(|m| !m.is_empty() && *m != "<synthetic>")
        else {
            continue;
        };
        let Some(at_ms) = s(&value, "timestamp").and_then(iso_to_ms) else {
            continue;
        };
        let written = n(usage, "cache_creation_input_tokens");
        let (write_5m, write_1h) = match usage.get("cache_creation") {
            Some(split) if split.is_object() => (
                n(split, "ephemeral_5m_input_tokens"),
                n(split, "ephemeral_1h_input_tokens"),
            ),
            _ => (written, 0),
        };
        let tokens = Tokens {
            input: n(usage, "input_tokens"),
            cache_read: n(usage, "cache_read_input_tokens"),
            cache_write_5m: write_5m,
            cache_write_1h: write_1h,
            output: n(usage, "output_tokens"),
        };
        let key = match (s(message, "id"), s(&value, "requestId")) {
            (None, None) => None,
            (id, request) => Some(key_of(&[
                "claude",
                id.unwrap_or_default(),
                request.unwrap_or_default(),
            ])),
        };
        let entry = Entry {
            at_ms,
            model: strings.get(model),
            cwd: s(&value, "cwd").map(|cwd| strings.get(cwd)),
            tokens,
            key,
        };
        match key.and_then(|key| seen.get(&key).copied()) {
            Some(index) => {
                if entry.tokens.output > entries[index].tokens.output {
                    entries[index] = entry;
                }
            }
            None => {
                if let Some(key) = key {
                    seen.insert(key, entries.len());
                }
                entries.push(entry);
            }
        }
    }
    Parsed {
        entries,
        limits: None,
    }
}

/// A Codex rollout file.
pub fn parse_codex(text: &str) -> Parsed {
    let mut strings = Interner::default();
    let mut records: Vec<Entry> = Vec::new();
    let mut from_totals: Vec<Entry> = Vec::new();
    let mut limits: Option<Limits> = None;
    let mut model: Arc<str> = strings.get("unknown");
    let mut cwd: Option<Arc<str>> = None;
    let mut session = String::new();
    let mut last_total: Option<Tokens> = None;

    // Codex counts cached input inside `input_tokens`; ours is the part outside the cache.
    let tokens = |usage: &Value| {
        let input = n(usage, "input_tokens");
        let cached = n(usage, "cached_input_tokens").min(input);
        Tokens {
            input: input - cached,
            cache_read: cached,
            output: n(usage, "output_tokens"),
            ..Tokens::default()
        }
    };

    for line in text.lines() {
        if !(line.contains("token_usage_record")
            || line.contains("token_count")
            || line.contains("turn_context")
            || line.contains("session_meta"))
        {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = &value["payload"];
        let at_ms = s(&value, "timestamp").and_then(iso_to_ms);
        match s(&value, "type") {
            Some("session_meta") => {
                if let Some(dir) = s(payload, "cwd") {
                    cwd = Some(strings.get(dir));
                }
                if let Some(id) = s(payload, "id").or_else(|| s(payload, "session_id")) {
                    session = id.to_owned();
                }
            }
            Some("turn_context") => {
                if let Some(name) = s(payload, "model").filter(|m| !m.is_empty()) {
                    model = strings.get(name);
                }
                if let Some(dir) = s(payload, "cwd") {
                    cwd = Some(strings.get(dir));
                }
            }
            Some("token_usage_record") => {
                let (Some(at_ms), Some(usage)) = (at_ms, payload.get("usage")) else {
                    continue;
                };
                records.push(Entry {
                    at_ms,
                    model: Arc::clone(&model),
                    cwd: cwd.clone(),
                    tokens: tokens(usage),
                    key: s(payload, "response_id").map(|id| key_of(&["codex", id])),
                });
            }
            Some("event_msg") if s(payload, "type") == Some("token_count") => {
                let Some(at_ms) = at_ms else { continue };
                if let Some(total) = payload["info"].get("total_token_usage") {
                    let total = tokens(total);
                    let before = last_total.unwrap_or_default();
                    // The same totals are repeated whenever only the rate limits moved.
                    if total != before && total.total() >= before.total() {
                        from_totals.push(Entry {
                            at_ms,
                            model: Arc::clone(&model),
                            cwd: cwd.clone(),
                            tokens: Tokens {
                                input: total.input.saturating_sub(before.input),
                                cache_read: total.cache_read.saturating_sub(before.cache_read),
                                output: total.output.saturating_sub(before.output),
                                ..Tokens::default()
                            },
                            key: Some(key_of(&[
                                "codex-total",
                                &session,
                                &total.total().to_string(),
                            ])),
                        });
                        last_total = Some(total);
                    }
                }
                if let Some(seen) = codex_limits(&payload["rate_limits"], at_ms) {
                    if limits
                        .as_ref()
                        .is_none_or(|known| known.observed_at_ms <= seen.observed_at_ms)
                    {
                        limits = Some(seen);
                    }
                }
            }
            _ => {}
        }
    }
    Parsed {
        // Records are the exact figures; totals only stand in for versions that lack them.
        entries: if records.is_empty() {
            from_totals
        } else {
            records
        },
        limits,
    }
}

fn codex_limits(rate_limits: &Value, observed_at_ms: i64) -> Option<Limits> {
    if !rate_limits.is_object() {
        return None;
    }
    let windows: Vec<LimitWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|which| {
            let window = rate_limits.get(*which)?;
            Some(LimitWindow {
                minutes: window
                    .get("window_minutes")
                    .and_then(Value::as_u64)
                    .and_then(|m| u32::try_from(m).ok()),
                used_percent: window.get("used_percent")?.as_f64()?,
                resets_at_ms: window
                    .get("resets_at")
                    .and_then(Value::as_i64)
                    .map(|seconds| seconds * 1000),
            })
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    Some(Limits {
        observed_at_ms,
        plan: s(rate_limits, "plan_type").map(str::to_owned),
        windows,
    })
}

/// A Grok session's `usage.json`. Grok names the folder only in the directory's name.
pub fn parse_grok(text: &str, session_id: &str, cwd: Option<&str>) -> Parsed {
    let mut strings = Interner::default();
    let Ok(usage) = serde_json::from_str::<Value>(text) else {
        return Parsed::default();
    };
    let cwd = cwd.map(|dir| strings.get(dir));
    let mut entries = Vec::new();
    for turn in usage["turns"].as_array().into_iter().flatten() {
        let Some(at_ms) = s(turn, "endedAt").and_then(iso_to_ms) else {
            continue;
        };
        let number = turn
            .get("turnNumber")
            .map(Value::to_string)
            .unwrap_or_default();
        // Per model where Grok split it, else the whole turn under its main model.
        let parts: Vec<(&str, &Value)> = match turn.get("modelUsage").and_then(Value::as_object) {
            Some(models) if !models.is_empty() => {
                models.iter().map(|(m, u)| (m.as_str(), u)).collect()
            }
            _ => vec![(s(turn, "primaryModelId").unwrap_or("unknown"), turn)],
        };
        for (model, counts) in parts {
            // Like Codex, Grok counts cached input inside `inputTokens`.
            let input = n(counts, "inputTokens");
            let cached = n(counts, "cachedReadTokens").min(input);
            entries.push(Entry {
                at_ms,
                model: strings.get(model),
                cwd: cwd.clone(),
                tokens: Tokens {
                    input: input - cached,
                    cache_read: cached,
                    output: n(counts, "outputTokens"),
                    ..Tokens::default()
                },
                key: Some(key_of(&["grok", session_id, &number, model])),
            });
        }
    }
    Parsed {
        entries,
        limits: None,
    }
}

/// Grok's directory for a folder is the folder's path, percent-encoded.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where each agent keeps its logs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub claude: Vec<PathBuf>,
    pub codex: Option<PathBuf>,
    pub grok: Option<PathBuf>,
}

impl Sources {
    /// From the environment the agents are launched in, so these are the folders they use.
    pub fn from_env(var: &dyn Fn(&str) -> Option<String>) -> Self {
        let home = var("HOME")
            .or_else(|| var("USERPROFILE"))
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let claude = match var("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => vec![PathBuf::from(dir)],
            None => home
                .iter()
                .flat_map(|home| [home.join(".claude"), home.join(".config").join("claude")])
                .collect(),
        };
        Self {
            claude,
            codex: crate::activity::codex::codex_home(var),
            grok: crate::activity::grok::grok_home(var),
        }
    }

    /// The folders to look in for one agent, and which files there are its logs.
    fn roots(&self, agent: Agent) -> Vec<PathBuf> {
        match agent {
            Agent::Claude => self.claude.iter().map(|dir| dir.join("projects")).collect(),
            Agent::Codex => self
                .codex
                .iter()
                .flat_map(|dir| [dir.join("sessions"), dir.join("archived_sessions")])
                .collect(),
            Agent::Grok => self.grok.iter().map(|dir| dir.join("sessions")).collect(),
        }
    }

    /// Where an agent's logs are, for the person reading the page: the first that exists, or
    /// the first it would be.
    pub fn location(&self, agent: Agent) -> Option<PathBuf> {
        let dirs: Vec<PathBuf> = match agent {
            Agent::Claude => self.claude.clone(),
            Agent::Codex => self.codex.iter().cloned().collect(),
            Agent::Grok => self.grok.iter().cloned().collect(),
        };
        dirs.iter()
            .find(|dir| dir.is_dir())
            .or(dirs.first())
            .cloned()
    }
}

fn is_log(agent: Agent, path: &Path) -> bool {
    match agent {
        Agent::Claude | Agent::Codex => path.extension().is_some_and(|ext| ext == "jsonl"),
        Agent::Grok => path.file_name().is_some_and(|name| name == "usage.json"),
    }
}

/// Every log file under `root`, without following links out of it. Deep enough for Codex's
/// date folders and Claude Code's sub-agent folders; no deeper.
fn walk(agent: Agent, root: &Path, out: &mut Vec<PathBuf>) {
    fn go(agent: Agent, dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth < 5 {
                go(agent, &path, depth + 1, out);
            } else if kind.is_file() && is_log(agent, &path) {
                out.push(path);
            }
        }
    }
    go(agent, root, 0, out);
}

fn parse_file(agent: Agent, path: &Path, text: &str) -> Parsed {
    match agent {
        Agent::Claude => parse_claude(text),
        Agent::Codex => parse_codex(text),
        Agent::Grok => {
            let session_dir = path.parent();
            let session = session_dir
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let cwd = session_dir
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .map(|name| percent_decode(&name.to_string_lossy()));
            parse_grok(text, &session, cwd.as_deref())
        }
    }
}

struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    parsed: Arc<Parsed>,
}

/// One file's contents, as far as usage goes.
pub struct File {
    pub agent: Agent,
    pub parsed: Arc<Parsed>,
}

/// What was read, with how many files each agent had.
pub struct Read {
    pub files: Vec<File>,
    pub counts: HashMap<Agent, u32>,
}

/// Parsed files, kept between reads. A file is parsed again only when its size or time
/// changed, so after the first read only the conversations still going cost anything.
#[derive(Default)]
pub struct LogCache {
    files: Mutex<HashMap<PathBuf, Cached>>,
}

impl LogCache {
    /// Every log file last written at or after `since_ms`. Older ones cannot hold a call in the
    /// range and are not opened.
    pub fn read(&self, sources: &Sources, since_ms: i64) -> Read {
        let since = SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis(u64::try_from(since_ms).unwrap_or(0));
        let mut found: Vec<(Agent, PathBuf)> = Vec::new();
        let mut counts = HashMap::new();
        for agent in Agent::ALL {
            let mut paths = Vec::new();
            for root in sources.roots(agent) {
                walk(agent, &root, &mut paths);
            }
            counts.insert(agent, u32::try_from(paths.len()).unwrap_or(u32::MAX));
            found.extend(paths.into_iter().map(|path| (agent, path)));
        }

        let mut cache = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|path, _| found.iter().any(|(_, p)| p == path));
        let mut files = Vec::new();
        for (agent, path) in found {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta.modified().ok();
            if modified.is_some_and(|time| time < since) {
                continue;
            }
            let fresh = cache
                .get(&path)
                .is_some_and(|known| known.len == meta.len() && known.modified == modified);
            if !fresh {
                // A file that cannot be read now is skipped, and tried again next time.
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes);
                let parsed = Arc::new(parse_file(agent, &path, &text));
                cache.insert(
                    path.clone(),
                    Cached {
                        len: meta.len(),
                        modified,
                        parsed,
                    },
                );
            }
            if let Some(known) = cache.get(&path) {
                files.push(File {
                    agent,
                    parsed: Arc::clone(&known.parsed),
                });
            }
        }
        Read { files, counts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join(path),
        )
        .unwrap()
    }

    #[test]
    fn claude_counts_a_message_once_however_many_lines_carry_it() {
        let parsed = parse_claude(&fixture("claude-transcript/2.1.285/session.jsonl"));
        // Three API calls; the second is written twice, once per content block.
        assert_eq!(parsed.entries.len(), 3, "{:#?}", parsed.entries);
        let first = &parsed.entries[0];
        assert_eq!(&*first.model, "claude-opus-5-5");
        assert_eq!(first.cwd.as_deref(), Some("/home/demo/code/app"));
        assert_eq!(
            first.tokens,
            Tokens {
                input: 2,
                cache_read: 25_148,
                cache_write_5m: 0,
                cache_write_1h: 22_463,
                output: 139,
            }
        );
        // The later line of the same message saw more output; that one is kept.
        assert_eq!(parsed.entries[1].tokens.output, 412);
        assert_eq!(&*parsed.entries[2].model, "claude-haiku-4-5-20251001");
        // Without the per-TTL split, a cache write is the five-minute kind.
        assert_eq!(parsed.entries[2].tokens.cache_write_5m, 1_000);
    }

    #[test]
    fn codex_counts_each_response_once_with_its_model_and_folder() {
        let parsed = parse_codex(&fixture("codex/0.156.1/rollout.jsonl"));
        assert_eq!(parsed.entries.len(), 2);
        let first = &parsed.entries[0];
        assert_eq!(
            first.tokens,
            Tokens {
                input: 14_717 - 12_160,
                cache_read: 12_160,
                output: 133,
                ..Tokens::default()
            }
        );
        assert!(first.cwd.is_some());
        assert!(first.key.is_some());
        let limits = parsed.limits.expect("token_count carries the rate limits");
        assert!(!limits.windows.is_empty());
    }

    #[test]
    fn codex_without_usage_records_is_counted_from_its_running_totals() {
        let line = |at: &str, input: u64, cached: u64, output: u64| {
            format!(
                r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output}}}}},"rate_limits":{{"plan_type":"pro","primary":{{"used_percent":14.0,"window_minutes":10080,"resets_at":1791310550}}}}}}}}"#
            )
        };
        let text = [
            r#"{"timestamp":"2026-10-01T10:00:00Z","type":"turn_context","payload":{"model":"gpt-6-sol","cwd":"/w/a"}}"#.to_owned(),
            line("2026-10-01T10:00:01Z", 1000, 400, 50),
            // Repeated because only the limits moved: not more usage.
            line("2026-10-01T10:00:02Z", 1000, 400, 50),
            line("2026-10-01T10:00:03Z", 3000, 2000, 80),
        ]
        .join("\n");
        let parsed = parse_codex(&text);
        let tokens: Vec<Tokens> = parsed.entries.iter().map(|e| e.tokens).collect();
        assert_eq!(
            tokens,
            vec![
                Tokens {
                    input: 600,
                    cache_read: 400,
                    output: 50,
                    ..Tokens::default()
                },
                Tokens {
                    input: 400,
                    cache_read: 1600,
                    output: 30,
                    ..Tokens::default()
                },
            ]
        );
        assert!(parsed.entries.iter().all(|e| &*e.model == "gpt-6-sol"));
        let limits = parsed.limits.unwrap();
        assert_eq!(limits.plan.as_deref(), Some("pro"));
        assert_eq!(limits.windows[0].minutes, Some(10080));
        assert_eq!(limits.windows[0].resets_at_ms, Some(1_791_310_550_000));
    }

    #[test]
    fn grok_counts_each_turn_per_model() {
        let parsed = parse_grok(
            &fixture("grok/1.0.41/usage.json"),
            "11111111-1111-4111-8111-111111111111",
            Some("/home/demo/code/app"),
        );
        assert_eq!(parsed.entries.len(), 1);
        let turn = &parsed.entries[0];
        assert_eq!(&*turn.model, "grok-4.7-build");
        assert_eq!(turn.tokens.input, 56_643 - 43_648);
        assert_eq!(turn.tokens.cache_read, 43_648);
        assert_eq!(turn.tokens.output, 464);
    }

    #[test]
    fn grok_folder_names_decode_to_the_folder() {
        assert_eq!(
            percent_decode("%2Fhome%2Fdemo%2Fmy%20app"),
            "/home/demo/my app"
        );
        assert_eq!(percent_decode("C%3A%5Ccode"), "C:\\code");
        assert_eq!(percent_decode("plain%2"), "plain%2");
    }

    #[test]
    fn locations_follow_the_agents_own_variables() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let sources = Sources::from_env(&env(&[("HOME", "/h")]));
        assert_eq!(
            sources.claude,
            vec![
                PathBuf::from("/h/.claude"),
                PathBuf::from("/h/.config/claude")
            ]
        );
        assert_eq!(sources.codex, Some(PathBuf::from("/h/.codex")));
        assert_eq!(sources.grok, Some(PathBuf::from("/h/.grok")));

        let sources = Sources::from_env(&env(&[
            ("HOME", "/h"),
            ("CLAUDE_CONFIG_DIR", "/c"),
            ("CODEX_HOME", "/x"),
            ("GROK_HOME", "/g"),
        ]));
        assert_eq!(sources.claude, vec![PathBuf::from("/c")]);
        assert_eq!(sources.codex, Some(PathBuf::from("/x")));
        assert_eq!(sources.grok, Some(PathBuf::from("/g")));
    }

    /// The real thing: logs on disk where each agent keeps them, read, cached, and read again
    /// only when they change.
    #[test]
    fn the_cache_reads_a_file_again_only_once_it_changed() {
        let home = tempfile::tempdir().unwrap();
        let claude = home.path().join(".claude/projects/-code-app");
        std::fs::create_dir_all(&claude).unwrap();
        let transcript = claude.join("s.jsonl");
        let text = fixture("claude-transcript/2.1.285/session.jsonl");
        let mut lines: Vec<&str> = text.lines().collect();
        let last = lines.pop().unwrap();
        std::fs::write(&transcript, lines.join("\n")).unwrap();

        let codex = home.path().join(".codex/sessions/2026/09/25");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("rollout-x.jsonl"),
            fixture("codex/0.156.1/rollout.jsonl"),
        )
        .unwrap();
        let grok = home
            .path()
            .join(".grok/sessions/%2Fcode%2Fapp/11111111-1111-4111-8111-111111111111");
        std::fs::create_dir_all(&grok).unwrap();
        std::fs::write(grok.join("usage.json"), fixture("grok/1.0.41/usage.json")).unwrap();

        let dir = home.path().to_string_lossy().into_owned();
        let sources = Sources::from_env(&|key| (key == "HOME").then(|| dir.clone()));
        let cache = LogCache::default();
        let count = |read: &Read, agent| {
            read.files
                .iter()
                .filter(|f| f.agent == agent)
                .map(|f| f.parsed.entries.len())
                .sum::<usize>()
        };

        let first = cache.read(&sources, 0);
        assert_eq!(count(&first, Agent::Claude), 2);
        assert_eq!(count(&first, Agent::Codex), 2);
        assert_eq!(count(&first, Agent::Grok), 1);
        let grok_entry = &first
            .files
            .iter()
            .find(|f| f.agent == Agent::Grok)
            .unwrap()
            .parsed
            .entries[0];
        assert_eq!(grok_entry.cwd.as_deref(), Some("/code/app"));

        let again = cache.read(&sources, 0);
        let claude_file = |read: &Read| {
            Arc::clone(
                &read
                    .files
                    .iter()
                    .find(|f| f.agent == Agent::Claude)
                    .unwrap()
                    .parsed,
            )
        };
        assert!(
            Arc::ptr_eq(&claude_file(&first), &claude_file(&again)),
            "unchanged: not parsed again"
        );

        std::fs::write(&transcript, format!("{}\n{last}", lines.join("\n"))).unwrap();
        let grown = cache.read(&sources, 0);
        assert_eq!(count(&grown, Agent::Claude), 3);

        // Nothing written since the cut-off: not opened at all.
        let later = cache.read(&sources, crate::store::now_ms() + 60_000);
        assert!(later.files.is_empty());
        assert_eq!(later.counts[&Agent::Claude], 1);
    }
}

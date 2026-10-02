//! Token usage: what the agents spent, read from their own session logs (see [`logs`]) and
//! priced at their vendors' API rates (see [`prices`]).
//!
//! Nothing here asks a vendor anything: the figures are what the agents wrote down on this
//! machine. That makes them complete for this machine and blind to every other, and the cost an
//! estimate of what the same tokens would have cost billed per token — which a subscription is
//! not.

pub mod logs;
pub mod prices;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use specta::Type;

pub use logs::{Agent, LogCache, Sources};
use logs::{File, Tokens};

/// A folder an agent can run in that Yardsort knows by name.
#[derive(Debug, Clone)]
pub struct Place {
    pub path: String,
    pub project: String,
    pub workspace: String,
    pub workspace_id: String,
}

/// What the page asks for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The last this many days, today included.
    pub days: u32,
    pub now_ms: i64,
    /// The viewer's offset from UTC, so a day is their day.
    pub utc_offset_minutes: i32,
}

impl Request {
    /// The first millisecond of the first day in range.
    pub fn since_ms(&self) -> i64 {
        let offset = i64::from(self.utc_offset_minutes) * 60_000;
        let today = (self.now_ms + offset).div_euclid(DAY_MS);
        (today - i64::from(self.days.max(1)) + 1) * DAY_MS - offset
    }
}

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub days: u32,
    /// `YYYY-MM-DD`, the first and the last day in range.
    pub from: String,
    pub to: String,
    /// What the priced tokens would cost at API rates, in US dollars.
    pub cost: f64,
    /// Models seen whose price is not known: their tokens are counted, their cost is not.
    pub unpriced_models: Vec<String>,
    pub totals: TokenTotals,
    /// Every agent with logs or usage, in a fixed order.
    pub agents: Vec<AgentUsage>,
    /// One per day in range, oldest first; `cost` and `tokens` line up with `agents`.
    pub daily: Vec<DayUsage>,
    /// Most expensive first, then most tokens.
    pub models: Vec<ModelUsage>,
    pub places: Vec<PlaceUsage>,
    pub limits: Vec<AgentLimits>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    /// Everything: input of every kind, and output.
    pub processed: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// Input that was neither read from nor written to a cache.
    pub uncached_input: f64,
    pub output: f64,
    /// What reading from the cache saved against paying full input price for the same tokens.
    pub cache_savings: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    /// The harness id: `claude`, `codex` or `grok`.
    pub agent: String,
    pub tokens: f64,
    pub cost: f64,
    /// Where its logs are, with the home folder as `~`.
    pub location: Option<String>,
    /// Log files found there, of any age.
    pub files: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DayUsage {
    pub date: String,
    pub cost: Vec<f64>,
    pub tokens: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub agent: String,
    pub tokens: f64,
    /// `None` when the model's price is not known.
    pub known_cost: Option<f64>,
}

/// Where the tokens were spent: a Yardsort workspace, or a folder Yardsort does not know.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaceUsage {
    pub label: String,
    pub project: Option<String>,
    pub workspace_id: Option<String>,
    /// The folder, for one Yardsort does not know.
    pub folder: Option<String>,
    pub tokens: f64,
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentLimits {
    pub agent: String,
    pub plan: Option<String>,
    /// When the agent last heard these from its vendor, in epoch milliseconds.
    pub observed_at: f64,
    pub windows: Vec<LimitWindowDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindowDto {
    pub minutes: Option<u32>,
    pub used_percent: f64,
    pub resets_at: Option<f64>,
}

/// What a call's tokens cost, or `None` for a model with no known price.
fn cost_of(model: &str, tokens: &Tokens) -> Option<f64> {
    let price = prices::price(model)?;
    let m = |count: u64, rate: f64| count as f64 * rate / 1_000_000.0;
    Some(
        m(tokens.input, price.input)
            + m(tokens.cache_read, price.cache_read)
            + m(tokens.cache_write_5m, price.cache_write_5m)
            + m(tokens.cache_write_1h, price.cache_write_1h)
            + m(tokens.output, price.output),
    )
}

/// `YYYY-MM-DD` of a day counted from the epoch. Howard Hinnant's civil-from-days.
fn date_of(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The Yardsort workspace a folder is in: the deepest one containing it. A worktree inside the
/// project's own folder is its own workspace, not the project's `local`.
fn place_of<'a>(cwd: &str, places: &'a [Place]) -> Option<&'a Place> {
    let cwd = Path::new(cwd);
    places
        .iter()
        .filter(|place| !place.path.is_empty() && cwd.starts_with(&place.path))
        .max_by_key(|place| Path::new(&place.path).components().count())
}

/// A path for reading, with the home folder as `~`.
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => Path::new("~").join(rest).to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// Add up the calls in range. A call logged in more than one file counts once.
pub fn report(
    files: &[File],
    file_counts: &HashMap<Agent, u32>,
    sources: &Sources,
    home: Option<&Path>,
    places: &[Place],
    request: &Request,
) -> UsageReport {
    let offset = i64::from(request.utc_offset_minutes) * 60_000;
    let since = request.since_ms();
    let first_day = (since + offset).div_euclid(DAY_MS);
    let days = request.days.max(1) as usize;

    let mut seen: HashSet<u64> = HashSet::new();
    let mut totals = TokenTotals::default();
    let mut cost = 0.0;
    let mut unpriced: BTreeMap<String, ()> = BTreeMap::new();
    let mut by_agent: HashMap<Agent, (f64, f64)> = HashMap::new();
    let mut daily: Vec<HashMap<Agent, (f64, f64)>> = vec![HashMap::new(); days];
    let mut by_model: HashMap<(Agent, Arc<str>), (f64, Option<f64>)> = HashMap::new();
    let mut by_place: HashMap<String, PlaceUsage> = HashMap::new();
    let mut limits: HashMap<Agent, &logs::Limits> = HashMap::new();

    for file in files {
        if let Some(found) = &file.parsed.limits {
            let newer = limits
                .get(&file.agent)
                .is_none_or(|known| known.observed_at_ms < found.observed_at_ms);
            if newer {
                limits.insert(file.agent, found);
            }
        }
        for entry in &file.parsed.entries {
            if entry.at_ms < since || entry.at_ms > request.now_ms + DAY_MS {
                continue;
            }
            if let Some(key) = entry.key {
                if !seen.insert(key) {
                    continue;
                }
            }
            let day = (entry.at_ms + offset).div_euclid(DAY_MS) - first_day;
            let Some(day) = usize::try_from(day).ok().filter(|d| *d < days) else {
                continue;
            };
            let t = &entry.tokens;
            let tokens = t.total() as f64;
            let price = prices::price(&entry.model);
            let spent = cost_of(&entry.model, t);
            if spent.is_none() {
                unpriced.insert(entry.model.to_string(), ());
            }
            let spent_or_zero = spent.unwrap_or(0.0);

            totals.processed += tokens;
            totals.cache_read += t.cache_read as f64;
            totals.cache_write += (t.cache_write_5m + t.cache_write_1h) as f64;
            totals.uncached_input += t.input as f64;
            totals.output += t.output as f64;
            if let Some(price) = price {
                totals.cache_savings +=
                    t.cache_read as f64 * (price.input - price.cache_read) / 1_000_000.0;
            }
            cost += spent_or_zero;

            let agent = by_agent.entry(file.agent).or_default();
            agent.0 += tokens;
            agent.1 += spent_or_zero;
            let on_day = daily[day].entry(file.agent).or_default();
            on_day.0 += tokens;
            on_day.1 += spent_or_zero;
            let model = by_model
                .entry((file.agent, Arc::clone(&entry.model)))
                .or_insert((0.0, spent.map(|_| 0.0)));
            model.0 += tokens;
            if let (Some(sum), Some(spent)) = (model.1.as_mut(), spent) {
                *sum += spent;
            }

            let (key, place) = match entry.cwd.as_deref() {
                Some(cwd) => match place_of(cwd, places) {
                    Some(place) => (
                        format!("ws:{}", place.workspace_id),
                        PlaceUsage {
                            label: place.workspace.clone(),
                            project: Some(place.project.clone()),
                            workspace_id: Some(place.workspace_id.clone()),
                            folder: None,
                            tokens: 0.0,
                            cost: 0.0,
                        },
                    ),
                    None => (
                        format!("dir:{cwd}"),
                        PlaceUsage {
                            label: Path::new(cwd)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| cwd.to_owned()),
                            project: None,
                            workspace_id: None,
                            folder: Some(tilde(Path::new(cwd), home)),
                            tokens: 0.0,
                            cost: 0.0,
                        },
                    ),
                },
                None => (
                    "unknown".to_owned(),
                    PlaceUsage {
                        label: "Folder not recorded".to_owned(),
                        project: None,
                        workspace_id: None,
                        folder: None,
                        tokens: 0.0,
                        cost: 0.0,
                    },
                ),
            };
            let place = by_place.entry(key).or_insert(place);
            place.tokens += tokens;
            place.cost += spent_or_zero;
        }
    }

    let agents: Vec<Agent> = Agent::ALL
        .into_iter()
        .filter(|agent| {
            by_agent.contains_key(agent) || file_counts.get(agent).copied().unwrap_or(0) > 0
        })
        .collect();

    let mut models: Vec<ModelUsage> = by_model
        .into_iter()
        .map(|((agent, model), (tokens, known_cost))| ModelUsage {
            model: model.to_string(),
            agent: agent.id().to_owned(),
            tokens,
            known_cost,
        })
        .collect();
    models.sort_by(|a, b| {
        b.known_cost
            .unwrap_or(-1.0)
            .total_cmp(&a.known_cost.unwrap_or(-1.0))
            .then(b.tokens.total_cmp(&a.tokens))
            .then(a.model.cmp(&b.model))
    });
    let mut places: Vec<PlaceUsage> = by_place.into_values().collect();
    places.sort_by(|a, b| {
        b.cost
            .total_cmp(&a.cost)
            .then(b.tokens.total_cmp(&a.tokens))
            .then(a.label.cmp(&b.label))
    });

    let mut limits: Vec<AgentLimits> = limits
        .into_iter()
        .map(|(agent, found)| AgentLimits {
            agent: agent.id().to_owned(),
            plan: found.plan.clone(),
            observed_at: found.observed_at_ms as f64,
            windows: found
                .windows
                .iter()
                .map(|w| LimitWindowDto {
                    minutes: w.minutes,
                    used_percent: w.used_percent,
                    resets_at: w.resets_at_ms.map(|ms| ms as f64),
                })
                .collect(),
        })
        .collect();
    limits.sort_by(|a, b| a.agent.cmp(&b.agent));

    UsageReport {
        days: request.days.max(1),
        from: date_of(first_day),
        to: date_of(first_day + days as i64 - 1),
        cost,
        unpriced_models: unpriced.into_keys().collect(),
        totals,
        daily: daily
            .iter()
            .enumerate()
            .map(|(i, day)| DayUsage {
                date: date_of(first_day + i as i64),
                cost: agents
                    .iter()
                    .map(|a| day.get(a).map_or(0.0, |v| v.1))
                    .collect(),
                tokens: agents
                    .iter()
                    .map(|a| day.get(a).map_or(0.0, |v| v.0))
                    .collect(),
            })
            .collect(),
        agents: agents
            .iter()
            .map(|agent| {
                let (tokens, cost) = by_agent.get(agent).copied().unwrap_or_default();
                AgentUsage {
                    agent: agent.id().to_owned(),
                    tokens,
                    cost,
                    location: sources.location(*agent).map(|dir| tilde(&dir, home)),
                    files: file_counts.get(agent).copied().unwrap_or(0),
                }
            })
            .collect(),
        models,
        places,
        limits,
    }
}

#[cfg(test)]
mod tests {
    use super::logs::{Entry, Parsed};
    use super::*;

    const NOW: i64 = 1_790_000_000_000; // 2026-09-21T14:13:20Z

    fn entry(at_ms: i64, model: &str, cwd: &str, tokens: Tokens, key: Option<u64>) -> Entry {
        Entry {
            at_ms,
            model: Arc::from(model),
            cwd: Some(Arc::from(cwd)),
            tokens,
            key,
        }
    }

    fn file(agent: Agent, entries: Vec<Entry>) -> File {
        File {
            agent,
            parsed: Arc::new(Parsed {
                entries,
                limits: None,
            }),
        }
    }

    fn million(input: u64, output: u64) -> Tokens {
        Tokens {
            input: input * 1_000_000,
            output: output * 1_000_000,
            ..Tokens::default()
        }
    }

    fn request(days: u32) -> Request {
        Request {
            days,
            now_ms: NOW,
            utc_offset_minutes: 0,
        }
    }

    fn places() -> Vec<Place> {
        vec![
            Place {
                path: "/code/app".into(),
                project: "app".into(),
                workspace: "local".into(),
                workspace_id: "w-local".into(),
            },
            // A worktree inside the project's folder is its own place.
            Place {
                path: "/code/app/.worktrees/fix".into(),
                project: "app".into(),
                workspace: "fix".into(),
                workspace_id: "w-fix".into(),
            },
        ]
    }

    fn run(files: &[File], days: u32) -> UsageReport {
        let counts = files.iter().fold(HashMap::new(), |mut counts, f| {
            *counts.entry(f.agent).or_insert(0) += 1;
            counts
        });
        report(
            files,
            &counts,
            &Sources::default(),
            None,
            &places(),
            &request(days),
        )
    }

    #[test]
    fn a_call_is_priced_counted_once_and_put_on_its_day_and_workspace() {
        let files = vec![
            file(
                Agent::Claude,
                vec![entry(
                    NOW,
                    "claude-opus-5-5",
                    "/code/app/src",
                    million(1, 1),
                    Some(7),
                )],
            ),
            // The same call again, from a resumed conversation's copy of it.
            file(
                Agent::Claude,
                vec![entry(
                    NOW,
                    "claude-opus-5-5",
                    "/code/app/src",
                    million(1, 1),
                    Some(7),
                )],
            ),
            file(
                Agent::Codex,
                vec![entry(
                    NOW - DAY_MS,
                    "gpt-6-sol",
                    "/code/app/.worktrees/fix",
                    Tokens {
                        input: 1_000_000,
                        cache_read: 2_000_000,
                        ..Tokens::default()
                    },
                    Some(8),
                )],
            ),
        ];
        let report = run(&files, 7);

        // $4 in + $20 out for Opus 5.5; $2 in + 2 × $0.20 cached for gpt-6-sol.
        assert!((report.cost - 26.4).abs() < 1e-9, "{}", report.cost);
        assert_eq!(report.totals.processed, 5_000_000.0);
        assert_eq!(report.totals.cache_read, 2_000_000.0);
        assert!((report.totals.cache_savings - 3.6).abs() < 1e-9);
        assert_eq!(report.daily.len(), 7);
        assert_eq!(report.to, "2026-09-21");
        assert_eq!(report.from, "2026-09-15");
        let agents: Vec<&str> = report.agents.iter().map(|a| a.agent.as_str()).collect();
        assert_eq!(agents, ["claude", "codex"]);
        assert_eq!(report.daily[6].cost, vec![24.0, 0.0]);
        assert!((report.daily[5].cost[1] - 2.4).abs() < 1e-9);

        let labels: Vec<(&str, Option<&str>)> = report
            .places
            .iter()
            .map(|p| (p.label.as_str(), p.workspace_id.as_deref()))
            .collect();
        assert_eq!(labels, [("local", Some("w-local")), ("fix", Some("w-fix"))]);
        assert_eq!(report.models[0].model, "claude-opus-5-5");
    }

    #[test]
    fn an_unknown_model_counts_its_tokens_and_says_its_cost_is_unknown() {
        let files = vec![file(
            Agent::Grok,
            vec![entry(
                NOW,
                "grok-4.7-build",
                "/elsewhere/tool",
                million(2, 0),
                Some(1),
            )],
        )];
        let report = run(&files, 7);
        assert_eq!(report.cost, 0.0);
        assert_eq!(report.unpriced_models, ["grok-4.7-build"]);
        assert_eq!(report.models[0].known_cost, None);
        assert_eq!(report.totals.processed, 2_000_000.0);
        // A folder Yardsort does not know is named by its last part.
        assert_eq!(report.places[0].label, "tool");
        assert_eq!(report.places[0].folder.as_deref(), Some("/elsewhere/tool"));
    }

    #[test]
    fn calls_before_the_range_are_left_out() {
        let files = vec![file(
            Agent::Claude,
            vec![
                entry(
                    NOW - 8 * DAY_MS,
                    "claude-opus-5-5",
                    "/code/app",
                    million(1, 0),
                    None,
                ),
                entry(NOW, "claude-opus-5-5", "/code/app", million(1, 0), None),
            ],
        )];
        assert_eq!(run(&files, 7).totals.processed, 1_000_000.0);
        assert_eq!(run(&files, 30).totals.processed, 2_000_000.0);
    }

    #[test]
    fn a_day_is_the_viewers_day() {
        // 23:30 UTC on the 20th is already the 21st two hours east.
        let late = NOW - 14 * 3_600_000 - 13 * 60_000 - 20_000 - 30 * 60_000;
        let files = vec![file(
            Agent::Claude,
            vec![entry(
                late,
                "claude-opus-5-5",
                "/code/app",
                million(1, 0),
                None,
            )],
        )];
        let mut east = request(2);
        east.utc_offset_minutes = 120;
        let counts = HashMap::from([(Agent::Claude, 1)]);
        let report = report(&files, &counts, &Sources::default(), None, &[], &east);
        assert_eq!(report.daily[1].date, "2026-09-21");
        assert_eq!(report.daily[1].tokens, vec![1_000_000.0]);
        assert_eq!(run(&files, 2).daily[0].tokens, vec![1_000_000.0]);
    }

    #[test]
    fn dates_and_the_home_folder_read_as_people_write_them() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(20_727), "2026-10-01");
        assert_eq!(
            tilde(Path::new("/home/me/.codex"), Some(Path::new("/home/me"))),
            Path::new("~").join(".codex").to_string_lossy()
        );
        assert_eq!(
            tilde(Path::new("/opt/x"), Some(Path::new("/home/me"))),
            "/opt/x"
        );
    }
}

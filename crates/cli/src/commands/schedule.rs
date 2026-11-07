//! `ys schedule …` — standing instructions to start a workflow or a smoke test: once at a set
//! time, or again and again.
//!
//! A schedule is a row; `ys` writes it and the app, when it is open, acts on it within a
//! second. See `yardsort_core::schedule` and docs/guide/schedules.md.

use std::collections::BTreeMap;

use serde::Serialize;
use yardsort_core::memory;
use yardsort_core::presence;
use yardsort_core::schedule::{
    self, clock, fire, when, Draft, IfMissed, ScheduleNotify, ScheduleSpec, ScheduleStanding,
    ScheduleTarget, TimeOfDay, Weekday,
};
use yardsort_core::store::{ScheduleRow, ScheduleRunRow};

use super::workflow::Looking;
use crate::{table, Failure, Output, Yardsort};

/// How many times to come `show` and `add` print.
const UPCOMING: usize = 5;

#[derive(clap::Subcommand)]
pub enum Command {
    /// Every schedule: what it runs, where, when, and how its last time went.
    List,
    /// One schedule: its inputs, its next times, and anything it is waiting on.
    Show {
        /// The schedule: its name, its id, or the start of its id.
        schedule: String,
    },
    /// Add a schedule. Give what to run (`--workflow` or `--test`) and when (one of `--at`,
    /// `--every`, `--daily`, `--weekly`, `--monthly`, `--cron`). Times are this machine's
    /// local time. Schedules fire while Yardsort is open.
    Add(Box<Add>),
    /// Switch a schedule off. It keeps its place and its history, and fires nothing.
    Pause { schedule: String },
    /// Switch a schedule back on, from now: times that passed while it was off are not made
    /// up for.
    Resume { schedule: String },
    /// Start a schedule's run now. Yardsort must be open. Its next time is left where it is.
    Run { schedule: String },
    /// A schedule's history: every time it came due and what became of it, newest first.
    Runs {
        schedule: String,
        /// How many to list.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Remove a schedule and its history. The runs it started stay where they are.
    Remove { schedule: String },
}

/// What `add` takes.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("what").required(true).args(["workflow", "test"])))]
#[command(group(clap::ArgGroup::new("when").required(true).args(
    ["at", "every", "daily", "weekly", "monthly", "cron"]
)))]
pub struct Add {
    /// Run this workflow, from `ys workflow list`.
    #[arg(long, value_name = "ID")]
    pub workflow: Option<String>,
    /// Run this smoke test, from `ys test list`.
    #[arg(long, value_name = "ID")]
    pub test: Option<String>,
    /// Which workspace to run it in, by name or id. Default: the one this is run in.
    #[arg(long, value_name = "WORKSPACE")]
    pub workspace: Option<String>,
    /// An answer to one of its inputs. Repeat for each.
    #[arg(long = "input", value_name = "ID=VALUE")]
    pub inputs: Vec<String>,
    /// What to call it. Default: the workflow's or the test's own name.
    #[arg(long)]
    pub name: Option<String>,
    /// Once, at this date and time: "2026-11-06 18:00".
    #[arg(long, value_name = "DATE TIME")]
    pub at: Option<String>,
    /// Again every so often: 15m, 2h, 1d.
    #[arg(long, value_name = "INTERVAL")]
    pub every: Option<String>,
    /// Every day at this time: 09:00.
    #[arg(long, value_name = "TIME")]
    pub daily: Option<String>,
    /// On these days at this time: mon-fri 09:00, or sat,sun 10:30.
    #[arg(long, num_args = 2, value_names = ["DAYS", "TIME"])]
    pub weekly: Option<Vec<String>>,
    /// On this day of the month, 1 to 28 or `last`, at this time: 1 09:00.
    #[arg(long, num_args = 2, value_names = ["DAY", "TIME"])]
    pub monthly: Option<Vec<String>>,
    /// A five-field cron expression: "0 9 * * 1-5".
    #[arg(long, value_name = "EXPRESSION")]
    pub cron: Option<String>,
    /// What to do with a time that passed while Yardsort was not running.
    #[arg(long, value_enum, default_value_t = MissedArg::RunOnce)]
    pub if_missed: MissedArg,
    /// When to be told how a run ended.
    #[arg(long, value_enum, default_value_t = NotifyArg::Failure)]
    pub notify: NotifyArg,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum MissedArg {
    /// One run when Yardsort is next open, however many times were missed.
    RunOnce,
    /// Let it go, and wait for the next time.
    Skip,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum NotifyArg {
    Failure,
    Always,
    Never,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List => list(ys, out),
        Command::Show { schedule } => show(ys, &schedule, out),
        Command::Add(asked) => {
            let Add {
                workflow,
                test,
                workspace,
                inputs,
                name,
                at,
                every,
                daily,
                weekly,
                monthly,
                cron,
                if_missed,
                notify,
            } = *asked;
            let (target, target_id) = match (workflow, test) {
                (Some(id), _) => (ScheduleTarget::Workflow, id),
                (None, Some(id)) => (ScheduleTarget::Test, id),
                (None, None) => return Err(Failure::new("Say what to run: --workflow or --test.")),
            };
            let spec = spec_of(at, every, daily, weekly, monthly, cron)?;
            add(
                ys,
                Asked {
                    target,
                    target_id,
                    workspace,
                    inputs,
                    name,
                    spec,
                    missed: match if_missed {
                        MissedArg::RunOnce => IfMissed::RunOnce,
                        MissedArg::Skip => IfMissed::Skip,
                    },
                    notify: match notify {
                        NotifyArg::Failure => ScheduleNotify::Failure,
                        NotifyArg::Always => ScheduleNotify::Always,
                        NotifyArg::Never => ScheduleNotify::Never,
                    },
                },
                out,
            )
        }
        Command::Pause { schedule } => switch(ys, &schedule, false, out),
        Command::Resume { schedule } => switch(ys, &schedule, true, out),
        Command::Run { schedule } => run_now(ys, &schedule, out),
        Command::Runs { schedule, limit } => runs(ys, &schedule, limit, out),
        Command::Remove { schedule } => remove(ys, &schedule, out),
    }
}

/// The one way of saying when that was given, as a spec. Clap has already made sure it is one.
fn spec_of(
    at: Option<String>,
    every: Option<String>,
    daily: Option<String>,
    weekly: Option<Vec<String>>,
    monthly: Option<Vec<String>>,
    cron: Option<String>,
) -> Result<ScheduleSpec, Failure> {
    let time = |text: &str| {
        TimeOfDay::parse(text).ok_or_else(|| {
            Failure::new(format!(
                "`{text}` is not a time of day. Write it as 09:00 or 18:30."
            ))
        })
    };
    if let Some(at) = at {
        let at = clock::parse_local(&at).ok_or_else(|| {
            Failure::new(format!(
                "`{at}` is not a date and time. Write it as \"2026-11-06 18:00\"."
            ))
        })?;
        return Ok(ScheduleSpec::Once { at });
    }
    if let Some(every) = every {
        return Ok(ScheduleSpec::Every {
            minutes: interval(&every)?,
        });
    }
    if let Some(daily) = daily {
        return Ok(ScheduleSpec::Daily {
            time: time(&daily)?,
        });
    }
    if let Some(weekly) = weekly {
        let [days, at] = weekly.as_slice() else {
            return Err(Failure::new(
                "--weekly takes days and a time: mon-fri 09:00.",
            ));
        };
        return Ok(ScheduleSpec::Weekly {
            days: weekdays(days)?,
            time: time(at)?,
        });
    }
    if let Some(monthly) = monthly {
        let [day, at] = monthly.as_slice() else {
            return Err(Failure::new("--monthly takes a day and a time: 1 09:00."));
        };
        let day = if day.eq_ignore_ascii_case("last") {
            None
        } else {
            Some(day.parse::<u8>().map_err(|_| {
                Failure::new(format!(
                    "`{day}` is not a day of the month. Give 1 to 28, or `last`."
                ))
            })?)
        };
        return Ok(ScheduleSpec::Monthly {
            day,
            time: time(at)?,
        });
    }
    match cron {
        Some(expr) => Ok(ScheduleSpec::Cron { expr }),
        None => Err(Failure::new(
            "Say when: --at, --every, --daily, --weekly, --monthly or --cron.",
        )),
    }
}

/// `15m`, `2h`, `1d`, or a bare number of minutes, as minutes.
fn interval(text: &str) -> Result<u32, Failure> {
    let text = text.trim().to_ascii_lowercase();
    let (number, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text.as_str(), "m"),
    };
    let each = match unit.trim() {
        "m" | "min" | "mins" | "minute" | "minutes" => Some(1),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(60),
        "d" | "day" | "days" => Some(24 * 60),
        _ => None,
    };
    number
        .parse::<u32>()
        .ok()
        .zip(each)
        .and_then(|(number, each)| number.checked_mul(each))
        .ok_or_else(|| {
            Failure::new(format!(
                "`{text}` is not an interval. Write it as 15m, 2h or 1d."
            ))
        })
}

/// `mon-fri`, `sat,sun`, `mon,wed-fri`, `weekdays`, `weekends`.
fn weekdays(text: &str) -> Result<Vec<Weekday>, Failure> {
    let not_days = || {
        Failure::new(format!(
            "`{text}` is not days of the week. Write them as mon-fri, or sat,sun."
        ))
    };
    let place = |day: Weekday| Weekday::ALL.iter().position(|d| *d == day).unwrap_or(0);
    let mut days = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        match part.to_ascii_lowercase().as_str() {
            "weekdays" => days.extend_from_slice(&Weekday::ALL[..5]),
            "weekends" => days.extend_from_slice(&Weekday::ALL[5..]),
            _ => match part.split_once('-') {
                Some((from, to)) => {
                    let from = place(Weekday::parse(from).ok_or_else(not_days)?);
                    let to = place(Weekday::parse(to).ok_or_else(not_days)?);
                    // `fri-mon` goes round the end of the week.
                    let count = (to + 7 - from) % 7 + 1;
                    days.extend((0..count).map(|step| Weekday::ALL[(from + step) % 7]));
                }
                None => days.push(Weekday::parse(part).ok_or_else(not_days)?),
            },
        }
    }
    days.sort();
    days.dedup();
    Ok(days)
}

/// A schedule by its id, the start of its id, or its name.
fn find(ys: &Yardsort, wanted: &str) -> Result<ScheduleRow, Failure> {
    let wanted = wanted.trim();
    let all = ys.store.schedules()?;
    if let Some(exact) = all.iter().find(|s| s.id == wanted) {
        return Ok(exact.clone());
    }
    let matches: Vec<&ScheduleRow> = all
        .iter()
        .filter(|s| {
            !wanted.is_empty() && (s.name.eq_ignore_ascii_case(wanted) || s.id.starts_with(wanted))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] if all.is_empty() => Err(Failure::new(
            "There are no schedules. `ys schedule add` makes one.",
        )),
        [] => Err(Failure::new(format!(
            "No schedule called {wanted:?}. There are: {}.",
            all.iter()
                .map(|s| format!("{} ({})", s.name, memory::short_id(&s.id)))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        many => Err(Failure::new(format!(
            "{} schedules match {wanted:?}. Use the id instead: {}.",
            many.len(),
            many.iter()
                .map(|s| format!("{} ({})", memory::short_id(&s.id), s.workspace_name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A schedule as `list` and `show` print it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    id: String,
    name: String,
    target: String,
    target_id: String,
    workspace: String,
    workspace_id: Option<String>,
    inputs: BTreeMap<String, String>,
    spec: Option<ScheduleSpec>,
    when: String,
    if_missed: String,
    notify: String,
    enabled: bool,
    next_at: Option<i64>,
    standing: ScheduleStanding,
    last: Option<Line>,
}

/// One line of a schedule's history.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    due_at: i64,
    fired_at: i64,
    outcome: String,
    manual: bool,
    run: Option<String>,
    result: Option<String>,
    note: Option<String>,
}

impl From<&ScheduleRunRow> for Line {
    fn from(run: &ScheduleRunRow) -> Self {
        Self {
            due_at: run.due_at,
            fired_at: run.fired_at,
            outcome: run.outcome.clone(),
            manual: run.manual,
            run: run.run_id.clone(),
            result: run.result.clone(),
            note: run.note.clone(),
        }
    }
}

fn listed(
    row: &ScheduleRow,
    last: Option<&ScheduleRunRow>,
    going: Option<&ScheduleRunRow>,
) -> Listed {
    let spec = ScheduleSpec::parse(&row.spec);
    Listed {
        id: row.id.clone(),
        name: row.name.clone(),
        target: row.target_kind.clone(),
        target_id: row.target_id.clone(),
        workspace: row.workspace_name.clone(),
        workspace_id: row.workspace_id.clone(),
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        when: words(spec.as_ref()),
        spec,
        if_missed: row.missed.clone(),
        notify: row.notify.clone(),
        enabled: row.enabled,
        next_at: row.next_at.filter(|_| row.enabled),
        standing: schedule::standing(row, last, going),
        last: last.map(Line::from),
    }
}

/// One schedule as it is listed, with its history read for where it stands.
fn listed_one(ys: &Yardsort, row: &ScheduleRow) -> Result<Listed, Failure> {
    let last = ys.store.schedule_runs(&row.id, 1)?.into_iter().next();
    let going = schedule::going_runs(&ys.store)?;
    Ok(listed(row, last.as_ref(), going.get(&row.id)))
}

fn words(spec: Option<&ScheduleSpec>) -> String {
    match spec {
        Some(spec) => schedule::describe(spec, &clock::now().timezone()),
        None => "unknown".to_owned(),
    }
}

/// Where a schedule stands, in a word or two.
fn standing_words(standing: ScheduleStanding) -> &'static str {
    match standing {
        ScheduleStanding::Waiting => "on",
        ScheduleStanding::Running => "running",
        ScheduleStanding::AwaitingApproval => "waiting for approval",
        ScheduleStanding::Refused => "refused",
        ScheduleStanding::NoWorkspace => "no workspace",
        ScheduleStanding::Off => "off",
        ScheduleStanding::Done => "done",
        ScheduleStanding::Missed => "missed",
    }
}

/// What became of a time, in a word or two: the run's ending when it has one.
fn outcome_words(run: &ScheduleRunRow) -> String {
    match (run.outcome.as_str(), run.result.as_deref()) {
        ("started", Some(result)) => result.to_owned(),
        ("started", None) if run.told_waiting => "waiting for approval".to_owned(),
        ("started", None) => "running".to_owned(),
        (outcome, _) => outcome.to_owned(),
    }
}

fn list(ys: &Yardsort, out: &Output) -> Result<(), Failure> {
    let rows = ys.store.schedules()?;
    let latest: BTreeMap<String, ScheduleRunRow> = ys
        .store
        .latest_schedule_runs()?
        .into_iter()
        .map(|run| (run.schedule_id.clone(), run))
        .collect();
    let going = schedule::going_runs(&ys.store)?;
    let all: Vec<Listed> = rows
        .iter()
        .map(|row| listed(row, latest.get(&row.id), going.get(&row.id)))
        .collect();
    out.emit(&all, || {
        if rows.is_empty() {
            sayln!("No schedules. `ys schedule add` makes one.");
            return;
        }
        let mut lines = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "RUNS".to_owned(),
            "IN".to_owned(),
            "WHEN".to_owned(),
            "NEXT".to_owned(),
            "LAST".to_owned(),
            "".to_owned(),
        ]];
        lines.extend(rows.iter().zip(&all).map(|(row, item)| {
            vec![
                memory::short_id(&row.id).to_owned(),
                row.name.clone(),
                format!("{} {}", row.target_kind, row.target_id),
                row.workspace_name.clone(),
                item.when.clone(),
                item.next_at.map_or_else(|| "-".to_owned(), clock::show),
                latest
                    .get(&row.id)
                    .map_or_else(|| "-".to_owned(), outcome_words),
                standing_words(item.standing).to_owned(),
            ]
        }));
        table(&lines);
        not_running_note(ys);
    })
}

/// Said under anything that shows when a schedule will next fire: it will not, with no app.
fn not_running_note(ys: &Yardsort) {
    match presence::holder(&ys.data_dir) {
        Some(presence::Holder::App) => {}
        Some(presence::Holder::Host) => {
            sayln!();
            sayln!(
                "This host does not fire schedules yet: yardsort-host is running it, not the app."
            );
        }
        None => {
            sayln!();
            sayln!("Yardsort is not running. Schedules fire while it is open.");
        }
    }
}

fn show(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let row = find(ys, wanted)?;
    let last = ys.store.schedule_runs(&row.id, 1)?.into_iter().next();
    let going = schedule::going_runs(&ys.store)?;
    let item = listed(&row, last.as_ref(), going.get(&row.id));
    let upcoming: Vec<i64> = match (&item.spec, row.enabled) {
        (Some(spec), true) => when::upcoming(spec, &clock::now(), UPCOMING)
            .unwrap_or_default()
            .iter()
            .map(|at| at.timestamp_millis())
            .collect(),
        _ => Vec::new(),
    };
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Shown<'a> {
        #[serde(flatten)]
        schedule: &'a Listed,
        upcoming: &'a [i64],
    }
    out.emit(
        &Shown {
            schedule: &item,
            upcoming: &upcoming,
        },
        || {
            sayln!("{} ({})", row.name, memory::short_id(&row.id));
            sayln!(
                "Runs the {} `{}` in {}.",
                ScheduleTarget::parse(&row.target_kind).map_or("run", ScheduleTarget::words),
                row.target_id,
                row.workspace_name
            );
            for (id, value) in &item.inputs {
                sayln!("  {id} = {value}");
            }
            sayln!("When: {}", item.when);
            sayln!(
                "If a time is missed: {}. Tells you: {}.",
                match IfMissed::parse(&row.missed).unwrap_or_default() {
                    IfMissed::RunOnce => "runs once when Yardsort opens",
                    IfMissed::Skip => "skips it",
                },
                match ScheduleNotify::parse(&row.notify).unwrap_or_default() {
                    ScheduleNotify::Failure => "when a run fails",
                    ScheduleNotify::Always => "how every run ends",
                    ScheduleNotify::Never => "never",
                }
            );
            sayln!("It is {}.", standing_words(item.standing));
            if let Some(last) = &last {
                sayln!(
                    "Last: {} — {}{}",
                    clock::show(last.fired_at),
                    outcome_words(last),
                    last.note
                        .as_deref()
                        .map(|note| format!(". {note}"))
                        .unwrap_or_default()
                );
            }
            if !upcoming.is_empty() {
                sayln!();
                sayln!("Next:");
                for at in &upcoming {
                    sayln!("  {}", clock::show(*at));
                }
                not_running_note(ys);
            }
        },
    )
}

/// What `add` was given.
struct Asked {
    target: ScheduleTarget,
    target_id: String,
    workspace: Option<String>,
    inputs: Vec<String>,
    name: Option<String>,
    spec: ScheduleSpec,
    missed: IfMissed,
    notify: ScheduleNotify,
}

fn add(ys: &Yardsort, asked: Asked, out: &Output) -> Result<(), Failure> {
    let workspace = match asked.workspace.as_deref() {
        Some(wanted) => super::workspace::find_workspace(ys, wanted)?,
        None => super::workflow::here(ys)?,
    };
    let mut inputs = BTreeMap::new();
    for pair in &asked.inputs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(Failure::new(format!(
                "`--input {pair}` needs an `=`: `--input id=value`."
            )));
        };
        inputs.insert(key.trim().to_owned(), value.to_owned());
    }
    let row = schedule::add(
        &ys.store,
        &ys.data_dir,
        &Looking(ys),
        &Draft {
            name: asked.name.as_deref(),
            target: asked.target,
            target_id: &asked.target_id,
            workspace_id: &workspace.id,
            inputs: &inputs,
            spec: &asked.spec,
            missed: asked.missed,
            notify: asked.notify,
        },
        "cli",
        &clock::now(),
    )?;
    // `ys` approves nothing. A test file nobody has approved in the app waits there, once.
    let unapproved = asked.target == ScheduleTarget::Test && {
        let hash =
            yardsort_core::smoke::find(std::path::Path::new(&workspace.path), &asked.target_id)
                .map(|entry| yardsort_core::smoke::runs::content_hash(&entry.text));
        match hash {
            Some(hash) => {
                !ys.store
                    .test_approved(&workspace.project_id, &asked.target_id, &hash)?
            }
            None => false,
        }
    };
    let item = listed(&row, None, None);
    out.emit(&item, || {
        sayln!(
            "Added `{}` ({}): {}.",
            row.name,
            memory::short_id(&row.id),
            item.when
        );
        if let Some(next) = row.next_at {
            sayln!("Next: {}", clock::show(next));
        }
        if unapproved {
            sayln!(
                "Note: `{}` has not been approved in Yardsort, so its first run will wait for \
                 you there.",
                asked.target_id
            );
        }
        not_running_note(ys);
    })
}

fn switch(ys: &Yardsort, wanted: &str, enabled: bool, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    let row = schedule::set_enabled(&ys.store, &found.id, enabled, &clock::now())?;
    out.emit(&listed_one(ys, &row)?, || {
        if enabled {
            sayln!(
                "`{}` is on. Next: {}",
                row.name,
                row.next_at.map_or_else(|| "never".to_owned(), clock::show)
            );
            not_running_note(ys);
        } else {
            sayln!(
                "`{}` is off. `ys schedule resume` switches it back on.",
                row.name
            );
        }
    })
}

fn run_now(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    super::someone_carries_it_out(
        ys,
        "Yardsort is not running. Runs are carried out in the app: open it, then run this \
             again. Nothing was queued.",
    )?;
    let fired = fire::run_now(
        &ys.store,
        &ys.data_dir,
        &Looking(ys),
        &found.id,
        clock::now().timestamp_millis(),
    )?;
    let run_id = fired.run.run_id.clone().unwrap_or_default();
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Queued<'a> {
        schedule: &'a str,
        run: &'a str,
        target: &'a str,
    }
    out.emit(
        &Queued {
            schedule: &found.id,
            run: &run_id,
            target: &found.target_kind,
        },
        || {
            let short = memory::short_id(&run_id);
            sayln!("Queued `{}` as run {short}.", found.name);
            match ScheduleTarget::parse(&found.target_kind) {
                Some(ScheduleTarget::Test) => sayln!("Follow it with: ys test wait {short}"),
                _ => sayln!("Follow it with: ys workflow runs --run {short}"),
            }
        },
    )
}

fn runs(ys: &Yardsort, wanted: &str, limit: usize, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    let history = ys.store.schedule_runs(&found.id, limit)?;
    let lines: Vec<Line> = history.iter().map(Line::from).collect();
    out.emit(&lines, || {
        if history.is_empty() {
            sayln!("`{}` has not come due yet.", found.name);
            return;
        }
        let mut rows = vec![vec![
            "WHEN".to_owned(),
            "WHAT".to_owned(),
            "RUN".to_owned(),
            "NOTE".to_owned(),
        ]];
        rows.extend(history.iter().map(|run| {
            vec![
                clock::show(run.fired_at),
                outcome_words(run),
                run.run_id
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), |id| memory::short_id(id).to_owned()),
                match (run.manual, run.note.as_deref()) {
                    (true, _) => "Run now".to_owned(),
                    (false, Some(note)) => note.to_owned(),
                    // Acted on well after it was due: say when it was.
                    (false, None) if run.fired_at - run.due_at > fire::LATE_AFTER_MS => {
                        format!("Due {}", clock::show(run.due_at))
                    }
                    (false, None) => String::new(),
                },
            ]
        }));
        table(&rows);
    })
}

fn remove(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    ys.store.remove_schedule(&found.id)?;
    #[derive(Serialize)]
    struct Removed<'a> {
        schedule: &'a str,
    }
    out.emit(
        &Removed {
            schedule: &found.id,
        },
        || {
            sayln!(
                "Removed `{}` and its history. The runs it started are still there.",
                found.name
            );
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_are_read_in_minutes_hours_and_days() {
        let minutes = |text: &str| interval(text).ok();
        assert_eq!(minutes("15m"), Some(15));
        assert_eq!(minutes("2h"), Some(120));
        assert_eq!(minutes("1d"), Some(1440));
        assert_eq!(minutes("90"), Some(90));
        assert_eq!(minutes(" 3 Hours "), Some(180));
        for bad in ["", "h", "2w", "-5m", "1.5h", "99999999999d"] {
            assert_eq!(minutes(bad), None, "{bad}");
        }
    }

    #[test]
    fn days_are_read_as_lists_ranges_and_words() {
        use Weekday::*;
        let days = |text: &str| weekdays(text).ok();
        assert_eq!(days("mon-fri"), Some(vec![Mon, Tue, Wed, Thu, Fri]));
        assert_eq!(days("weekdays"), Some(vec![Mon, Tue, Wed, Thu, Fri]));
        assert_eq!(days("sat,sun"), Some(vec![Sat, Sun]));
        assert_eq!(days("Sunday, mon"), Some(vec![Mon, Sun]));
        assert_eq!(days("fri-mon"), Some(vec![Mon, Fri, Sat, Sun]));
        assert_eq!(days("mon,wed-thu,mon"), Some(vec![Mon, Wed, Thu]));
        for bad in ["", "funday", "mon-", "mon fri"] {
            assert_eq!(days(bad), None, "{bad}");
        }
    }

    #[test]
    fn exactly_one_way_of_saying_when_becomes_a_spec() {
        let none = || (None, None, None, None, None, None);
        let (at, every, daily, weekly, monthly, cron) = none();
        assert!(spec_of(at, every, daily, weekly, monthly, cron).is_err());

        let spec = spec_of(None, None, Some("9:00".into()), None, None, None)
            .ok()
            .unwrap();
        assert_eq!(
            spec,
            ScheduleSpec::Daily {
                time: TimeOfDay { hour: 9, minute: 0 }
            }
        );
        let spec = spec_of(
            None,
            None,
            None,
            None,
            Some(vec!["last".into(), "06:30".into()]),
            None,
        )
        .ok()
        .unwrap();
        assert_eq!(
            spec,
            ScheduleSpec::Monthly {
                day: None,
                time: TimeOfDay {
                    hour: 6,
                    minute: 30
                }
            }
        );
        assert!(spec_of(None, None, Some("nine".into()), None, None, None).is_err());
        assert!(spec_of(Some("tomorrow".into()), None, None, None, None, None).is_err());
    }
}

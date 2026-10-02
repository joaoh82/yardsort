//! The Usage view's side of IPC: what Yardsort and its agents use of this machine, and the
//! tokens the agents spent (read and priced by `yardsort_core::usage`).
//!
//! Machine resources are sampled when the view asks — every two seconds while it is open — and
//! never otherwise: walking every process on the machine costs something, and nobody is looking.
//! A terminal's share is its whole process tree, so an agent's language servers, test runs and
//! builds count as the agent's. Memory is resident memory, summed over the processes; pages they
//! share are counted once per process, as every task manager does.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pty_host::{SessionInfo, SessionState};
use serde::Serialize;
use specta::Type;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::AppHandle;
use yardsort_core::launch::{HARNESS_LABEL, RECORD_LABEL, WORKSPACE_LABEL};
use yardsort_core::usage::{self, LogCache, Place, Sources, UsageReport};

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// How far back the charts go.
const HISTORY: Duration = Duration::from_secs(5 * 60);
/// Two views asking at once get the same sample rather than a second walk.
const REUSE: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    /// Percent of the whole machine: every core counted, so it never passes 100.
    pub cpu: f64,
    /// Bytes resident.
    pub memory: f64,
    pub processes: u32,
}

impl Load {
    const ZERO: Load = Load {
        cpu: 0.0,
        memory: 0.0,
        processes: 0,
    };

    fn add(&mut self, other: Load) {
        self.cpu += other.cpu;
        self.memory += other.memory;
        self.processes += other.processes;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SystemLoad {
    pub total_memory: f64,
    pub used_memory: f64,
    pub available_memory: f64,
    pub cpu_count: u32,
    /// Percent of the whole machine in use, by anything.
    pub cpu: f64,
    /// The one-minute load average. Windows has none.
    pub load_one: Option<f64>,
}

/// A part of the app itself: its main process, its webview, the terminal host.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppPart {
    /// `main`, `webview` or `host`.
    pub part: String,
    pub load: Load,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalLoad {
    pub session_id: String,
    /// The conversation's title, else the harness's name; `None` for a plain shell or a run
    /// command, which the page names itself.
    pub label: Option<String>,
    pub harness: Option<String>,
    /// Set for a project's run command.
    pub run: bool,
    pub load: Load,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLoad {
    pub workspace_id: String,
    pub name: String,
    pub load: Load,
    pub terminals: Vec<TerminalLoad>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLoad {
    pub project_id: String,
    pub name: String,
    pub load: Load,
    pub workspaces: Vec<WorkspaceLoad>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    /// Epoch milliseconds.
    pub at: f64,
    pub cpu: f64,
    pub memory: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MachineReport {
    pub sampled_at: f64,
    /// Everything below together: the app, the terminal host and every terminal.
    pub yardsort: Load,
    pub system: SystemLoad,
    pub app: Vec<AppPart>,
    /// Projects with something running, heaviest first.
    pub projects: Vec<ProjectLoad>,
    /// Terminals in no workspace.
    pub loose: Vec<TerminalLoad>,
    /// Yardsort's own total, oldest first, over the last five minutes it was looked at.
    pub history: Vec<HistoryPoint>,
}

/// One process, as much as attribution needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    pub cpu: f64,
    pub memory: f64,
}

/// Whose each process is.
#[derive(Debug, Default, PartialEq)]
pub struct Attribution {
    pub main: Load,
    pub webview: Load,
    pub host: Load,
    /// By the session's process id.
    pub sessions: HashMap<u32, Load>,
}

impl Default for Load {
    fn default() -> Self {
        Load::ZERO
    }
}

/// Split the machine's processes between the app, its terminal host and each terminal. A
/// terminal's tree is taken first, so it is never counted again as the app's or the host's —
/// which matters when there is no daemon and terminals are the app's own children.
pub fn attribute(procs: &[Proc], app: u32, host: Option<u32>, sessions: &[u32]) -> Attribution {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs {
        if let Some(parent) = p.parent.filter(|parent| *parent != p.pid) {
            children.entry(parent).or_default().push(p.pid);
        }
    }
    let mut claimed: HashSet<u32> = HashSet::new();
    let tree = |root: u32, claimed: &mut HashSet<u32>| -> Load {
        let mut load = Load::ZERO;
        let mut stack = vec![root];
        while let Some(pid) = stack.pop() {
            if !claimed.insert(pid) {
                continue;
            }
            if let Some(p) = by_pid.get(&pid) {
                load.add(Load {
                    cpu: p.cpu,
                    memory: p.memory,
                    processes: 1,
                });
            }
            stack.extend(children.get(&pid).into_iter().flatten().copied());
        }
        load
    };

    let mut result = Attribution::default();
    for &pid in sessions {
        let load = tree(pid, &mut claimed);
        result.sessions.insert(pid, load);
    }
    if let Some(host) = host.filter(|host| *host != app) {
        result.host = tree(host, &mut claimed);
    }
    // The main process alone, then whatever else hangs below it: the webview's processes.
    if let Some(p) = by_pid.get(&app) {
        claimed.insert(app);
        result.main = Load {
            cpu: p.cpu,
            memory: p.memory,
            processes: 1,
        };
        for child in children.get(&app).into_iter().flatten() {
            let load = tree(*child, &mut claimed);
            result.webview.add(load);
        }
    }
    result
}

struct Sampler {
    system: System,
    primed: bool,
    last: Option<(Instant, MachineReport)>,
    history: VecDeque<HistoryPoint>,
}

/// Machine sampling and the parsed logs, kept between requests.
pub struct Usage {
    sampler: Mutex<Sampler>,
    logs: LogCache,
}

impl Default for Usage {
    fn default() -> Self {
        Self {
            sampler: Mutex::new(Sampler {
                system: System::new(),
                primed: false,
                last: None,
                history: VecDeque::new(),
            }),
            logs: LogCache::default(),
        }
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Sampler {
    fn refresh(&mut self) {
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .without_tasks(),
        );
    }

    fn procs(&self) -> Vec<Proc> {
        let cores = self.system.cpus().len().max(1) as f64;
        self.system
            .processes()
            .values()
            // Linux lists threads beside processes; their memory is their process's.
            .filter(|p| p.thread_kind().is_none())
            .map(|p| Proc {
                pid: p.pid().as_u32(),
                parent: p.parent().map(Pid::as_u32),
                cpu: f64::from(p.cpu_usage()) / cores,
                memory: p.memory() as f64,
            })
            .collect()
    }
}

/// What each running terminal is, by its process id.
struct Running<'a> {
    pid: u32,
    session: &'a SessionInfo,
}

fn terminal_load(state: &AppState, session: &SessionInfo, load: Load) -> TerminalLoad {
    let harness = session.labels.get(HARNESS_LABEL).cloned();
    let title = session
        .labels
        .get(RECORD_LABEL)
        .and_then(|record| state.store.session(record).ok().flatten())
        .map(|row| row.title)
        .filter(|title| !title.trim().is_empty());
    let harness_name = harness.as_deref().and_then(|id| {
        crate::harness::find(id, &state.settings.get().harnesses).map(|def| def.label)
    });
    TerminalLoad {
        session_id: session.id.0.clone(),
        label: title.or(harness_name),
        harness,
        run: session.labels.contains_key("projectRun"),
        load,
    }
}

fn sample(state: &AppState, usage: &Usage) -> IpcResult<MachineReport> {
    let mut sampler = usage.sampler.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, report)) = &sampler.last {
        if at.elapsed() < REUSE {
            return Ok(report.clone());
        }
    }
    // CPU use is a difference between two readings; the very first needs a second one.
    sampler.refresh();
    if !sampler.primed {
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
        sampler.refresh();
        sampler.primed = true;
    }

    let sessions = state.host.list();
    let running: Vec<Running<'_>> = sessions
        .iter()
        .filter(|s| matches!(s.state, SessionState::Running))
        .filter_map(|session| {
            Some(Running {
                pid: session.pid?,
                session,
            })
        })
        .collect();
    let pids: Vec<u32> = running.iter().map(|r| r.pid).collect();
    let host = state.daemon.pid.filter(|_| state.daemon.running);
    let split = attribute(&sampler.procs(), std::process::id(), host, &pids);

    let mut yardsort = Load::ZERO;
    yardsort.add(split.main);
    yardsort.add(split.webview);
    yardsort.add(split.host);
    let mut app = vec![
        AppPart {
            part: "main".into(),
            load: split.main,
        },
        AppPart {
            part: "webview".into(),
            load: split.webview,
        },
    ];
    if host.is_some() {
        app.push(AppPart {
            part: "host".into(),
            load: split.host,
        });
    }

    let mut projects: HashMap<String, ProjectLoad> = HashMap::new();
    let mut loose = Vec::new();
    for terminal in &running {
        let load = split
            .sessions
            .get(&terminal.pid)
            .copied()
            .unwrap_or_default();
        yardsort.add(load);
        let row = terminal_load(state, terminal.session, load);
        let workspace = terminal
            .session
            .labels
            .get(WORKSPACE_LABEL)
            .and_then(|id| state.store.workspace(id).ok().flatten());
        let Some(workspace) = workspace else {
            loose.push(row);
            continue;
        };
        let project = projects
            .entry(workspace.project_id.clone())
            .or_insert_with(|| ProjectLoad {
                project_id: workspace.project_id.clone(),
                name: state
                    .store
                    .project(&workspace.project_id)
                    .ok()
                    .flatten()
                    .map_or_else(|| workspace.project_id.clone(), |p| p.name),
                load: Load::ZERO,
                workspaces: Vec::new(),
            });
        project.load.add(load);
        match project
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == workspace.id)
        {
            Some(found) => {
                found.load.add(load);
                found.terminals.push(row);
            }
            None => project.workspaces.push(WorkspaceLoad {
                workspace_id: workspace.id.clone(),
                name: workspace.name.clone(),
                load,
                terminals: vec![row],
            }),
        }
    }
    let heaviest = |a: &Load, b: &Load| b.memory.total_cmp(&a.memory);
    let mut projects: Vec<ProjectLoad> = projects.into_values().collect();
    for project in &mut projects {
        for workspace in &mut project.workspaces {
            workspace
                .terminals
                .sort_by(|a, b| heaviest(&a.load, &b.load));
        }
        project
            .workspaces
            .sort_by(|a, b| heaviest(&a.load, &b.load));
    }
    projects.sort_by(|a, b| heaviest(&a.load, &b.load));
    loose.sort_by(|a, b| heaviest(&a.load, &b.load));

    let system = &sampler.system;
    let load_one = (!cfg!(windows)).then(|| System::load_average().one);
    let system = SystemLoad {
        total_memory: system.total_memory() as f64,
        used_memory: system.used_memory() as f64,
        available_memory: system.available_memory() as f64,
        cpu_count: u32::try_from(system.cpus().len()).unwrap_or(u32::MAX),
        cpu: f64::from(system.global_cpu_usage()),
        load_one,
    };

    let at = epoch_ms() as f64;
    sampler.history.push_back(HistoryPoint {
        at,
        cpu: yardsort.cpu,
        memory: yardsort.memory,
    });
    let oldest = at - HISTORY.as_millis() as f64;
    while sampler.history.front().is_some_and(|p| p.at < oldest) {
        sampler.history.pop_front();
    }
    let report = MachineReport {
        sampled_at: at,
        yardsort,
        system,
        app,
        projects,
        loose,
        history: sampler.history.iter().copied().collect(),
    };
    sampler.last = Some((Instant::now(), report.clone()));
    Ok(report)
}

/// What Yardsort and its agents use of this machine right now, with the last five minutes.
#[tauri::command]
#[specta::specta]
pub async fn usage_machine(app: AppHandle) -> IpcResult<MachineReport> {
    blocking(app, |state| sample(state, &state.usage)).await
}

/// The tokens the agents spent over the last `days` days, from their own logs on this machine.
/// `utc_offset_minutes` is the viewer's, so a day is theirs.
#[tauri::command]
#[specta::specta]
pub async fn usage_tokens(
    app: AppHandle,
    days: u32,
    utc_offset_minutes: i32,
) -> IpcResult<UsageReport> {
    if !(1..=366).contains(&days) {
        return Err(IpcError::new(
            "bad_range",
            "Usage covers from 1 to 366 days.",
        ));
    }
    blocking(app, move |state| {
        let env = state.env();
        let sources = Sources::from_env(&|name| env.get(name).map(str::to_owned));
        let request = usage::Request {
            days,
            now_ms: epoch_ms(),
            utc_offset_minutes,
        };
        let read = state.usage.logs.read(&sources, request.since_ms());
        let projects: HashMap<String, String> = state
            .store
            .projects()?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        let places: Vec<Place> = state
            .store
            .workspaces()?
            .into_iter()
            .map(|w| Place {
                project: projects.get(&w.project_id).cloned().unwrap_or_default(),
                path: w.path,
                workspace: w.name,
                workspace_id: w.id,
            })
            .collect();
        Ok(usage::report(
            &read.files,
            &read.counts,
            &sources,
            env.home_dir().as_deref(),
            &places,
            &request,
        ))
    })
    .await
}

/// Whether Usage is offered at the foot of the sidebar.
#[tauri::command]
#[specta::specta]
pub async fn settings_save_usage(
    app: AppHandle,
    show_in_sidebar: bool,
) -> IpcResult<crate::workspaces::commands::SettingsInfo> {
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| settings.usage.show_in_sidebar = show_in_sidebar)
            .map_err(|error| {
                IpcError::new(
                    "settings_write",
                    format!("Could not save settings: {error}"),
                )
            })?;
        crate::workspaces::commands::settings_info(state)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, memory: f64) -> Proc {
        Proc {
            pid,
            parent,
            cpu: 1.0,
            memory,
        }
    }

    #[test]
    fn a_terminal_owns_its_whole_tree_and_nothing_is_counted_twice() {
        // 1 app → 2 webview → 3 webview; 10 host → 11 agent → 12 its tool; 13 a shell.
        let procs = [
            proc(1, None, 100.0),
            proc(2, Some(1), 20.0),
            proc(3, Some(2), 5.0),
            proc(10, None, 7.0),
            proc(11, Some(10), 300.0),
            proc(12, Some(11), 50.0),
            proc(13, Some(10), 3.0),
            proc(99, None, 1000.0),
        ];
        let split = attribute(&procs, 1, Some(10), &[11, 13]);
        assert_eq!(split.main.memory, 100.0);
        assert_eq!(split.webview.memory, 25.0);
        assert_eq!(split.webview.processes, 2);
        assert_eq!(split.host.memory, 7.0, "the host without its terminals");
        assert_eq!(split.sessions[&11].memory, 350.0);
        assert_eq!(split.sessions[&11].processes, 2);
        assert_eq!(split.sessions[&13].memory, 3.0);
    }

    #[test]
    fn without_a_daemon_terminals_are_taken_out_of_the_apps_children() {
        let procs = [
            proc(1, None, 100.0),
            proc(2, Some(1), 20.0),
            proc(5, Some(1), 300.0),
        ];
        let split = attribute(&procs, 1, None, &[5]);
        assert_eq!(split.webview.memory, 20.0);
        assert_eq!(split.sessions[&5].memory, 300.0);
    }

    #[test]
    fn a_terminal_that_already_exited_counts_nothing() {
        let split = attribute(&[proc(1, None, 1.0)], 1, None, &[42]);
        assert_eq!(split.sessions[&42], Load::ZERO);
    }

    /// The real thing: this test process starts a child, which starts one of its own, and the
    /// sampler finds both under the child, with memory.
    #[test]
    fn a_real_process_tree_is_found_with_its_memory() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/C", "ping -n 10 127.0.0.1 > NUL"])
        } else {
            ("sh", &["-c", "sleep 10 & wait"])
        };
        let mut child = std::process::Command::new(program)
            .args(args)
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut sampler = Sampler {
            system: System::new(),
            primed: false,
            last: None,
            history: VecDeque::new(),
        };
        // The grandchild takes a moment to appear.
        let mut found = Load::ZERO;
        for _ in 0..50 {
            sampler.refresh();
            let split = attribute(&sampler.procs(), std::process::id(), None, &[pid]);
            found = split.sessions[&pid];
            if found.processes >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(found.processes >= 2, "{found:?}");
        assert!(found.memory > 0.0, "{found:?}");
    }
}

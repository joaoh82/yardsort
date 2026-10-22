//! The host's side of the remote protocol: an allowlisted, read-only facade over the store,
//! git and the terminal daemon.
//!
//! What a phone or another computer may ask is [`remote_proto::Operation`]; this module answers
//! it for one host. It is a translation layer, not a passage: each answer is built field by field
//! from what the app already shows, and the types it builds into ([`remote_proto::model`]) have
//! no room for a path, an argument list, an environment, a label or a byte of terminal output.
//!
//! **There is no listener here, and none may be added in front of this until devices are
//! authenticated.** Every call takes a [`Grant`], and nothing in this module makes one: in this
//! slice only tests do. Enrollment, approval at the host and revocation come first (phase R2 of
//! [the remote design](../../../docs/design/26-remote-r0-r1.md)).
//!
//! What is checked, in order, for every request:
//!
//! 1. The protocol major. Another one is `protocol_incompatible`.
//! 2. The request's own bounds.
//! 3. The grant's scope for the operation. Missing is `forbidden`.
//! 4. The project the request reaches, against the grant's projects. Outside it is `not_found`,
//!    exactly like an id that does not exist, so ids cannot be probed.
//!
//! Activity is read only from the `metadata` privacy class, and only through an allowlist of
//! fields, each checked again on the way out: see [`activity`].
//!
//! The terminal operations stop after step 3 with `unsupported`, and the host does not advertise
//! them: viewing and typing need control leases enforced across the desktop and every device,
//! which do not exist yet. This facade never writes to a terminal, never resizes one and never
//! attaches to one; [`TerminalHost::list`] is the only call it makes on the daemon.
//!
//! Nothing here writes to the store either. In particular there is no route to approve, edit or
//! propose memory: an agent can reach a terminal, and a terminal could one day reach a remote
//! client, so the only way to approve memory stays the desktop's Memory view.

use pty_host::{SessionInfo, SessionState, TerminalHost};
use remote_proto::{
    page_size, ActivityListParams, ActivityPage, BranchRef, Cursor, ErrorCode, Grant, Hello,
    HostDescription, HostId, MemoryId, MemoryListParams, MemoryNote, MemoryPage, Operation,
    Outcome, Platform, ProcessState, ProjectId, ProjectSummary, ProtocolVersion, RemoteError,
    Reply, Request, Response, SessionListParams, SessionPage, SessionRecordId, SessionSummary,
    TaskLink, TermSize, TerminalActivity, TerminalId, TerminalPurpose, TerminalSummary, Welcome,
    WorkspaceDetail, WorkspaceId, WorkspaceKind, WorkspaceListParams, WorkspacePage,
    WorkspaceSummary, WorkspaceTerminal, PROTOCOL_MAJOR,
};

use crate::git::Git;
use crate::launch::{HARNESS_LABEL, PROJECT_RUN_LABEL, RECORD_LABEL, RUN_LABEL, WORKSPACE_LABEL};
use crate::projects::{self, Projects};
use crate::store::{EventBound, EventFilter, ProjectRow, SessionRow, Store, WorkspaceRow};
use crate::tasks::delegate::TaskRef;

/// The operations this host answers. Advertised in [`HostDescription::capabilities`]; anything
/// else is `unsupported`.
pub const ANSWERED: &[&str] = &[
    "host.describe",
    "workspace.list",
    "workspace.describe",
    "session.list",
    "activity.list",
    "memory.list",
];

/// Who this host is, to remote clients. Supplied by whoever builds the facade: no host identity
/// is stored before Remote is enabled, so nothing is created for people who never use it.
#[derive(Debug, Clone)]
pub struct HostIdentity {
    pub id: HostId,
    pub name: String,
}

pub struct HostFacade<'a> {
    pub store: &'a Store,
    pub git: &'a Git,
    /// The terminal daemon, when one answers. Without it, a session's record is all there is to
    /// go on, and its process state is reported as unknown rather than guessed.
    pub terminals: Option<&'a dyn TerminalHost>,
    pub host: &'a HostIdentity,
}

type Answer<T> = Result<T, RemoteError>;

impl HostFacade<'_> {
    /// Answer a client's greeting, or say why this host cannot serve it.
    pub fn welcome(&self, hello: &Hello) -> Answer<Welcome> {
        Ok(Welcome {
            protocol: remote_proto::negotiate(hello)?,
            host: self.describe(),
        })
    }

    /// Answer one request read from untrusted bytes, as bytes to send back.
    pub fn handle_bytes(&self, grant: &Grant, bytes: &[u8]) -> Vec<u8> {
        let response = match remote_proto::decode_request(bytes) {
            Ok(request) => self.handle(grant, &request),
            Err(refusal) => *refusal,
        };
        remote_proto::encode_response(&response)
    }

    pub fn handle(&self, grant: &Grant, request: &Request) -> Response {
        Response {
            request_id: Some(request.request_id.clone()),
            outcome: match self.answer(grant, request) {
                Ok(reply) => Outcome::Result(reply),
                Err(error) => Outcome::Error(error),
            },
        }
    }

    fn answer(&self, grant: &Grant, request: &Request) -> Answer<Reply> {
        if request.protocol.major != PROTOCOL_MAJOR {
            return Err(RemoteError::incompatible());
        }
        let operation = &request.operation;
        operation.validate()?;
        if let Some(scope) = operation.scope() {
            if !grant.allows(scope) {
                return Err(RemoteError::forbidden(scope));
            }
        }
        match operation {
            Operation::HostDescribe => Ok(Reply::Host(self.describe())),
            Operation::WorkspaceList(params) => {
                self.workspaces(grant, params).map(Reply::Workspaces)
            }
            Operation::WorkspaceDescribe(params) => self
                .workspace(grant, &params.workspace)
                .map(Reply::Workspace),
            Operation::SessionList(params) => self.sessions(grant, params).map(Reply::Sessions),
            Operation::ActivityList(params) => self.activity(grant, params).map(Reply::Activity),
            Operation::MemoryList(params) => self.memory(grant, params).map(Reply::Memory),
            Operation::TerminalAttach(_)
            | Operation::TerminalDetach(_)
            | Operation::TerminalAcquire(_)
            | Operation::TerminalRenew(_)
            | Operation::TerminalRelease(_)
            | Operation::TerminalInput(_)
            | Operation::TerminalResize(_) => Err(RemoteError::new(
                ErrorCode::Unsupported,
                "Remote terminals are not available on this host yet.",
            )),
        }
    }

    fn describe(&self) -> HostDescription {
        HostDescription {
            host: self.host.id.clone(),
            name: self.host.name.clone(),
            platform: Platform::current(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol: ProtocolVersion::CURRENT,
            capabilities: ANSWERED.iter().map(|&op| op.to_owned()).collect(),
        }
    }

    fn workspaces(&self, grant: &Grant, params: &WorkspaceListParams) -> Answer<WorkspacePage> {
        let projects = Projects {
            store: self.store,
            git: self.git,
        }
        .list()
        .map_err(|_| unreadable())?;

        let mut summaries = Vec::new();
        let mut shown = Vec::new();
        let mut wanted_found = params.project.is_none();
        for project in projects {
            let id = local_id::<ProjectId>(&project.id)?;
            if !grant.reaches(&id) {
                continue;
            }
            if let Some(wanted) = &params.project {
                if *wanted != id {
                    continue;
                }
                wanted_found = true;
            }
            shown.push(ProjectSummary {
                id: id.clone(),
                name: project.name.clone(),
                missing: project.missing,
            });
            for workspace in project.workspaces {
                if workspace.archived && !params.include_archived {
                    continue;
                }
                let running = self
                    .store
                    .sessions(&workspace.id)
                    .map_err(|_| unreadable())?
                    .iter()
                    .filter(|s| s.running)
                    .count();
                summaries.push(summary(&workspace, running)?);
            }
        }
        if !wanted_found {
            return Err(RemoteError::not_found());
        }

        let (workspaces, next) = page(
            summaries,
            |w| w.id.as_str(),
            params.cursor.as_ref(),
            params.limit,
        )?;
        // Only the projects this page needs, in the order they were listed.
        shown.retain(|p| workspaces.iter().any(|w| w.project == p.id));
        Ok(WorkspacePage {
            projects: shown,
            workspaces,
            next,
        })
    }

    fn workspace(&self, grant: &Grant, id: &WorkspaceId) -> Answer<WorkspaceDetail> {
        let (row, project) = self.reachable_workspace(grant, id)?;
        let running = self
            .store
            .sessions(&row.id)
            .map_err(|_| unreadable())?
            .iter()
            .filter(|s| s.running)
            .count();
        let described = Projects {
            store: self.store,
            git: self.git,
        }
        .describe_workspace(row);
        Ok(WorkspaceDetail {
            workspace: summary(&described, running)?,
            project: ProjectSummary {
                id: local_id(&project.id)?,
                missing: !std::path::Path::new(&project.root_path).is_dir(),
                name: project.name,
            },
        })
    }

    fn sessions(&self, grant: &Grant, params: &SessionListParams) -> Answer<SessionPage> {
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        // Asked once for the whole list, so every session is judged against the same moment.
        let live: Option<Vec<SessionInfo>> = self.terminals.map(|host| host.list());
        let records = self
            .store
            .sessions(&workspace.id)
            .map_err(|_| unreadable())?;
        let terminals = match (&live, &params.cursor) {
            (Some(live), None) => self.other_terminals(&workspace.id, &records, live)?,
            _ => Vec::new(),
        };
        let sessions = records
            .into_iter()
            .filter(|row| params.include_ended || row.running)
            .map(|row| session(row, &workspace.id, live.as_deref()))
            .collect::<Answer<Vec<_>>>()?;
        let (sessions, next) = page(
            sessions,
            |s| s.id.as_str(),
            params.cursor.as_ref(),
            params.limit,
        )?;
        Ok(SessionPage {
            sessions,
            terminals,
            next,
        })
    }

    /// The daemon's terminals in this workspace that belong to none of its conversations.
    ///
    /// A terminal belongs to a conversation when a record of this workspace names it as its
    /// process, or when its own label names a record of this workspace (an earlier process of a
    /// resumed conversation). Its purpose comes from its labels and its activity run, never from
    /// its command line, which stays on the host with its arguments.
    fn other_terminals(
        &self,
        workspace_id: &str,
        records: &[SessionRow],
        live: &[SessionInfo],
    ) -> Answer<Vec<WorkspaceTerminal>> {
        let belongs = |info: &SessionInfo| {
            records.iter().any(|row| {
                row.pty_session_id.as_deref() == Some(info.id.0.as_str())
                    || info.labels.get(RECORD_LABEL) == Some(&row.id)
            })
        };
        live.iter()
            .filter(|info| {
                info.labels.get(WORKSPACE_LABEL).map(String::as_str) == Some(workspace_id)
            })
            .filter(|info| !belongs(info))
            .map(|info| {
                let harness = info.labels.get(HARNESS_LABEL).cloned();
                let purpose = if harness.is_some() {
                    TerminalPurpose::Agent
                } else if info.labels.get(PROJECT_RUN_LABEL).map(String::as_str)
                    == Some(workspace_id)
                {
                    TerminalPurpose::RunCommand
                } else {
                    self.run_purpose(info.labels.get(RUN_LABEL))?
                };
                Ok(WorkspaceTerminal {
                    purpose,
                    program: crate::activity::program_name(Some(&info.program)),
                    harness,
                    terminal: terminal_summary(info)?,
                })
            })
            .collect()
    }

    /// What the activity run a terminal was started as says it is, when it was recorded.
    fn run_purpose(&self, run: Option<&String>) -> Answer<TerminalPurpose> {
        let Some(run) = run else {
            return Ok(TerminalPurpose::Other);
        };
        Ok(match self.store.run(run).map_err(|_| unreadable())? {
            Some(row) if row.kind == "shell" => TerminalPurpose::Shell,
            Some(row) if row.kind == "program" => TerminalPurpose::Program,
            _ => TerminalPurpose::Other,
        })
    }

    /// A page of a workspace's activity, oldest first. See [`ActivityPage`] for how the cursors
    /// walk, and [`activity`] for what of each event is shown.
    fn activity(&self, grant: &Grant, params: &ActivityListParams) -> Answer<ActivityPage> {
        let (workspace, _) = self.reachable_workspace(grant, &params.workspace)?;
        if let Some(session) = &params.session {
            let theirs = self
                .store
                .session(session.as_str())
                .map_err(|_| unreadable())?
                .is_some_and(|row| row.workspace_id == workspace.id);
            if !theirs {
                return Err(RemoteError::not_found());
            }
        }
        let filter = EventFilter {
            workspace_id: &workspace.id,
            privacy_class: activity::SHOWN_CLASS,
            session_id: params.session.as_ref().map(SessionRecordId::as_str),
        };
        let bound = match &params.cursor {
            None => EventBound::Newest,
            Some(cursor) => {
                let (bound, seq) = activity_bound(cursor)?;
                // The event a cursor names must still be kept, with the same filter: otherwise
                // events may have gone from beside it, and the next page could skip some.
                if !self
                    .store
                    .event_matches(&filter, seq)
                    .map_err(|_| unreadable())?
                {
                    return Err(cursor_expired());
                }
                bound
            }
        };
        let size = page_size(params.limit);
        // One more than a page says whether there is another, without a count query.
        let mut rows = self
            .store
            .events_window(&filter, bound, size + 1)
            .map_err(|_| unreadable())?;
        let more = rows.len() > size;
        if more {
            match bound {
                // Oldest first: the extra one is the oldest, before the page.
                EventBound::Newest | EventBound::Before(_) => drop(rows.remove(0)),
                EventBound::After(_) => drop(rows.pop()),
            }
        }
        let older = match (rows.first(), bound) {
            // Walking forward from a cursor, the cursor's own event is older than this page.
            (Some(first), EventBound::After(_)) => Some(activity_cursor('o', first.seq)?),
            (Some(first), _) if more => Some(activity_cursor('o', first.seq)?),
            _ => None,
        };
        let newer = match (rows.last(), bound) {
            (Some(last), _) => Some(activity_cursor('n', last.seq)?),
            // Nothing new since the cursor: the same position, to ask from again.
            (None, EventBound::After(seq)) => Some(activity_cursor('n', seq)?),
            (None, _) => None,
        };
        Ok(ActivityPage {
            events: rows.iter().filter_map(activity::event).collect(),
            older,
            newer,
        })
    }

    /// Approved entries only, newest first.
    ///
    /// Not gated on the project's "share memory with agents" switch, as `ys memory list` is.
    /// That switch decides what reaches *agents*; a remote client is the person themselves,
    /// whose desktop Memory view shows every approved entry either way. What gates this is the
    /// grant's `memory:read`, which nothing infers from the local switch.
    fn memory(&self, grant: &Grant, params: &MemoryListParams) -> Answer<MemoryPage> {
        let project = self.reachable_project(grant, &params.project)?;
        let notes = crate::memory::approved(self.store, &project.id)
            .map_err(|_| unreadable())?
            .into_iter()
            .map(|entry| {
                Ok(MemoryNote {
                    id: local_id::<MemoryId>(&entry.id)?,
                    project: params.project.clone(),
                    text: entry.text,
                    author: entry.author,
                    harness: entry.harness_id,
                    created_at: entry.created_at,
                    updated_at: entry.updated_at,
                })
            })
            .collect::<Answer<Vec<_>>>()?;
        let (notes, next) = page(
            notes,
            |n| n.id.as_str(),
            params.cursor.as_ref(),
            params.limit,
        )?;
        Ok(MemoryPage { notes, next })
    }

    /// A project on show whose id the grant reaches. Removed projects are not on show.
    fn reachable_project(&self, grant: &Grant, id: &ProjectId) -> Answer<ProjectRow> {
        if !grant.reaches(id) {
            return Err(RemoteError::not_found());
        }
        self.store
            .projects()
            .map_err(|_| unreadable())?
            .into_iter()
            .find(|p| p.id == id.as_str())
            .ok_or_else(RemoteError::not_found)
    }

    /// A workspace on show, and its project, when the grant reaches that project. Forgotten
    /// workspaces and removed projects' workspaces are not on show, as in the app's sidebar.
    fn reachable_workspace(
        &self,
        grant: &Grant,
        id: &WorkspaceId,
    ) -> Answer<(WorkspaceRow, ProjectRow)> {
        let row = self
            .store
            .workspace(id.as_str())
            .map_err(|_| unreadable())?
            .filter(|w| !w.forgotten)
            .ok_or_else(RemoteError::not_found)?;
        let project = self.reachable_project(grant, &local_id(&row.project_id)?)?;
        Ok((row, project))
    }
}

/// The store would not answer. Its error text stays on the host: it can name files.
fn unreadable() -> RemoteError {
    RemoteError::new(
        ErrorCode::Internal,
        "The host could not read its own records. Try again in a moment.",
    )
}

/// A local id as a protocol id. Local ids are UUIDs, so this only fails on a damaged database.
fn local_id<T: TryFrom<String>>(text: &str) -> Answer<T> {
    T::try_from(text.to_owned()).map_err(|_| unreadable())
}

fn summary(workspace: &projects::Workspace, running: usize) -> Answer<WorkspaceSummary> {
    Ok(WorkspaceSummary {
        id: local_id(&workspace.id)?,
        project: local_id(&workspace.project_id)?,
        name: workspace.name.clone(),
        kind: match workspace.kind {
            projects::WorkspaceKind::Local => WorkspaceKind::Local,
            projects::WorkspaceKind::Worktree => WorkspaceKind::Worktree,
        },
        branch: workspace.head.as_ref().map(|head| BranchRef {
            label: head.label.clone(),
            detached: head.detached,
            unborn: head.unborn,
        }),
        archived: workspace.archived,
        missing: workspace.missing,
        tasks: workspace.tasks.iter().map(task_link).collect(),
        running_sessions: u32::try_from(running).unwrap_or(u32::MAX),
    })
}

fn task_link(task: &TaskRef) -> TaskLink {
    TaskLink {
        source: serde_json::to_value(task.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        key: task.key.clone(),
        title: task.title.clone(),
        url: task.url.clone(),
    }
}

/// One conversation, as its record and the daemon (if there is one) describe it.
///
/// The record and the daemon are matched by the PTY id the record keeps, and the daemon's session
/// must also carry this workspace's label: a terminal is never shown under a workspace it was not
/// started in.
fn session(
    row: SessionRow,
    workspace_id: &str,
    live: Option<&[SessionInfo]>,
) -> Answer<SessionSummary> {
    let terminal = live.and_then(|live| {
        let pty = row.pty_session_id.as_deref()?;
        live.iter().find(|info| {
            info.id.0 == pty
                && info.labels.get(WORKSPACE_LABEL).map(String::as_str) == Some(workspace_id)
        })
    });
    let process = match (row.running, live, terminal) {
        (false, _, _) => ProcessState::Ended,
        (true, None, _) => ProcessState::Unknown,
        (true, Some(_), Some(info)) if info.state == SessionState::Running => ProcessState::Running,
        (true, Some(_), _) => ProcessState::Gone,
    };
    Ok(SessionSummary {
        id: local_id::<SessionRecordId>(&row.id)?,
        workspace: local_id(&row.workspace_id)?,
        harness: row.harness_id,
        model: row.model,
        effort: row.effort,
        title: row.title,
        forked_from: row.forked_from.as_deref().map(local_id).transpose()?,
        started_at: row.started_at,
        ended_at: row.ended_at,
        exit_code: row.exit_code,
        process,
        terminal: terminal.map(terminal_summary).transpose()?,
    })
}

/// Activity from timing alone — whether and when it last printed — never from what it printed.
fn terminal_summary(info: &SessionInfo) -> Answer<TerminalSummary> {
    let (activity, exit_code) = match &info.state {
        SessionState::Exited { exit } => (TerminalActivity::Exited, Some(exit.code)),
        SessionState::Running if !info.has_output => (TerminalActivity::Starting, None),
        SessionState::Running if info.busy => (TerminalActivity::Busy, None),
        SessionState::Running => (TerminalActivity::Quiet, None),
    };
    Ok(TerminalSummary {
        id: local_id::<TerminalId>(&info.id.0)?,
        activity,
        idle_ms: info.idle_ms,
        size: TermSize {
            cols: info.size.cols,
            rows: info.size.rows,
        },
        exit_code,
    })
}

/// Activity cursors name an event by its sequence number, and which way to walk from it: `o`
/// for older, `n` for newer. Versioned like [`CURSOR_PREFIX`].
const ACTIVITY_CURSOR_PREFIX: &str = "a1.";

fn activity_cursor(direction: char, seq: i64) -> Answer<Cursor> {
    Cursor::new(format!("{ACTIVITY_CURSOR_PREFIX}{direction}{seq}")).map_err(|_| unreadable())
}

/// Where a cursor starts, and the event it names.
fn activity_bound(cursor: &Cursor) -> Answer<(EventBound, i64)> {
    let rest = cursor
        .as_str()
        .strip_prefix(ACTIVITY_CURSOR_PREFIX)
        .ok_or_else(cursor_expired)?;
    let seq = |digits: &str| {
        digits
            .parse::<i64>()
            .ok()
            .filter(|seq| *seq > 0)
            .ok_or_else(cursor_expired)
    };
    match rest.split_at_checked(1) {
        Some(("o", digits)) => seq(digits).map(|seq| (EventBound::Before(seq), seq)),
        Some(("n", digits)) => seq(digits).map(|seq| (EventBound::After(seq), seq)),
        _ => Err(cursor_expired()),
    }
}

fn cursor_expired() -> RemoteError {
    RemoteError::new(
        ErrorCode::CursorExpired,
        "The list changed since that page. Start again from the top.",
    )
}

/// Cursors name the last item a page held. Versioned, so a later change of scheme turns old
/// cursors into `cursor_expired` rather than into the wrong page.
const CURSOR_PREFIX: &str = "k1.";

/// One page of `items`, in their order, starting after the cursor's item.
///
/// An item that has gone since the cursor was issued leaves no way to know where the next page
/// starts, so that is `cursor_expired`: the client starts the list again rather than silently
/// missing or repeating items.
fn page<T>(
    items: Vec<T>,
    key: impl Fn(&T) -> &str,
    cursor: Option<&Cursor>,
    limit: Option<u32>,
) -> Answer<(Vec<T>, Option<Cursor>)> {
    let start = match cursor {
        None => 0,
        Some(cursor) => {
            let after = cursor
                .as_str()
                .strip_prefix(CURSOR_PREFIX)
                .ok_or_else(cursor_expired)?;
            items
                .iter()
                .position(|item| key(item) == after)
                .ok_or_else(cursor_expired)?
                + 1
        }
    };
    let size = page_size(limit);
    let more = items.len() > start + size;
    let items: Vec<T> = items.into_iter().skip(start).take(size).collect();
    let next = match items.last() {
        Some(last) if more => {
            Some(Cursor::new(format!("{CURSOR_PREFIX}{}", key(last))).map_err(|_| unreadable())?)
        }
        _ => None,
    };
    Ok((items, next))
}

mod activity;
#[cfg(test)]
mod tests;

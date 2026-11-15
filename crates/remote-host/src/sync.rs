//! Sync between an account's hosts, as this host: the loop that pushes what is queued, takes in
//! what changed, and reads a snapshot when it has to; and the person's own acts, turning a
//! project on, linking one, stopping and removing. See [42](../../../docs/design/42-remote-r6.md).
//!
//! The local side is `yardsort_core::store::sync`: every change goes into the outbox in the same
//! step as the change, and every answer and every pulled page is taken in in one step, so a crash
//! anywhere leaves nothing half done. Nothing taken in is queued again, so nothing echoes.
//!
//! **When it runs:** once Remote connects, after the person changes something, when the relay
//! says a linked project changed, and every five minutes. A pass that fails waits, longer each
//! time, up to five minutes. It never blocks anything else: while the server is away, everything
//! works here and the outbox waits. A host that syncs nothing makes no requests.
//!
//! **Nothing here decides by itself.** Turning on, linking, stopping and removing are the
//! person's, in the window or at a headless host's own terminal. `ys` changes none of it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remote_client::{Client, ClientError};
use remote_proto::account::Secret;
use remote_proto::sync::{
    PushRequest, SyncId, SyncRecord, MAX_PUSH_OPS, MAX_SYNC_BODY, SYNC_SCHEMA,
};
use remote_proto::ErrorCode;
use tokio::sync::Notify;
use yardsort_core::store::now_ms;
use yardsort_core::sync::ProjectBody;

use crate::account::{self, enabled, refused_here, Failure, Home, Result};

/// How long a quiet host waits before looking again, without being told.
pub const EVERY: Duration = Duration::from_secs(5 * 60);
/// The longest a failed pass waits before the next.
pub const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);
const FIRST_BACKOFF: Duration = Duration::from_secs(2);

/// Where the loop stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loop {
    /// Waiting for the next reason to sync.
    Idle,
    Syncing,
    /// The last pass failed; the next is in `retry_in`.
    Waiting {
        why: String,
        retry_in: Duration,
    },
}

/// The loop, while Remote is connected.
pub struct Syncer {
    wake: Arc<Notify>,
    stop: Arc<Notify>,
    state: Arc<Mutex<Loop>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Syncer {
    /// Start the loop. Its first pass is at once.
    pub fn start<H: Home>(home: Arc<H>) -> Self {
        let wake = Arc::new(Notify::new());
        let stop = Arc::new(Notify::new());
        let state = Arc::new(Mutex::new(Loop::Idle));
        let task = tokio::spawn(run(home, wake.clone(), stop.clone(), state.clone()));
        Self {
            wake,
            stop,
            state,
            task: Some(task),
        }
    }

    /// Run a pass now, or straight after the one running.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn state(&self) -> Loop {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub async fn stop(mut self) {
        self.stop.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        self.stop.notify_one();
    }
}

fn set(state: &Mutex<Loop>, to: Loop) {
    *state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = to;
}

async fn run<H: Home>(home: Arc<H>, wake: Arc<Notify>, stop: Arc<Notify>, state: Arc<Mutex<Loop>>) {
    let mut backoff = FIRST_BACKOFF;
    loop {
        let failing = matches!(
            *state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Loop::Waiting { .. }
        );
        set(&state, Loop::Syncing);
        let wait = match pass(home.as_ref()).await {
            Ok(changed) => {
                backoff = FIRST_BACKOFF;
                set(&state, Loop::Idle);
                // A pass that went through after one that failed clears the failure shown.
                if changed || failing {
                    home.synced();
                }
                EVERY
            }
            Err(failure) => {
                let _ = home
                    .store()
                    .note_sync_pass(Some(&failure.message), now_ms());
                let wait = jittered(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                set(
                    &state,
                    Loop::Waiting {
                        why: failure.message,
                        retry_in: wait,
                    },
                );
                home.synced();
                wait
            }
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = wake.notified() => {}
            () = stop.notified() => return,
        }
    }
}

/// `wait`, give or take a tenth, so hosts that failed together do not come back together.
fn jittered(wait: Duration) -> Duration {
    let spread = wait.as_millis() as u64 / 10;
    if spread == 0 {
        return wait;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    wait - Duration::from_millis(spread) + Duration::from_millis(nanos % (2 * spread))
}

/// The host's client and an access credential, from its session.
async fn signed_in(home: &impl Home) -> Result<(Client, Secret)> {
    let row = enabled(home)?;
    account::client(home, &row).await
}

/// One pass: what was removed here, then the outbox, then any project linked since the last
/// pass, then everything that changed. True when anything a person sees changed.
pub async fn pass(home: &impl Home) -> Result<bool> {
    let _turn = home.remote().sync_turn().await;
    let store = home.store();
    let synced = store.synced_projects().map_err(store_failed)?;
    if synced.is_empty() && store.sync_waiting().map_err(store_failed)? == 0 {
        return Ok(false);
    }
    let (client, access) = signed_in(home).await?;
    let mut changed = false;

    // A project removed from Yardsort here stops syncing here.
    let here: BTreeSet<String> = store
        .projects()
        .map_err(store_failed)?
        .into_iter()
        .map(|p| p.id)
        .collect();
    for gone in synced.iter().filter(|row| !here.contains(&row.local_id)) {
        unlink(&client, &access, &gone.sync_id).await?;
        store
            .forget_synced_project(&gone.local_id)
            .map_err(store_failed)?;
        changed = true;
    }

    changed |= push(home, &client, &access).await?;

    let fresh = store.fresh_synced_projects().map_err(store_failed)?;
    for project in &fresh {
        match snapshot(&client, &access, Some(project)).await {
            Ok((records, head)) => {
                changed |= store
                    .take_sync_snapshot(
                        Some(std::slice::from_ref(project)),
                        &records,
                        head,
                        now_ms(),
                    )
                    .map_err(store_failed)?;
            }
            // Unlinked or removed meanwhile.
            Err(error) if error.code() == Some(ErrorCode::NotFound) => {
                changed |= store
                    .take_sync_snapshot(Some(std::slice::from_ref(project)), &[], 0, now_ms())
                    .map_err(store_failed)?;
            }
            Err(error) => return Err(refused_here(error)),
        }
    }
    // A project just linked has this host's own notes for it to send.
    if !fresh.is_empty() {
        changed |= push(home, &client, &access).await?;
    }

    let mut after = store.sync_state().map_err(store_failed)?.cursor;
    loop {
        match client.sync_pull(&access, after).await {
            Ok(page) => {
                changed |= store
                    .take_sync_records(&page.records, Some(page.cursor))
                    .map_err(store_failed)?;
                if !page.more || page.cursor == after {
                    break;
                }
                after = page.cursor;
            }
            // Away too long, or the server lost its records: start again from a snapshot.
            Err(error) if error.code() == Some(ErrorCode::CursorExpired) => {
                let (records, head) = snapshot(&client, &access, None)
                    .await
                    .map_err(refused_here)?;
                changed |= store
                    .take_sync_snapshot(None, &records, head, now_ms())
                    .map_err(store_failed)?;
                break;
            }
            Err(error) => return Err(refused_here(error)),
        }
    }
    store.note_sync_pass(None, now_ms()).map_err(store_failed)?;
    Ok(changed)
}

/// The most pushes in one pass. An answer can queue another op, as a conflict does; a server
/// that kept answering so would otherwise hold the pass for ever.
const MAX_PUSHES: usize = 20;

/// Push the outbox, a batch at a time, each under the server's limits.
async fn push(home: &impl Home, client: &Client, access: &Secret) -> Result<bool> {
    let store = home.store();
    let mut changed = false;
    for _ in 0..MAX_PUSHES {
        let waiting = store.sync_outbox(MAX_PUSH_OPS).map_err(store_failed)?;
        let mut ops = Vec::new();
        let mut size = 64;
        for op in waiting {
            let more = serde_json::to_vec(&op).map_or(usize::MAX, |b| b.len()) + 1;
            if !ops.is_empty() && size + more > MAX_SYNC_BODY {
                break;
            }
            size += more;
            ops.push(op);
        }
        if ops.is_empty() {
            return Ok(changed);
        }
        let full = ops.len() == MAX_PUSH_OPS;
        let sent = ops.len();
        let reply = client
            .sync_push(access, &PushRequest { ops })
            .await
            .map_err(refused_here)?;
        let settled = reply.results.len();
        changed |= store
            .settle_sync(&reply.results, now_ms())
            .map_err(store_failed)?;
        // A short answer, or a batch that was all there was and queued nothing more: the rest
        // waits for the next pass.
        if settled < sent || !full && store.sync_outbox(1).map_err(store_failed)?.is_empty() {
            return Ok(changed);
        }
    }
    Ok(changed)
}

/// Every page of a snapshot, and the head the first was read at.
async fn snapshot(
    client: &Client,
    access: &Secret,
    project: Option<&SyncId>,
) -> std::result::Result<(Vec<SyncRecord>, u64), ClientError> {
    let first = client.sync_snapshot(access, project, None).await?;
    let head = first.cursor;
    let mut records = first.records;
    let mut next = first.next;
    while let Some(after) = next {
        let page = client.sync_snapshot(access, project, Some(&after)).await?;
        records.extend(page.records);
        next = page.next;
    }
    Ok((records, head))
}

/// Unlink this host from a project at the server. Not there any more is as good.
async fn unlink(client: &Client, access: &Secret, project: &SyncId) -> Result<()> {
    match client.sync_link(access, project, false).await {
        Ok(()) => Ok(()),
        Err(error) if error.code() == Some(ErrorCode::NotFound) => Ok(()),
        Err(error) => Err(refused_here(error)),
    }
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new("remote_store", error.to_string())
}

fn refused(error: yardsort_core::store::StoreError) -> Failure {
    match error {
        yardsort_core::store::StoreError::Refused(why) => Failure::new("sync_refused", why),
        other => store_failed(other),
    }
}

/// Wake the loop, when it runs.
pub fn wake(home: &impl Home) {
    if let Some(syncer) = home.remote().syncer().as_ref() {
        syncer.wake();
    }
}

/// Where the loop stands; `None` when it is not running.
pub fn state(home: &impl Home) -> Option<Loop> {
    home.remote().syncer().as_ref().map(Syncer::state)
}

/// A synced project of the account, offered for linking here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub id: SyncId,
    pub name: String,
    /// Its remote hint, as the host that turned it on sent it.
    pub remote: Option<String>,
}

/// Turn sync on for a local project: queue its record, with `hint` (`yardsort_core::sync::
/// project_hint`). The person's act, never an agent's.
pub fn turn_on(home: &impl Home, project_id: &str, hint: Option<String>) -> Result<()> {
    enabled(home)?;
    let project = home
        .store()
        .project(project_id)
        .map_err(store_failed)?
        .ok_or_else(|| Failure::new("sync_refused", "There is no such project."))?;
    let body = ProjectBody {
        name: project.name,
        remote: hint,
    };
    home.store()
        .turn_on_sync(project_id, body.to_value(), now_ms())
        .map_err(refused)?;
    wake(home);
    Ok(())
}

/// The account's synced projects this host could link: none of its projects is linked to it
/// here, and it is not removed. One the server says this host is linked to is offered too when
/// no project here is: a link whose answer was lost, or one from before Remote was turned off.
pub async fn offered(home: &impl Home) -> Result<Vec<Offer>> {
    let (client, access) = signed_in(home).await?;
    let mine: BTreeSet<SyncId> = home
        .store()
        .synced_projects()
        .map_err(store_failed)?
        .into_iter()
        .map(|row| row.sync_id)
        .collect();
    let projects = client
        .sync_projects(&access)
        .await
        .map_err(refused_here)?
        .projects;
    Ok(projects
        .into_iter()
        .filter(|p| !p.record.deleted && !mine.contains(&p.record.id))
        .filter(|p| p.record.schema <= SYNC_SCHEMA)
        .filter_map(|p| {
            let body = ProjectBody::of(&p.record)?;
            Some(Offer {
                id: p.record.id,
                name: body.name,
                remote: body.remote,
            })
        })
        .collect())
}

/// Link a local project to one of the account's synced projects, as the person chose.
pub async fn link(home: &impl Home, project_id: &str, synced: &SyncId) -> Result<()> {
    let _turn = home.remote().sync_turn().await;
    let (client, access) = signed_in(home).await?;
    let store = home.store();
    if store
        .synced_project(project_id)
        .map_err(store_failed)?
        .is_some()
    {
        return Err(Failure::new(
            "sync_refused",
            "That project syncs already. Stop syncing it first.",
        ));
    }
    if store
        .synced_projects()
        .map_err(store_failed)?
        .iter()
        .any(|row| row.sync_id == *synced)
    {
        return Err(Failure::new(
            "sync_refused",
            "Another project here is linked to that synced project already.",
        ));
    }
    let found = client
        .sync_projects(&access)
        .await
        .map_err(refused_here)?
        .projects
        .into_iter()
        .find(|p| p.record.id == *synced && !p.record.deleted)
        .ok_or_else(|| {
            Failure::new(
                "sync_refused",
                "That synced project is not on the server any more.",
            )
        })?;
    client
        .sync_link(&access, synced, true)
        .await
        .map_err(refused_here)?;
    if let Err(error) = store.link_synced_project(project_id, &found.record) {
        // Leave the server as it was: unlink only what this call linked.
        if !found.linked {
            let _ = unlink(&client, &access, synced).await;
        }
        return Err(refused(error));
    }
    wake(home);
    home.synced();
    Ok(())
}

/// Stop syncing a project here. The records stay on the server for the other hosts.
pub async fn stop(home: &impl Home, project_id: &str) -> Result<()> {
    // After any pass under way, so a create it has out settles first and is unlinked here.
    let _turn = home.remote().sync_turn().await;
    let row = home
        .store()
        .synced_project(project_id)
        .map_err(store_failed)?
        .ok_or_else(|| Failure::new("sync_refused", "That project does not sync."))?;
    let (client, access) = signed_in(home).await?;
    unlink(&client, &access, &row.sync_id).await?;
    home.store()
        .forget_synced_project(project_id)
        .map_err(store_failed)?;
    home.synced();
    Ok(())
}

/// Remove a project's records from the server, for every host. Each keeps its own copy; this
/// one stops syncing it once the server has done it.
pub fn remove(home: &impl Home, project_id: &str) -> Result<()> {
    home.store()
        .queue_sync_removal(project_id, now_ms())
        .map_err(refused)?;
    wake(home);
    home.synced();
    Ok(())
}

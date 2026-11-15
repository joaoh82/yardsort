//! Approved notes between two installations, through a server that keeps the server's rules:
//! revisions, revoke over any base, tombstones. Each test is two stores that sync as two hosts.

use std::collections::BTreeMap;

use remote_proto::sync::{Action, OpResult, RecordKind, SyncId, SyncOp, SyncRecord};
use remote_proto::{ErrorCode, HostId, RemoteError};

use super::*;
use crate::memory::{self, Decision, Settle};
use crate::store::{MemoryRow, RemoteHostRow};

/// The server's rules for records, without the network.
#[derive(Default)]
pub(super) struct Server {
    pub(super) records: BTreeMap<SyncId, SyncRecord>,
    pub(super) head: u64,
}

impl Server {
    fn push(&mut self, host: &str, ops: Vec<SyncOp>) -> Vec<OpResult> {
        ops.into_iter().map(|op| self.apply(host, op)).collect()
    }

    fn apply(&mut self, host: &str, op: SyncOp) -> OpResult {
        let current = self.records.get(&op.id).cloned();
        let conflict = |current: SyncRecord| OpResult::Conflict {
            op: op.op.clone(),
            current,
        };
        let mut next = match (op.action, current) {
            (Action::Put, None) if op.base.is_none() => SyncRecord {
                kind: op.kind,
                id: op.id.clone(),
                project: op.project.clone(),
                revision: 0,
                schema: op.schema.unwrap(),
                body: op.body.clone(),
                revoked: false,
                deleted: false,
                host: HostId::new(host).unwrap(),
                changed_at: 0,
                cursor: 0,
            },
            (_, None) => {
                return OpResult::Refused {
                    op: op.op,
                    error: RemoteError::new(ErrorCode::NotFound, "No such record."),
                }
            }
            (_, Some(current)) if current.deleted => return conflict(current),
            (Action::Delete, Some(current)) if op.base == Some(current.revision) => SyncRecord {
                body: None,
                deleted: true,
                ..current
            },
            (Action::Revoke, Some(current)) => SyncRecord {
                revoked: true,
                ..current
            },
            (Action::Put, Some(current)) if op.base == Some(current.revision) => SyncRecord {
                body: op.body.clone(),
                revoked: false,
                ..current
            },
            (_, Some(current)) => return conflict(current),
        };
        self.head += 1;
        next.revision += 1;
        next.cursor = self.head;
        next.changed_at = self.head as i64;
        next.host = HostId::new(host).unwrap();
        self.records.insert(op.id.clone(), next.clone());
        OpResult::Applied {
            op: op.op,
            revision: next.revision,
            cursor: next.cursor,
        }
    }

    fn after(&self, cursor: u64) -> Vec<SyncRecord> {
        let mut records: Vec<_> = self
            .records
            .values()
            .filter(|r| r.cursor > cursor)
            .cloned()
            .collect();
        records.sort_by_key(|r| r.cursor);
        records
    }
}

/// One installation, signed in as a host.
pub(super) struct Host {
    pub(super) store: Store,
    id: &'static str,
}

impl Host {
    pub(super) fn new(id: &'static str, name: &str) -> Self {
        let store = Store::in_memory();
        store
            .set_remote_host(&RemoteHostRow {
                server_url: "https://server.example".into(),
                server_id: "server".into(),
                host_id: id.into(),
                device_id: format!("device-{id}"),
                name: name.into(),
                enabled_at: 1,
            })
            .unwrap();
        Self { store, id }
    }

    /// Push everything waiting, as the loop does, then pull.
    pub(super) fn sync(&self, server: &mut Server) {
        self.push(server);
        self.pull(server);
    }

    fn push(&self, server: &mut Server) {
        for _ in 0..20 {
            if !self.push_once(server) {
                return;
            }
        }
        panic!("the outbox never emptied");
    }

    /// One push, as the loop makes it. False when there was nothing to send.
    fn push_once(&self, server: &mut Server) -> bool {
        let ops = self.store.sync_batch(100).unwrap();
        if ops.is_empty() {
            return false;
        }
        let results = server.push(self.id, ops);
        self.store.settle_sync(&results, 1).unwrap();
        true
    }

    fn pull(&self, server: &Server) {
        let cursor = self.store.sync_state().unwrap().cursor;
        self.store
            .take_sync_records(&server.after(cursor), Some(server.head))
            .unwrap();
    }

    /// Link `project` here to the synced one, and take its snapshot, as a link and the pass after
    /// it do.
    pub(super) fn link(&self, server: &mut Server, project: &str, synced: &SyncId) {
        let record = server.records[synced].clone();
        self.store.link_synced_project(project, &record).unwrap();
        let records: Vec<_> = server
            .records
            .values()
            .filter(|r| r.project == *synced)
            .cloned()
            .collect();
        self.store
            .take_sync_snapshot(Some(std::slice::from_ref(synced)), &records, server.head, 1)
            .unwrap();
        self.push(server);
    }

    fn note(&self, project: &str, text: &str) -> MemoryRow {
        self.notes(project)
            .into_iter()
            .find(|n| n.text == text)
            .unwrap_or_else(|| panic!("no note {text:?} in {:?}", self.notes(project)))
    }

    fn notes(&self, project: &str) -> Vec<MemoryRow> {
        self.store.memory_entries(project).unwrap()
    }
}

pub(super) fn project_body(name: &str) -> serde_json::Value {
    ProjectBody {
        name: name.into(),
        remote: None,
    }
    .to_value()
}

/// Here and there, each with the project, synced from here and linked there.
pub(super) fn linked(server: &mut Server) -> (Host, String, Host, String) {
    let here = Host::new("host-here", "Workstation");
    let there = Host::new("host-there", "Build box");
    let mine = here.store.add_project("widgets", "/w").unwrap().id;
    let theirs = there.store.add_project("widgets", "/x").unwrap().id;
    here.store
        .turn_on_sync(&mine, project_body("widgets"), 1)
        .unwrap();
    here.sync(server);
    let synced = here.store.synced_project(&mine).unwrap().unwrap().sync_id;
    there.link(server, &theirs, &synced);
    there.pull(server);
    (here, mine, there, theirs)
}

#[test]
fn an_approved_note_arrives_approved_and_cited_and_nothing_echoes() {
    let mut server = Server::default();
    let here = Host::new("host-here", "Workstation");
    let there = Host::new("host-there", "Build box");
    let mine = here.store.add_project("widgets", "/w").unwrap().id;
    let theirs = there.store.add_project("widgets", "/x").unwrap().id;
    // Written before sync was on: it goes up when it is turned on.
    memory::write(&here.store, &mine, "The tests need TZ=UTC.").unwrap();
    // Candidates and rejected notes never go up.
    let waiting = memory::propose(
        &here.store,
        &mine,
        "Run just fmt.",
        &memory::Source::default(),
    )
    .unwrap();
    let memory::Proposed::New(waiting) = waiting else {
        panic!()
    };
    let set_aside = memory::propose(
        &here.store,
        &mine,
        "Nothing works.",
        &memory::Source::default(),
    )
    .unwrap();
    let memory::Proposed::New(set_aside) = set_aside else {
        panic!()
    };
    memory::decide(&here.store, &set_aside, Decision::Reject).unwrap();

    here.store
        .turn_on_sync(&mine, project_body("widgets"), 1)
        .unwrap();
    here.sync(&mut server);
    let synced = here.store.synced_project(&mine).unwrap().unwrap().sync_id;
    there.link(&mut server, &theirs, &synced);

    let notes = there.notes(&theirs);
    assert_eq!(notes.len(), 1, "only the approved note: {notes:?}");
    let note = &notes[0];
    assert_eq!(
        (
            note.text.as_str(),
            note.state.as_str(),
            note.from_host.as_deref()
        ),
        ("The tests need TZ=UTC.", "approved", Some("Workstation"))
    );
    assert_eq!(memory::citation(note), "the user, on Workstation");
    assert!(
        there.store.sync_outbox(100).unwrap().is_empty(),
        "nothing taken in goes back up"
    );
    // Its agents get it, cited with where it is from.
    let section = memory::prompt_section(&there.store, &theirs)
        .unwrap()
        .unwrap();
    assert!(
        section.contains("- The tests need TZ=UTC. (memory ")
            && section.contains("from the user, on Workstation)"),
        "{section}"
    );
    // And the history came with it.
    assert_eq!(
        there.store.memory_history(&note.id).unwrap()[0].action,
        "written"
    );

    // Approving the candidate later sends it then.
    memory::decide(&here.store, &waiting, Decision::Approve).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    assert_eq!(there.note(&theirs, "Run just fmt.").state, "approved");
    assert!(there
        .notes(&theirs)
        .iter()
        .all(|n| n.text != "Nothing works."));
    // Here, it is this host's own: no other host named.
    assert!(here.notes(&mine).iter().all(|n| n.from_host.is_none()));
}

#[test]
fn edits_made_apart_both_stay_until_settled_once() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let first = memory::write(&here.store, &mine, "The tests need TZ=UTC.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let other = there.note(&theirs, "The tests need TZ=UTC.").id;

    // Both offline, each edits it.
    memory::edit(&here.store, &first, "The tests need TZ=UTC and LANG=C.").unwrap();
    memory::edit(&there.store, &other, "Run the tests with TZ=UTC.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    here.sync(&mut server);

    // The first to reach the server stands, on both, and the other waits beside it, on both.
    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(note.text, "The tests need TZ=UTC and LANG=C.");
        let conflict = note.conflict.expect("a conflict on both");
        assert_eq!(
            (conflict.text.as_str(), conflict.from.as_str()),
            ("Run the tests with TZ=UTC.", "Build box")
        );
        // Agents get the text that stands, not the other.
        let section = memory::prompt_section(&host.store, &note.project_id)
            .unwrap()
            .unwrap();
        assert!(section.contains("TZ=UTC and LANG=C") && !section.contains("Run the tests"));
    }
    let open = here
        .store
        .memory_entry(&first)
        .unwrap()
        .unwrap()
        .conflict
        .unwrap()
        .id;
    assert_eq!(
        there
            .store
            .memory_entry(&other)
            .unwrap()
            .unwrap()
            .conflict
            .unwrap()
            .id,
        open,
        "one conflict, the same on both"
    );

    // Both settle it at once, differently: the first settle stands, and the second yields.
    memory::settle(&there.store, &other, Settle::Other).unwrap();
    memory::settle(&here.store, &first, Settle::Standing).unwrap();
    there.sync(&mut server);
    here.sync(&mut server);
    there.sync(&mut server);
    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(
            (note.text.as_str(), note.conflict.is_none()),
            ("Run the tests with TZ=UTC.", true),
            "settled once, as the first to settle it chose"
        );
    }
    assert!(here.store.sync_outbox(100).unwrap().is_empty());
    assert!(there.store.sync_outbox(100).unwrap().is_empty());
}

#[test]
fn a_revoke_wins_over_an_edit_made_meanwhile_and_never_comes_back() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let first = memory::write(&here.store, &mine, "Deploy on Fridays.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let other = there.note(&theirs, "Deploy on Fridays.").id;

    // Revoked here while there, offline, edits it.
    memory::decide(&here.store, &first, Decision::Revoke).unwrap();
    memory::edit(&there.store, &other, "Deploy on Fridays, after noon.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    here.sync(&mut server);

    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(note.state, "revoked", "revoked on both");
        assert!(note.conflict.is_none());
        assert!(memory::prompt_section(&host.store, &note.project_id)
            .unwrap()
            .unwrap()
            .contains("Nothing is in it yet"));
    }
    // The revoke is in the history there, and the edit is not lost: it is in it too.
    let history = there.store.memory_history(&other).unwrap();
    assert!(
        history.iter().any(|line| line.action == "revoked"),
        "{history:?}"
    );
    assert!(history
        .iter()
        .any(|line| line.previous_text.as_deref() == Some("Deploy on Fridays, after noon.")));

    // Restoring is the person's own act, and goes everywhere.
    memory::decide(&there.store, &other, Decision::Restore).unwrap();
    there.sync(&mut server);
    here.sync(&mut server);
    assert_eq!(
        here.store.memory_entry(&first).unwrap().unwrap().state,
        "approved"
    );
}

#[test]
fn several_edits_made_offline_never_undo_a_revoke_made_meanwhile() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let first = memory::write(&here.store, &mine, "Deploy on Fridays.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let other = there.note(&theirs, "Deploy on Fridays.").id;

    // There, offline, edits it twice; here revokes it.
    memory::edit(&there.store, &other, "Deploy on Fridays, after noon.").unwrap();
    memory::edit(&there.store, &other, "Deploy on Fridays, after one.").unwrap();
    memory::decide(&here.store, &first, Decision::Revoke).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    here.sync(&mut server);

    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(note.state, "revoked", "revoked on both: {note:?}");
    }
    let record = server
        .records
        .values()
        .find(|r| r.kind == RecordKind::Note)
        .unwrap();
    assert!(record.revoked, "and on the server");
}

#[test]
fn several_edits_made_apart_each_meet_the_conflict_not_each_other() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let first = memory::write(&here.store, &mine, "Use bun.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let other = there.note(&theirs, "Use bun.").id;

    memory::edit(&here.store, &first, "Use bun 1.3.").unwrap();
    memory::edit(&there.store, &other, "Use bun, not npm.").unwrap();
    memory::edit(&there.store, &other, "Use bun, never npm.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    here.sync(&mut server);

    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(
            note.text, "Use bun 1.3.",
            "the first to reach the server stands"
        );
        assert_eq!(
            note.conflict.map(|c| c.text).as_deref(),
            Some("Use bun, never npm."),
            "and the other's latest waits beside it"
        );
    }
}

#[test]
fn a_conflict_that_meets_a_newer_text_on_its_way_up_keeps_this_hosts_text() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let first = memory::write(&here.store, &mine, "Use bun.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let other = there.note(&theirs, "Use bun.").id;

    memory::edit(&here.store, &first, "Use bun 1.3.").unwrap();
    memory::edit(&there.store, &other, "Use bun, not npm.").unwrap();
    here.sync(&mut server);
    // There's edit meets here's: it takes here's text and queues its own beside it...
    assert!(there.push_once(&mut server));
    assert_eq!(
        there.store.memory_entry(&other).unwrap().unwrap().text,
        "Use bun 1.3."
    );
    // ...and before that goes up, here edits again.
    memory::edit(&here.store, &first, "Use bun 1.4.").unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    here.sync(&mut server);

    for (host, id) in [(&here, &first), (&there, &other)] {
        let note = host.store.memory_entry(id).unwrap().unwrap();
        assert_eq!(note.text, "Use bun 1.4.");
        assert_eq!(
            note.conflict.map(|c| c.text).as_deref(),
            Some("Use bun, not npm."),
            "there's text still waits, on both"
        );
    }
}

#[test]
fn linking_takes_the_same_note_as_one_and_sends_the_rest() {
    let mut server = Server::default();
    let here = Host::new("host-here", "Workstation");
    let there = Host::new("host-there", "Build box");
    let mine = here.store.add_project("widgets", "/w").unwrap().id;
    let theirs = there.store.add_project("widgets", "/x").unwrap().id;
    memory::write(&here.store, &mine, "The tests need TZ=UTC.").unwrap();
    memory::write(&there.store, &theirs, "the tests need tz=utc.").unwrap();
    memory::write(&there.store, &theirs, "Never edit bindings.ts.").unwrap();
    here.store
        .turn_on_sync(&mine, project_body("widgets"), 1)
        .unwrap();
    here.sync(&mut server);
    let synced = here.store.synced_project(&mine).unwrap().unwrap().sync_id;

    there.link(&mut server, &theirs, &synced);
    there.sync(&mut server);
    here.sync(&mut server);

    let texts = |host: &Host, project: &str| {
        let mut texts: Vec<_> = host
            .notes(project)
            .into_iter()
            .map(|n| n.text.to_lowercase())
            .collect();
        texts.sort();
        texts
    };
    assert_eq!(
        texts(&here, &mine),
        ["never edit bindings.ts.", "the tests need tz=utc."]
    );
    assert_eq!(texts(&there, &theirs), texts(&here, &mine));
    assert_eq!(
        here.note(&mine, "Never edit bindings.ts.")
            .from_host
            .as_deref(),
        Some("Build box")
    );
}

#[test]
fn a_project_that_does_not_sync_sends_no_notes() {
    let mut server = Server::default();
    let here = Host::new("host-here", "Workstation");
    let mine = here.store.add_project("widgets", "/w").unwrap().id;
    let other = here.store.add_project("gadgets", "/g").unwrap().id;
    here.store
        .turn_on_sync(&mine, project_body("widgets"), 1)
        .unwrap();
    here.sync(&mut server);
    memory::write(&here.store, &other, "Gadgets are private.").unwrap();
    assert!(here.store.sync_outbox(100).unwrap().is_empty());
    assert!(server.records.values().all(|r| r.kind != RecordKind::Note));
}

/// Every `Store` method that writes a note, and how it reaches the outbox. A method added
/// without a line here fails this test: it would change a synced note without telling the
/// other hosts.
#[test]
fn every_write_to_notes_queues_its_op_or_says_why_not() {
    const KNOWN: &[(&str, &str)] = &[
        ("add_memory", "queues, in its transaction"),
        ("change_memory", "queues, in its transaction"),
        ("propose_memory", "a candidate, which never syncs"),
    ];
    let source = include_str!("../store.rs");
    let mut writers = Vec::new();
    let mut current = None;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("pub fn ")
            .or_else(|| trimmed.strip_prefix("fn "))
        {
            current = rest.split(['(', '<']).next().map(str::to_owned);
        }
        let writes = [
            "INSERT INTO memory_entries",
            "UPDATE memory_entries",
            "DELETE FROM memory_entries",
        ]
        .iter()
        .any(|sql| line.contains(sql));
        if let (true, Some(name)) = (writes, current.as_ref()) {
            if !writers.contains(name) {
                writers.push(name.clone());
            }
        }
    }
    writers.sort();
    let mut known: Vec<_> = KNOWN.iter().map(|(name, _)| name.to_string()).collect();
    known.sort();
    assert_eq!(writers, known);
    assert!(
        include_str!("../store.rs")
            .matches("sync::queue_note(&tx")
            .count()
            >= 2
    );
}

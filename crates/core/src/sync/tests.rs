use remote_proto::sync::{OpResult, SyncRecord};
use remote_proto::{ErrorCode, HostId, RemoteError};

use super::*;
use crate::store::RemoteHostRow;

fn body(name: &str) -> serde_json::Value {
    ProjectBody {
        name: name.into(),
        remote: Some("https://github.com/acme/widgets.git".into()),
    }
    .to_value()
}

fn record(op: &remote_proto::sync::SyncOp, revision: u64, deleted: bool) -> SyncRecord {
    SyncRecord {
        kind: op.kind,
        id: op.id.clone(),
        project: op.project.clone(),
        revision,
        schema: 1,
        body: (!deleted).then(|| op.body.clone()).flatten(),
        revoked: false,
        deleted,
        host: HostId::new("host-there").unwrap(),
        changed_at: 1,
        cursor: revision,
    }
}

fn applied(op: &remote_proto::sync::SyncOp, revision: u64) -> OpResult {
    OpResult::Applied {
        op: op.op.clone(),
        revision,
        cursor: revision,
    }
}

fn host_row(server: &str) -> RemoteHostRow {
    RemoteHostRow {
        server_url: format!("https://{server}.example"),
        server_id: server.into(),
        host_id: format!("host-{server}"),
        device_id: "device".into(),
        name: "Workstation".into(),
        enabled_at: 1,
    }
}

#[test]
fn a_hint_keeps_the_address_and_drops_who_signed_in() {
    for (url, hint) in [
        (
            "https://ana:hunter2@github.com/acme/widgets.git",
            Some("https://github.com/acme/widgets.git"),
        ),
        (
            "https://token@gitlab.example:8443/acme/widgets",
            Some("https://gitlab.example:8443/acme/widgets"),
        ),
        (
            "ssh://git@github.com/acme/widgets.git",
            Some("ssh://github.com/acme/widgets.git"),
        ),
        (
            "git@github.com:acme/widgets.git",
            Some("github.com:acme/widgets.git"),
        ),
        ("github.com:acme/widgets", Some("github.com:acme/widgets")),
        // On this disk: nobody else's business, and no use elsewhere.
        ("/home/ana/src/widgets.git", None),
        ("file:///home/ana/src/widgets.git", None),
        ("../widgets", None),
        ("./a:b", None),
        ("C:/src/widgets", None),
        ("C:\\src\\widgets", None),
    ] {
        assert_eq!(remote_hint(url).as_deref(), hint, "{url}");
    }
}

#[test]
fn hints_match_whatever_the_scheme() {
    assert!(same_remote(
        "https://github.com/Acme/widgets.git",
        "github.com:acme/widgets"
    ));
    assert!(same_remote(
        "ssh://github.com:22/acme/widgets/",
        "https://github.com/acme/widgets"
    ));
    assert!(!same_remote(
        "https://github.com/acme/widgets",
        "https://github.com/acme/gadgets"
    ));
}

#[test]
fn turning_sync_on_queues_the_project_and_its_answer_settles_it() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    let row = store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    assert_eq!(row.sync_id, row.project);

    let waiting = store.sync_outbox(100).unwrap();
    assert_eq!(waiting.len(), 1);
    let create = &waiting[0];
    assert!(create.check().is_ok());
    assert_eq!(
        (create.action, create.kind, create.base, &create.id),
        (Action::Put, RecordKind::Project, None, &row.sync_id)
    );
    assert_eq!(create.body.as_ref().unwrap()["name"], "widgets");
    // Once is enough.
    assert!(store.turn_on_sync(&project.id, body("widgets"), 1).is_err());
    assert_eq!(
        project_sync(&store, &project.id, true, || unreachable!()).unwrap(),
        ProjectSync::On {
            reached: false,
            removing: false,
            refused: None
        }
    );

    // Reaching the server is a change the person sees.
    assert!(store.settle_sync(&[applied(create, 1)], 1).unwrap());
    assert!(store.sync_outbox(100).unwrap().is_empty());
    assert_eq!(
        store.synced_project(&project.id).unwrap().unwrap().revision,
        Some(1)
    );
    // The same answer again, as after a retried push, changes nothing.
    assert!(!store.settle_sync(&[applied(create, 1)], 1).unwrap());
    assert_eq!(store.sync_waiting().unwrap(), 0);
}

#[test]
fn a_create_answered_with_a_conflict_is_there_already() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    let create = store.sync_outbox(100).unwrap().remove(0);
    store
        .settle_sync(
            &[OpResult::Conflict {
                op: create.op.clone(),
                current: record(&create, 1, false),
            }],
            1,
        )
        .unwrap();
    assert!(store.sync_outbox(100).unwrap().is_empty());
    assert_eq!(
        store.synced_project(&project.id).unwrap().unwrap().revision,
        Some(1)
    );
}

#[test]
fn a_refused_op_is_held_and_shown_never_retried() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    let create = store.sync_outbox(100).unwrap().remove(0);
    let changed = store
        .settle_sync(
            &[OpResult::Refused {
                op: create.op.clone(),
                error: RemoteError::new(ErrorCode::InvalidRequest, "That record is too large."),
            }],
            1,
        )
        .unwrap();
    assert!(changed);
    assert!(store.sync_outbox(100).unwrap().is_empty(), "not retried");
    assert_eq!(store.sync_waiting().unwrap(), 1, "but kept");
    assert_eq!(
        project_sync(&store, &project.id, true, || unreachable!()).unwrap(),
        ProjectSync::On {
            reached: false,
            removing: false,
            refused: Some("That record is too large.".into())
        }
    );
    // Stopping clears it.
    assert!(store.forget_synced_project(&project.id).unwrap());
    assert_eq!(store.sync_waiting().unwrap(), 0);
}

#[test]
fn removing_from_the_server_stands_over_a_newer_revision_and_ends_syncing_here() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    // Not on the server yet: nothing to remove.
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    assert!(store.queue_sync_removal(&project.id, 2).is_err());
    let create = store.sync_outbox(100).unwrap().remove(0);
    store.settle_sync(&[applied(&create, 1)], 1).unwrap();

    store.queue_sync_removal(&project.id, 2).unwrap();
    store.queue_sync_removal(&project.id, 3).unwrap();
    let removal = store.sync_outbox(100).unwrap();
    assert_eq!(removal.len(), 1, "queued once");
    let removal = removal[0].clone();
    assert_eq!((removal.action, removal.base), (Action::Delete, Some(1)));
    assert!(matches!(
        project_sync(&store, &project.id, true, || unreachable!()).unwrap(),
        ProjectSync::On { removing: true, .. }
    ));

    // Another host changed it meanwhile: the removal goes again against that revision.
    let mut newer = record(&create, 2, false);
    newer.body = Some(body("widgets renamed"));
    store
        .settle_sync(
            &[OpResult::Conflict {
                op: removal.op.clone(),
                current: newer,
            }],
            1,
        )
        .unwrap();
    let again = store.sync_outbox(100).unwrap().remove(0);
    assert_eq!(
        (again.op.clone(), again.base),
        (removal.op.clone(), Some(2))
    );

    assert!(store.settle_sync(&[applied(&again, 3)], 1).unwrap());
    assert!(store.synced_project(&project.id).unwrap().is_none());
    assert_eq!(store.sync_waiting().unwrap(), 0);
    assert_eq!(store.projects().unwrap().len(), 1, "the project stays here");
}

#[test]
fn a_project_removed_elsewhere_stops_syncing_here() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    let create = store.sync_outbox(100).unwrap().remove(0);
    store.settle_sync(&[applied(&create, 1)], 1).unwrap();

    // A newer revision is only noted.
    assert!(!store
        .take_sync_records(&[record(&create, 2, false)], Some(5))
        .unwrap());
    assert_eq!(store.sync_state().unwrap().cursor, 5);
    assert_eq!(
        store.synced_project(&project.id).unwrap().unwrap().revision,
        Some(2)
    );
    // A record of something this host does not sync is left alone.
    let other = remote_proto::sync::SyncOp {
        id: remote_proto::sync::SyncId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        ..create.clone()
    };
    let other = remote_proto::sync::SyncOp {
        project: other.id.clone(),
        ..other
    };
    assert!(!store
        .take_sync_records(&[record(&other, 1, false)], None)
        .unwrap());

    assert!(store
        .take_sync_records(&[record(&create, 3, true)], Some(6))
        .unwrap());
    assert!(store.synced_project(&project.id).unwrap().is_none());
    assert_eq!(store.projects().unwrap().len(), 1);
}

#[test]
fn linking_is_once_each_way_and_a_snapshot_reads_it() {
    let store = Store::in_memory();
    let here = store.add_project("widgets", "/w").unwrap();
    let also = store.add_project("widgets-2", "/w2").unwrap();
    let elsewhere = Store::in_memory();
    let theirs = elsewhere.add_project("widgets", "/x").unwrap();
    elsewhere
        .turn_on_sync(&theirs.id, body("widgets"), 1)
        .unwrap();
    let create = elsewhere.sync_outbox(100).unwrap().remove(0);
    let synced = record(&create, 1, false);

    store.link_synced_project(&here.id, &synced).unwrap();
    assert!(store.link_synced_project(&here.id, &synced).is_err());
    assert!(
        store.link_synced_project(&also.id, &synced).is_err(),
        "one local project for each synced one"
    );
    assert_eq!(
        store.fresh_synced_projects().unwrap(),
        std::slice::from_ref(&synced.id)
    );
    assert!(
        store.sync_outbox(100).unwrap().is_empty(),
        "linking sends nothing"
    );
    assert_eq!(
        project_sync(&store, &here.id, true, || unreachable!()).unwrap(),
        ProjectSync::On {
            reached: true,
            removing: false,
            refused: None
        }
    );

    store
        .take_sync_snapshot(
            Some(std::slice::from_ref(&synced.id)),
            std::slice::from_ref(&synced),
            9,
            1,
        )
        .unwrap();
    assert!(store.fresh_synced_projects().unwrap().is_empty());
    assert_eq!(
        store.sync_state().unwrap().cursor,
        0,
        "one project's snapshot"
    );

    // A full snapshot without it: it is gone from the server.
    assert!(store.take_sync_snapshot(None, &[], 12, 1).unwrap());
    assert!(store.synced_project(&here.id).unwrap().is_none());
    assert_eq!(store.sync_state().unwrap().cursor, 12);
}

#[test]
fn a_snapshot_keeps_a_project_still_on_its_way_up() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    store.take_sync_snapshot(None, &[], 4, 1).unwrap();
    assert!(store.synced_project(&project.id).unwrap().is_some());
    assert_eq!(store.sync_waiting().unwrap(), 1);
}

#[test]
fn turning_remote_off_or_moving_servers_forgets_sync() {
    let store = Store::in_memory();
    let project = store.add_project("widgets", "/w").unwrap();
    store.set_remote_host(&host_row("one")).unwrap();
    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    store.note_sync_pass(None, 5).unwrap();
    // The same host again keeps it.
    store.set_remote_host(&host_row("one")).unwrap();
    assert!(store.synced_project(&project.id).unwrap().is_some());

    store.set_remote_host(&host_row("two")).unwrap();
    assert!(store.synced_project(&project.id).unwrap().is_none());
    assert_eq!(store.sync_waiting().unwrap(), 0);
    assert_eq!(store.sync_state().unwrap(), Default::default());

    store.turn_on_sync(&project.id, body("widgets"), 1).unwrap();
    store.forget_remote_host().unwrap();
    assert!(store.synced_project(&project.id).unwrap().is_none());
    assert_eq!(
        project_sync(&store, &project.id, false, || unreachable!()).unwrap(),
        ProjectSync::RemoteOff
    );
}

#[test]
fn a_pass_keeps_the_last_good_time_through_a_problem() {
    let store = Store::in_memory();
    store.note_sync_pass(None, 5).unwrap();
    store
        .note_sync_pass(Some("The server cannot be reached."), 9)
        .unwrap();
    let state = store.sync_state().unwrap();
    assert_eq!(
        (state.synced_at, state.problem.as_deref()),
        (Some(5), Some("The server cannot be reached."))
    );
    store.note_sync_pass(None, 12).unwrap();
    assert_eq!(store.sync_state().unwrap().problem, None);
}

//! Workspaces between two installations, through the same stand-in as the notes'. A workspace's
//! record is written only by the host it lives on; the other shows it and makes nothing for it.

use super::note_tests::{linked, project_body, Host, Server};
use crate::store::WorkspaceTaskRow;

fn task() -> WorkspaceTaskRow {
    WorkspaceTaskRow {
        source: "github".into(),
        repo: "github.com/acme/widgets".into(),
        key: "#91".into(),
        url: "https://github.com/acme/widgets/issues/91".into(),
        title: "Login fails on Safari".into(),
    }
}

#[test]
fn a_workspace_made_renamed_and_archived_is_seen_elsewhere_and_never_made_there() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let made = here
        .store
        .add_worktree(
            &mine,
            "fix-login",
            "/wt/fix-login",
            Some("ys/fix-login"),
            Some("main"),
        )
        .unwrap();
    here.store.link_task(&made.id, &task()).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);

    let seen = there.store.synced_workspaces(&theirs).unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let one = &seen[0];
    assert_eq!(
        (
            one.host.as_str(),
            one.name.as_str(),
            one.branch.as_deref(),
            one.base_branch.as_deref(),
            one.archived
        ),
        (
            "Workstation",
            "fix-login",
            Some("ys/fix-login"),
            Some("main"),
            false
        )
    );
    assert_eq!(one.tasks[0].key, "#91");
    // Nothing is made there: no workspace row, so no worktree, branch or directory.
    let rows: Vec<_> = there
        .store
        .workspaces()
        .unwrap()
        .into_iter()
        .filter(|w| w.project_id == theirs && w.kind == "worktree")
        .collect();
    assert!(rows.is_empty(), "{rows:?}");
    assert!(
        there.store.sync_outbox(100).unwrap().is_empty(),
        "nothing echoes"
    );
    // Here, its own workspace is not shown as another host's.
    assert!(here.store.synced_workspaces(&mine).unwrap().is_empty());

    here.store
        .rename_workspace(&made.id, "login-safari")
        .unwrap();
    here.store.set_workspace_archived(&made.id, true).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let one = &there.store.synced_workspaces(&theirs).unwrap()[0];
    assert_eq!((one.name.as_str(), one.archived), ("login-safari", true));

    // Deleted here: gone there.
    here.store.remove_worktree(&made.id).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    assert!(there.store.synced_workspaces(&theirs).unwrap().is_empty());
    assert!(here.store.sync_outbox(100).unwrap().is_empty());
}

#[test]
fn the_projects_own_checkout_and_a_forgotten_workspace_are_not_sent() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let made = here
        .store
        .add_worktree(&mine, "spike", "/wt/spike", Some("ys/spike"), None)
        .unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    assert_eq!(there.store.synced_workspaces(&theirs).unwrap().len(), 1);

    here.store.forget_workspace(&made.id, true).unwrap();
    // Brought back before the forgetting went up: a new record, and the old one goes.
    here.store
        .revive_workspace(&made.id, Some("ys/spike"))
        .unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    let seen = there.store.synced_workspaces(&theirs).unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].name, "spike");

    here.store.forget_workspace(&made.id, true).unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    assert!(there.store.synced_workspaces(&theirs).unwrap().is_empty());
    let kinds: Vec<_> = server
        .records
        .values()
        .filter(|r| !r.deleted)
        .map(|r| r.kind)
        .collect();
    assert_eq!(
        kinds,
        [remote_proto::sync::RecordKind::Project],
        "only the project is left"
    );
}

#[test]
fn turning_on_and_linking_send_each_hosts_workspaces_and_stopping_hides_the_others() {
    let mut server = Server::default();
    let here = Host::new("host-here", "Workstation");
    let there = Host::new("host-there", "Build box");
    let mine = here.store.add_project("widgets", "/w").unwrap().id;
    let theirs = there.store.add_project("widgets", "/x").unwrap().id;
    here.store
        .add_worktree(&mine, "a", "/w/a", Some("ys/a"), None)
        .unwrap();
    there
        .store
        .add_worktree(&theirs, "b", "/x/b", Some("ys/b"), None)
        .unwrap();
    here.store
        .turn_on_sync(&mine, project_body("widgets"), 1)
        .unwrap();
    here.sync(&mut server);
    let synced = here.store.synced_project(&mine).unwrap().unwrap().sync_id;
    there.link(&mut server, &theirs, &synced);
    there.sync(&mut server);
    here.sync(&mut server);

    let names = |host: &Host, project: &str| -> Vec<(String, String)> {
        host.store
            .synced_workspaces(project)
            .unwrap()
            .into_iter()
            .map(|w| (w.host, w.name))
            .collect()
    };
    assert_eq!(names(&there, &theirs), [("Workstation".into(), "a".into())]);
    assert_eq!(names(&here, &mine), [("Build box".into(), "b".into())]);

    there.store.forget_synced_project(&theirs).unwrap();
    assert!(there.store.synced_workspaces(&theirs).unwrap().is_empty());
}

#[test]
fn a_record_of_this_hosts_that_it_no_longer_knows_is_deleted_not_shown() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    here.store
        .add_worktree(&mine, "old", "/w/old", Some("ys/old"), None)
        .unwrap();
    here.sync(&mut server);
    // Stopped and linked again: its old record is on the server, and no longer linked here.
    let synced = here.store.synced_project(&mine).unwrap().unwrap().sync_id;
    here.store.forget_synced_project(&mine).unwrap();
    here.link(&mut server, &mine, &synced);
    here.sync(&mut server);
    there.sync(&mut server);

    assert!(here.store.synced_workspaces(&mine).unwrap().is_empty());
    let seen = there.store.synced_workspaces(&theirs).unwrap();
    assert_eq!(
        seen.len(),
        1,
        "the old record went, the new one stands: {seen:?}"
    );
}

#[test]
fn what_an_earlier_version_left_unsent_goes_up_once() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    // As slice 3 left a profile: the project syncs, and a workspace and an approved note it had
    // before have no record. Written here past the store, as that version did not queue them.
    {
        let conn = here.store.conn();
        conn.execute_batch(&format!(
            "INSERT INTO workspaces (id, project_id, kind, name, path, branch, created_at)
             VALUES ('old-ws', '{mine}', 'worktree', 'old-work', '/w/old', 'ys/old', 1);
             INSERT INTO memory_entries (id, project_id, text, state, author, created_at, updated_at)
             VALUES ('old-note', '{mine}', 'Run just check.', 'approved', 'user', 1, 1);"
        ))
        .unwrap();
    }
    assert!(here.store.sync_outbox(100).unwrap().is_empty());

    here.store.backfill_sync(1).unwrap();
    assert_eq!(here.store.sync_outbox(100).unwrap().len(), 2);
    here.store.backfill_sync(1).unwrap();
    assert_eq!(here.store.sync_outbox(100).unwrap().len(), 2, "queued once");
    here.sync(&mut server);
    there.sync(&mut server);
    assert_eq!(
        there.store.synced_workspaces(&theirs).unwrap()[0].name,
        "old-work"
    );
    assert!(there
        .store
        .memory_entries(&theirs)
        .unwrap()
        .iter()
        .any(|n| n.text == "Run just check."));
    here.store.backfill_sync(1).unwrap();
    assert!(
        here.store.sync_outbox(100).unwrap().is_empty(),
        "nothing more to send"
    );
}

#[test]
fn a_full_snapshot_drops_a_workspace_whose_tombstone_was_pruned() {
    let mut server = Server::default();
    let (here, mine, there, theirs) = linked(&mut server);
    let made = here
        .store
        .add_worktree(&mine, "gone-soon", "/w/gone", Some("ys/gone"), None)
        .unwrap();
    here.sync(&mut server);
    there.sync(&mut server);
    assert_eq!(there.store.synced_workspaces(&theirs).unwrap().len(), 1);
    here.store.remove_worktree(&made.id).unwrap();
    here.sync(&mut server);

    // There was away past retention: the tombstone is pruned, and its cursor expired.
    let live: Vec<_> = server
        .records
        .values()
        .filter(|r| !r.deleted)
        .cloned()
        .collect();
    there
        .store
        .take_sync_snapshot(None, &live, server.head, 1)
        .unwrap();
    assert!(there.store.synced_workspaces(&theirs).unwrap().is_empty());
}

/// Every `Store` method that writes a workspace, and how it reaches the outbox. A method added
/// without a line here fails this test.
#[test]
fn every_write_to_workspaces_queues_its_op_or_says_why_not() {
    const KNOWN: &[(&str, &str)] = &[
        (
            "add_project",
            "the project's own checkout, which never syncs",
        ),
        ("add_worktree", "queues, in its transaction"),
        ("link_task", "queues, in its transaction"),
        ("forget_workspace", "through change_workspace, which queues"),
        ("remove_worktree", "through change_workspace, which queues"),
        ("rename_workspace", "through change_workspace, which queues"),
        ("revive_workspace", "through change_workspace, which queues"),
        (
            "set_workspace_archived",
            "through change_workspace, which queues",
        ),
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
        // The store's own tests write rows as an old version did.
        if line.starts_with("mod tests") {
            break;
        }
        let writes = [
            "INSERT INTO workspaces",
            "UPDATE workspaces",
            "DELETE FROM workspaces",
            "INTO workspace_tasks",
            "DELETE FROM workspace_tasks",
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
    assert!(source.contains("let changed = tx.execute(sql, params)? > 0;\n        if changed {\n            sync::queue_workspace(&tx, id, now_ms())?;"));
}

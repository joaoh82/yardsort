use std::path::Path;

use pty_host::{HostEvent, SessionId};
use serde_json::json;

use super::*;
use crate::activity::{PRIVACY_METADATA, SCHEMA_VERSION};
use crate::store::{now_ms, NewEvent, NewRun, NewSession};
use crate::trigger::turns::Change;

/// A project with one workspace, in a store of its own.
struct Fixture {
    store: Store,
    workspace: String,
    events: std::cell::Cell<u32>,
}

impl Fixture {
    fn new() -> Self {
        Self::at("/code/weather-cli-fix")
    }

    fn at(path: &str) -> Self {
        let store = Store::in_memory();
        let project = store
            .add_project("weather-cli", "/code/weather-cli")
            .unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix-timezone-offsets",
                path,
                Some("ys/fix-timezone-offsets"),
                Some("main"),
            )
            .unwrap();
        Self {
            store,
            workspace: workspace.id,
            events: std::cell::Cell::new(0),
        }
    }

    /// An agent's conversation `record`, running in `terminal`, with its run.
    fn agent(&self, record: &str, terminal: &str) {
        self.agent_of(record, terminal, None);
    }

    fn agent_of(&self, record: &str, terminal: &str, harness_session: Option<&str>) {
        self.store
            .add_session(&NewSession {
                id: record,
                workspace_id: &self.workspace,
                harness_id: "claude",
                model: None,
                effort: None,
                harness_session_id: harness_session,
                title: "Cover half-hour zones",
                forked_from: None,
                pty_session_id: terminal,
                prompt: None,
            })
            .unwrap();
        let run = format!("run-{record}");
        self.store
            .add_run(&NewRun {
                id: &run,
                workspace_id: &self.workspace,
                session_id: Some(record),
                kind: "harness",
                harness_id: Some("claude"),
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();
        self.store.run_spawned(&run, terminal).unwrap();
    }

    /// One of the agent's hook events, as the inbox would have brought it.
    fn event(&self, record: &str, kind: &str, at: i64, payload: serde_json::Value) {
        let n = self.events.get() + 1;
        self.events.set(n);
        let run = format!("run-{record}");
        let added = self
            .store
            .add_event(&NewEvent {
                id: &format!("event-{n}"),
                schema_version: SCHEMA_VERSION,
                workspace_id: &self.workspace,
                session_id: Some(record),
                run_id: Some(&run),
                occurred_at: at,
                kind,
                producer: "claude",
                method: "hook",
                fidelity: "exact",
                source_key: None,
                privacy_class: PRIVACY_METADATA,
                payload: &payload.to_string(),
            })
            .unwrap();
        assert!(added);
    }

    /// A list that has had its first, quiet look: what comes after is news.
    fn listening(&self) -> Turns {
        let turns = Turns::default();
        assert!(!read_events(&self.store, &turns).unwrap());
        turns
    }

    fn list(&self, now: i64) -> Vec<Notification> {
        list(&self.store, now).unwrap()
    }

    fn parts(&self, now: i64) -> Vec<(NotificationKind, NotificationPart)> {
        self.list(now).iter().map(|n| (n.kind, n.part)).collect()
    }
}

fn busy(terminal: &str) -> HostEvent {
    HostEvent::Busy {
        id: SessionId(terminal.to_owned()),
    }
}

fn quiet(terminal: &str, busy_ms: u32) -> HostEvent {
    HostEvent::Quiet {
        id: SessionId(terminal.to_owned()),
        busy_ms,
    }
}

#[test]
fn a_permission_is_one_row_until_it_is_answered() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    let turns = f.listening();
    let t = now_ms();

    f.event(
        "s1",
        "approval.requested",
        t,
        json!({"toolUseId": "call-1"}),
    );
    assert!(read_events(&f.store, &turns).unwrap());
    let rows = f.list(t + 1);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(
        (row.kind, row.part),
        (NotificationKind::AgentAsks, NotificationPart::Needs)
    );
    assert_eq!(row.project_name, "weather-cli");
    assert_eq!(row.workspace_name, "fix-timezone-offsets");
    assert_eq!(row.branch.as_deref(), Some("ys/fix-timezone-offsets"));
    assert_eq!(row.session_id.as_deref(), Some("s1"));
    assert_eq!(row.session_title, "Cover half-hour zones");
    assert_eq!(row.harness_id.as_deref(), Some("claude"));
    assert_eq!(row.terminal_id.as_deref(), Some("pty-1"));
    assert_eq!(row.at, t);

    // A second request while the first waits is the same wait.
    f.event(
        "s1",
        "approval.requested",
        t + 10,
        json!({"toolUseId": "call-2"}),
    );
    assert!(!read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.parts(t + 11),
        [(NotificationKind::AgentAsks, NotificationPart::Needs)]
    );

    // Another tool finishing beside it is not an answer.
    f.event(
        "s1",
        "tool.completed",
        t + 20,
        json!({"toolUseId": "call-9"}),
    );
    assert!(!read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.parts(t + 21),
        [(NotificationKind::AgentAsks, NotificationPart::Needs)]
    );

    // The tool it asked about running is.
    f.event(
        "s1",
        "tool.completed",
        t + 30,
        json!({"toolUseId": "call-1"}),
    );
    assert!(read_events(&f.store, &turns).unwrap());
    let rows = f.list(t + 31);
    assert_eq!(rows[0].part, NotificationPart::Earlier);
    assert_eq!(rows[0].settled_at, Some(t + 30));

    // The next request is a new wait.
    f.event(
        "s1",
        "approval.requested",
        t + 40,
        json!({"toolUseId": "call-3"}),
    );
    assert!(read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.parts(t + 41),
        [
            (NotificationKind::AgentAsks, NotificationPart::Needs),
            (NotificationKind::AgentAsks, NotificationPart::Earlier)
        ]
    );
}

#[test]
fn what_happened_before_the_first_look_is_not_news() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    let t = now_ms();
    f.event("s1", "approval.requested", t, json!({}));
    f.event("s1", "turn.completed", t + 1, json!({}));
    let turns = Turns::default();
    assert!(!read_events(&f.store, &turns).unwrap());
    assert!(!read_events(&f.store, &turns).unwrap());
    assert!(f.list(t + 2).is_empty());
}

#[test]
fn a_quiet_terminal_and_a_reported_turn_are_one_finished_row() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    let turns = f.listening();
    let t = now_ms();

    // The terminal goes quiet after nine seconds of work: the turn's first word.
    turns.observe(&busy("pty-1"), t);
    assert_eq!(
        turns.observe(&quiet("pty-1", 9_000), t + 10_000),
        Some(Change::Settled {
            terminal: "pty-1".into()
        })
    );
    assert!(settled_terminal(&f.store, "pty-1", 9_000, t + 10_000).unwrap());
    let rows = f.list(t + 10_001);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].kind, rows[0].part),
        (NotificationKind::AgentFinished, NotificationPart::Finished)
    );
    assert_eq!(rows[0].worked_ms, Some(9_000));

    // Its harness then reports the same turn: no second row.
    f.event("s1", "turn.completed", t + 10_050, json!({}));
    assert!(!read_events(&f.store, &turns).unwrap());
    assert_eq!(f.list(t + 10_100).len(), 1);

    // Someone looks at the terminal.
    assert_eq!(seen_terminal(&f.store, "pty-1", t + 11_000).unwrap(), 1);
    assert_eq!(
        f.parts(t + 11_001),
        [(NotificationKind::AgentFinished, NotificationPart::Earlier)]
    );
    assert_eq!(seen_terminal(&f.store, "pty-1", t + 11_002).unwrap(), 0);

    // The next turn, told by the harness first this time, is a new row. It worked since the
    // message that started it.
    turns.observe(&busy("pty-1"), t + 12_000);
    f.event("s1", "prompt.submitted", t + 12_000, json!({}));
    f.event("s1", "turn.completed", t + 20_000, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    let rows = f.list(t + 20_001);
    assert_eq!(
        rows.iter().map(|n| n.part).collect::<Vec<_>>(),
        [NotificationPart::Finished, NotificationPart::Earlier]
    );
    assert_eq!(rows[0].worked_ms, Some(8_000));

    // Finishing again before anyone looked keeps the one unseen row, at the later time.
    turns.observe(&busy("pty-1"), t + 21_000);
    f.event("s1", "turn.completed", t + 25_000, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    let rows = f.list(t + 25_001);
    assert_eq!(
        rows.iter().map(|n| n.part).collect::<Vec<_>>(),
        [NotificationPart::Finished, NotificationPart::Earlier]
    );
    assert_eq!(rows[0].at, t + 25_000);

    // Typing to the agent is seeing where it stood.
    f.event("s1", "prompt.submitted", t + 26_000, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.list(t + 26_001)
            .iter()
            .map(|n| n.part)
            .collect::<Vec<_>>(),
        [NotificationPart::Earlier, NotificationPart::Earlier]
    );
}

#[test]
fn an_agent_that_finished_while_yardsort_was_closed_is_caught_up_with() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    f.agent("s2", "pty-2");
    f.agent("s3", "pty-3");
    let turns = f.listening();
    let t = now_ms();
    let waiting = |idle: &[(&str, u32)]| -> Vec<Waiting> {
        idle.iter()
            .map(|(terminal, idle_ms)| Waiting {
                terminal: (*terminal).to_owned(),
                idle_ms: *idle_ms,
            })
            .collect()
    };

    // The first time the list ever listens, a quiet agent is not news.
    assert!(!catch_up(&f.store, &waiting(&[("pty-1", 1_000)]), t).unwrap());
    assert!(f.list(t).is_empty());

    // While Yardsort is open: s1 finishes and is seen, and the list keeps hearing.
    assert!(settled_terminal(&f.store, "pty-1", 9_000, t + 60_000).unwrap());
    assert_eq!(seen_all(&f.store, t + 61_000).unwrap(), 1);
    heard(&f.store, t + 100_000).unwrap();

    // Closed for an hour. s1 has not printed since it was seen; s2 stopped printing twenty
    // minutes ago, after ten minutes of work its harness said nothing of; s3 finished ten
    // minutes ago and its harness reported the turn, which was read on the way in; a shell is
    // nobody's agent.
    let open = t + 3_700_000;
    f.event("s2", "prompt.submitted", open - 1_800_000, json!({}));
    f.event("s3", "turn.completed", open - 600_000, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    let alive = waiting(&[
        ("pty-1", (open - (t + 57_000)) as u32),
        ("pty-2", 1_200_000),
        ("pty-3", 603_000),
        ("pty-shell", 5_000),
    ]);
    assert!(catch_up(&f.store, &alive, open).unwrap());
    let unseen: Vec<_> = f
        .list(open)
        .into_iter()
        .filter(|n| n.part == NotificationPart::Finished)
        .map(|n| (n.session_id.unwrap(), n.at, n.worked_ms))
        .collect();
    assert_eq!(
        unseen,
        [
            ("s3".to_owned(), open - 600_000, None),
            ("s2".to_owned(), open - 1_200_000, Some(600_000)),
        ]
    );

    // Opening it again a moment later finds nothing new.
    assert!(!catch_up(&f.store, &alive, open + 1_000).unwrap());
}

#[test]
fn a_shell_going_quiet_is_not_a_row() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    assert!(!settled_terminal(&f.store, "pty-shell", 60_000, now_ms()).unwrap());
    assert!(f.list(now_ms()).is_empty());
}

#[test]
fn a_failed_turn_says_so_even_after_the_quiet_told_its_end() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    let turns = f.listening();
    let t = now_ms();
    turns.observe(&busy("pty-1"), t);
    assert!(turns.observe(&quiet("pty-1", 9_000), t + 10_000).is_some());
    assert!(settled_terminal(&f.store, "pty-1", 9_000, t + 10_000).unwrap());
    f.event("s1", "turn.failed", t + 10_050, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.parts(t + 10_100),
        [(NotificationKind::AgentFailed, NotificationPart::Finished)]
    );
}

#[test]
fn an_agent_that_stops_with_an_error_is_told_and_one_that_was_closed_is_not() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    f.agent("s2", "pty-2");
    let turns = f.listening();
    let t = now_ms();

    // It was waiting on a permission when it died: nothing waits any more, and it stopped.
    f.event("s1", "approval.requested", t, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    f.store.end_session_by_pty("pty-1", Some(1)).unwrap();
    assert!(exited(&f.store, "s1", Some(1), t + 10).unwrap());
    let rows = f.list(t + 11);
    assert_eq!(
        rows.iter().map(|n| (n.kind, n.part)).collect::<Vec<_>>(),
        [
            (NotificationKind::AgentStopped, NotificationPart::Finished),
            (NotificationKind::AgentAsks, NotificationPart::Earlier)
        ]
    );
    assert_eq!(rows[0].exit_code, Some(1));
    assert_eq!(rows[0].terminal_id, None, "its process is gone");

    // Closed by its person, or left with 0: nothing to tell.
    f.store.end_session_by_pty("pty-2", Some(0)).unwrap();
    assert!(!exited(&f.store, "s2", None, t + 20).unwrap());
    assert!(!exited(&f.store, "s2", Some(0), t + 20).unwrap());
    assert_eq!(f.list(t + 21).len(), 2);
}

#[test]
fn seen_takes_finished_rows_only_and_all_takes_every_one() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    f.agent("s2", "pty-2");
    f.agent("s3", "pty-3");
    let turns = f.listening();
    let t = now_ms();
    f.event("s1", "approval.requested", t, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    assert!(settled_terminal(&f.store, "pty-2", 9_000, t + 1).unwrap());
    assert!(settled_terminal(&f.store, "pty-3", 9_000, t + 2).unwrap());
    let rows = f.list(t + 3);
    let id = |session: &str| {
        rows.iter()
            .find(|n| n.session_id.as_deref() == Some(session))
            .unwrap()
            .id
            .clone()
    };

    // A question is not something to mark seen: it leaves when it is answered.
    assert_eq!(seen(&f.store, &[id("s1")], t + 4).unwrap(), 0);
    assert_eq!(seen(&f.store, &[id("s2")], t + 4).unwrap(), 1);
    assert_eq!(seen(&f.store, &[id("s2")], t + 5).unwrap(), 0);
    assert_eq!(seen_all(&f.store, t + 6).unwrap(), 1);
    let parts: Vec<_> = f.list(t + 7).iter().map(|n| n.part).collect();
    assert_eq!(
        parts,
        [
            NotificationPart::Needs,
            NotificationPart::Earlier,
            NotificationPart::Earlier
        ]
    );
}

#[test]
fn earlier_is_kept_a_week_and_a_question_whose_agent_is_gone_leaves() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    f.agent("s2", "pty-2");
    let turns = f.listening();
    let t = now_ms();
    assert!(settled_terminal(&f.store, "pty-1", 9_000, t).unwrap());
    assert_eq!(seen_all(&f.store, t + 1).unwrap(), 1);
    assert_eq!(f.list(t + KEPT_MS).len(), 1);
    assert!(f.list(t + KEPT_MS + 2).is_empty());

    // The agent that asked ended while the app was closed, so no exit was heard.
    f.event("s2", "approval.requested", t, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    assert_eq!(
        f.parts(t + 5),
        [(NotificationKind::AgentAsks, NotificationPart::Needs)]
    );
    f.store.end_session_by_pty("pty-2", None).unwrap();
    assert_eq!(
        f.parts(t + 6),
        [(NotificationKind::AgentAsks, NotificationPart::Earlier)]
    );
}

#[test]
fn a_row_of_a_workspace_that_was_deleted_goes_with_it() {
    let f = Fixture::new();
    f.agent("s1", "pty-1");
    let t = now_ms();
    assert!(settled_terminal(&f.store, "pty-1", 9_000, t).unwrap());
    assert_eq!(f.list(t + 1).len(), 1);
    f.store
        .conn()
        .execute("DELETE FROM workspaces WHERE id = ?", [&f.workspace])
        .unwrap();
    assert!(f.list(t + 2).is_empty());
}

#[test]
fn a_row_shows_the_command_asked_for_and_what_the_agent_said_last() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path().join("wt-fix");
    std::fs::create_dir_all(&worktree).unwrap();
    let f = Fixture::at(&worktree.to_string_lossy());
    f.agent_of("s1", "pty-1", Some("claude-a"));
    f.agent("s2", "pty-2");
    let turns = f.listening();
    let t = now_ms();

    // Claude Code's recorded conversation, where it keeps one.
    let claude = dir.path().join("claude");
    let folder = claude.join("projects").join("-wt-fix");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/claude-transcript/2.1.302/conversation.jsonl"),
        folder.join("claude-a.jsonl"),
    )
    .unwrap();
    let sources = Sources {
        claude: vec![claude],
        ..Sources::default()
    };
    let expected = {
        let workspace = f.store.workspace(&f.workspace).unwrap().unwrap();
        let conversation = digest::one(&f.store, &workspace, "s1", &sources, t)
            .unwrap()
            .unwrap();
        one_line(conversation.turns.last().unwrap().reply().unwrap())
    };
    assert!(!expected.is_empty());

    assert!(settled_terminal(&f.store, "pty-1", 9_000, t).unwrap());
    f.event("s2", "approval.requested", t, json!({}));
    assert!(read_events(&f.store, &turns).unwrap());
    let data = dir.path().join("data");
    asks::post(
        &asks::asks_dir(&data),
        &asks::Ask {
            id: "ask-1".into(),
            at_ms: t,
            session_record_id: "s2".into(),
            harness: "claude".into(),
            tool: "Bash".into(),
            command: Some("git push --force-with-lease\n".into()),
            path: None,
            call: "call-1".into(),
        },
    )
    .unwrap();

    let lines = Lines::default();
    let mut rows = f.list(t + 1);
    lines.fill(&f.store, &sources, &data, &mut rows, t + 1);
    let asks_row = rows
        .iter()
        .find(|n| n.kind == NotificationKind::AgentAsks)
        .unwrap();
    assert_eq!(
        asks_row.command.as_deref(),
        Some("git push --force-with-lease")
    );
    assert_eq!(asks_row.said, None);
    let finished_row = rows
        .iter()
        .find(|n| n.kind == NotificationKind::AgentFinished)
        .unwrap();
    assert_eq!(finished_row.said.as_deref(), Some(expected.as_str()));
    assert_eq!(finished_row.command, None);

    // Seen, it keeps what was read while this process runs, and the file is not read again.
    std::fs::remove_file(folder.join("claude-a.jsonl")).unwrap();
    assert_eq!(seen_all(&f.store, t + 2).unwrap(), 1);
    let mut rows = f.list(t + 3);
    lines.fill(&f.store, &sources, &data, &mut rows, t + 3);
    let earlier = rows
        .iter()
        .find(|n| n.kind == NotificationKind::AgentFinished)
        .unwrap();
    assert_eq!(earlier.part, NotificationPart::Earlier);
    assert_eq!(earlier.said.as_deref(), Some(expected.as_str()));
}

#[test]
fn a_long_reply_is_cut_to_its_first_line() {
    assert_eq!(one_line("\n\n  Done.  \nAnd more."), "Done.");
    let long = "a".repeat(SAID_CHARS + 20);
    let cut = one_line(&long);
    assert_eq!(cut.chars().count(), SAID_CHARS + 1);
    assert!(cut.ends_with('…'));
    assert_eq!(one_line("   "), "");
}

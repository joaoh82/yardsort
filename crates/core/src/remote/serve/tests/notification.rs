//! `notification.list`, `notification.seen` and following the list, against a real store with
//! two projects and a keeper of the list.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::{code, Fixture, Recorded, ALL, LAPTOP, PHONE};
use crate::notifications::{self, Keeper, NotificationPart};
use crate::remote::serve::{Context, Visitor};
use crate::store::{now_ms, NewSession, NewWorkflowRun, StepStatus};
use remote_proto::{ProjectGrant, ProjectId, ProtocolVersion, Scope};

fn context<'a>(fx: &'a Fixture, keeper: &'a Keeper) -> Context<'a> {
    Context {
        notifications: Some(keeper),
        ..fx.context()
    }
}

/// Send one request on the channel, to a host that keeps the list; `params` is left out when
/// there is none to send.
fn ask(
    fx: &Fixture,
    keeper: &Keeper,
    visitor: &Visitor,
    out: &Recorded,
    id: &str,
    op: &str,
    params: Value,
) -> Value {
    let mut request = json!({
        "requestId": id,
        "protocol": ProtocolVersion::CURRENT,
        "op": op,
    });
    if !params.is_null() {
        request["params"] = params;
    }
    visitor
        .handle(&context(fx, keeper), &request.to_string())
        .unwrap();
    out.texts()
        .into_iter()
        .rev()
        .find(|t| t["requestId"] == id)
        .unwrap_or_else(|| panic!("no reply to {op}"))
}

/// An agent's conversation in `workspace`, running in `terminal`, that just finished a stretch
/// of work nobody saw.
fn finished(fx: &Fixture, workspace: &str, record: &str, terminal: &str) {
    fx.store
        .add_session(&NewSession {
            id: record,
            workspace_id: workspace,
            harness_id: "claude",
            model: None,
            effort: None,
            harness_session_id: None,
            title: "Cover half-hour zones",
            forked_from: None,
            pty_session_id: terminal,
            prompt: None,
        })
        .unwrap();
    assert!(notifications::settled_terminal(&fx.store, terminal, 30_000, now_ms()).unwrap());
}

fn parts(fx: &Fixture) -> Vec<(String, NotificationPart)> {
    let mut rows: Vec<_> = notifications::list(&fx.store, now_ms() + 1)
        .unwrap()
        .into_iter()
        .map(|row| (row.project_name, row.part))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn events(out: &Recorded) -> Vec<String> {
    out.texts()
        .iter()
        .filter_map(|text| text["event"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_device_reads_the_list_of_the_projects_it_reaches_when_it_may_watch() {
    let fx = Fixture::new();
    let keeper = Keeper::new(|| {});
    finished(&fx, &fx.alpha, "s-alpha", "pty-alpha");
    finished(&fx, &fx.beta, "s-beta", "pty-beta");
    let only_alpha = ProjectGrant::Only(BTreeSet::from([
        ProjectId::new(fx.alpha_project.clone()).unwrap()
    ]));
    fx.approve(PHONE, "g-phone", &ALL, only_alpha);
    fx.approve(
        LAPTOP,
        "g-laptop",
        &[Scope::WorkspaceRead],
        ProjectGrant::All,
    );

    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    // A host that keeps the list says so; one that does not, does not, and answers none of it.
    let described = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "d1",
        "host.describe",
        Value::Null,
    );
    let offered = described["result"]["capabilities"].to_string();
    for op in [
        "notification.list",
        "notification.seen",
        "notification.follow",
        "notification.unfollow",
    ] {
        assert!(offered.contains(op), "{op} is not offered: {offered}");
    }
    let plain = super::ask(&fx, &phone, &out, "d2", "host.describe", json!({}));
    assert!(!plain["result"]["capabilities"]
        .to_string()
        .contains("notification."));
    let refused = super::ask(&fx, &phone, &out, "l0", "notification.list", json!({}));
    assert_eq!(code(&refused), "unsupported");

    let listed = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "l1",
        "notification.list",
        json!({}),
    );
    let page = &listed["result"];
    assert_eq!(page["kind"], "notifications", "{listed}");
    assert_eq!(
        (&page["needs"], &page["finished"], &page["more"]),
        (&json!(0), &json!(1), &json!(false))
    );
    let rows = page["notifications"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the project it reaches: {listed}");
    let row = &rows[0];
    assert_eq!(row["kind"], "agent.finished");
    assert_eq!(row["part"], "finished");
    assert_eq!(row["projectName"], "alpha");
    assert_eq!(row["workspace"], fx.alpha.as_str());
    assert_eq!(row["workspaceName"], "work");
    assert_eq!(row["session"], "s-alpha");
    assert_eq!(row["title"], "Cover half-hour zones");
    assert_eq!(row["harness"], "claude");
    assert_eq!(row["terminal"], "pty-alpha");
    assert_eq!(row["workedMs"], 30_000);

    // A device let in only to see workspaces may not watch, and is not shown what agents said.
    let out = Recorded::new(usize::MAX);
    let laptop = fx.visitor(LAPTOP, "g-laptop", &out);
    for (id, op, params) in [
        ("f1", "notification.list", json!({})),
        ("f2", "notification.seen", json!({"all": true})),
        ("f3", "notification.follow", Value::Null),
    ] {
        let reply = ask(&fx, &keeper, &laptop, &out, id, op, params);
        assert_eq!(code(&reply), "forbidden", "{op}");
    }
    assert!(!laptop.following());
}

#[test]
fn seen_marks_what_the_grant_is_shown_and_nothing_else() {
    let fx = Fixture::new();
    let keeper = Keeper::new(|| {});
    finished(&fx, &fx.alpha, "s-alpha", "pty-alpha");
    finished(&fx, &fx.beta, "s-beta", "pty-beta");
    let beta_row = notifications::list(&fx.store, now_ms() + 1)
        .unwrap()
        .into_iter()
        .find(|row| row.project_name == "beta")
        .unwrap()
        .id;
    let only_alpha = ProjectGrant::Only(BTreeSet::from([
        ProjectId::new(fx.alpha_project.clone()).unwrap()
    ]));
    fx.approve(PHONE, "g-phone", &ALL, only_alpha);
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);

    // Nothing named and not all is nothing asked.
    let empty = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "s0",
        "notification.seen",
        json!({}),
    );
    assert_eq!(code(&empty), "invalid_request");

    // A row of a project it does not reach is passed over, as one that is not there.
    let other = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "s1",
        "notification.seen",
        json!({"ids": [beta_row]}),
    );
    assert_eq!(other["result"]["kind"], "done", "{other}");
    assert_eq!(
        parts(&fx),
        [
            ("alpha".to_owned(), NotificationPart::Finished),
            ("beta".to_owned(), NotificationPart::Finished),
        ]
    );

    // All of them is all it is shown.
    let all = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "s2",
        "notification.seen",
        json!({"all": true}),
    );
    assert_eq!(all["result"]["kind"], "done", "{all}");
    assert_eq!(
        parts(&fx),
        [
            ("alpha".to_owned(), NotificationPart::Earlier),
            ("beta".to_owned(), NotificationPart::Finished),
        ]
    );
    let listed = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "l1",
        "notification.list",
        json!({}),
    );
    assert_eq!(listed["result"]["finished"], 0);
    assert_eq!(listed["result"]["notifications"][0]["part"], "earlier");
}

#[test]
fn a_device_that_follows_the_list_is_told_when_it_changes_and_only_then() {
    let fx = Fixture::new();
    let keeper = Keeper::new(|| {});
    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    assert!(!phone.following());

    let followed = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "f1",
        "notification.follow",
        Value::Null,
    );
    assert_eq!(followed["result"]["kind"], "done", "{followed}");
    assert!(phone.following(), "the host's owner polls it now");
    phone.poll(&context(&fx, &keeper));
    assert!(events(&out).is_empty(), "nothing changed yet");

    finished(&fx, &fx.alpha, "s-alpha", "pty-alpha");
    phone.poll(&context(&fx, &keeper));
    assert_eq!(events(&out), ["notifications.changed"]);
    // Told once for one change, however often it is looked at.
    phone.poll(&context(&fx, &keeper));
    assert_eq!(events(&out).len(), 1);

    // Seen from another place is a change too.
    notifications::seen_all(&fx.store, now_ms()).unwrap();
    phone.poll(&context(&fx, &keeper));
    assert_eq!(events(&out).len(), 2);

    let stopped = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "f2",
        "notification.unfollow",
        Value::Null,
    );
    assert_eq!(stopped["result"]["kind"], "done", "{stopped}");
    assert!(!phone.following());
    finished(&fx, &fx.beta, "s-beta", "pty-beta");
    phone.poll(&context(&fx, &keeper));
    assert_eq!(events(&out).len(), 2, "it follows no more");
}

#[test]
fn what_a_workflow_asks_is_said_only_to_a_grant_that_may_read_workflows() {
    let fx = Fixture::new();
    let keeper = Keeper::new(|| {});
    fx.store
        .queue_workflow_run(&NewWorkflowRun {
            id: "run-1",
            workflow_id: "review",
            workflow_name: "Review",
            definition: "id: review\n",
            project_id: &fx.alpha_project,
            workspace_id: &fx.alpha,
            workspace_name: "work",
            inputs: "{}",
            context: "{}",
            requested_by: "app",
            steps: &[("clarify", "wait_session")],
        })
        .unwrap();
    assert!(fx.store.start_workflow_run("run-1", None).unwrap());
    assert!(fx
        .store
        .move_workflow_step(
            "run-1",
            "clarify",
            &[StepStatus::Pending],
            StepStatus::Waiting,
            None,
            None,
            None,
            false,
        )
        .unwrap());
    assert!(fx
        .store
        .open_workflow_wait(
            "run-1",
            "clarify",
            crate::workflow::waits::KIND_QUESTION,
            "your answer",
            "Answer the question.",
            Some("Which zones should the fix cover?"),
            "[]",
            "{}",
        )
        .unwrap());

    fx.approve(PHONE, "g-phone", &ALL, ProjectGrant::All);
    let mut with_workflows = ALL.to_vec();
    with_workflows.push(Scope::WorkflowRead);
    fx.approve(LAPTOP, "g-laptop", &with_workflows, ProjectGrant::All);

    // Listing is what finds the wait: nothing told the list of it.
    let out = Recorded::new(usize::MAX);
    let phone = fx.visitor(PHONE, "g-phone", &out);
    let listed = ask(
        &fx,
        &keeper,
        &phone,
        &out,
        "l1",
        "notification.list",
        json!({}),
    );
    let row = &listed["result"]["notifications"][0];
    assert_eq!(listed["result"]["needs"], 1, "{listed}");
    assert_eq!(row["kind"], "workflow.question");
    assert_eq!(row["title"], "Review");
    assert_eq!(row["run"], "run-1");
    assert_eq!(row["step"], "clarify");
    assert!(row.get("said").is_none(), "{row}");

    let out = Recorded::new(usize::MAX);
    let laptop = fx.visitor(LAPTOP, "g-laptop", &out);
    let listed = ask(
        &fx,
        &keeper,
        &laptop,
        &out,
        "l2",
        "notification.list",
        json!({}),
    );
    assert_eq!(
        listed["result"]["notifications"][0]["said"],
        "Which zones should the fix cover?"
    );
}

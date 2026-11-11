use super::*;

/// A valid file, for the tests to break one thing at a time.
const BASE: &str = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
inputs:
  - id: who
    kind: harness
  - id: note
    kind: text
steps:
  - id: start
    action: start_session
    harness: "{{ inputs.who }}"
    prompt: Hello
"#;

/// `BASE` with more steps after its own.
fn with_steps(extra: &str) -> String {
    format!("{BASE}{extra}")
}

/// Every problem, as `line:column: message`.
fn problems(text: &str) -> Vec<String> {
    match parse(text) {
        Ok(_) => Vec::new(),
        Err(invalid) => invalid.problems.iter().map(ToString::to_string).collect(),
    }
}

/// The one problem `text` has. Fails the test when there are none or several.
fn problem(text: &str) -> String {
    let found = problems(text);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one problem, got {found:#?}"
    );
    found.into_iter().next().unwrap()
}

#[test]
fn the_base_fixture_is_valid() {
    let workflow = parse(BASE).unwrap();
    assert_eq!(workflow.id, "demo");
    assert_eq!(
        workflow.inputs[0].label, "who",
        "the label falls back to the id"
    );
    assert!(
        !workflow.inputs[0].required,
        "inputs are optional unless they say so"
    );
    assert_eq!(
        workflow.trigger.context,
        RunContext::Workspace,
        "the default context"
    );
}

#[test]
fn the_built_in_code_review_is_valid_and_ends_in_two_branches() {
    for (id, text) in BUILT_IN {
        let workflow = parse(text).unwrap_or_else(|e| panic!("{id}: {:#?}", e.problems));
        assert_eq!(
            workflow.id, *id,
            "a built-in's id is the one it is listed under"
        );
    }
    let review = parse(BUILT_IN[0].1).unwrap();
    let needs = |id: &str| {
        review
            .steps
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.needs.clone())
            .unwrap()
    };
    assert_eq!(needs("tell_user"), vec!["posted"]);
    assert_eq!(needs("tell_author"), vec!["posted"]);
    let author = review.steps.iter().find(|s| s.id == "tell_author").unwrap();
    assert!(matches!(
        &author.action,
        Action::SendToSession {
            session: SessionRef::Origin,
            ..
        }
    ));
    let settled = review.steps.iter().find(|s| s.id == "settled").unwrap();
    assert_eq!(
        settled.action,
        Action::WaitSession {
            session: SessionRef::Step("review".into()),
            until: Until::Settled,
            timeout_secs: Some(2 * 60 * 60),
            clarify: false,
            max_rounds: 0,
        }
    );
}

#[test]
fn the_built_in_software_factory_parses_with_its_phases_and_loops() {
    let text = BUILT_IN
        .iter()
        .find(|(id, _)| *id == "software-factory")
        .expect("software-factory ships")
        .1;
    let workflow = parse(text).unwrap_or_else(|invalid| panic!("{:#?}", invalid.problems));
    assert_eq!(workflow.id, "software-factory");
    assert_eq!(workflow.name, "Software factory");
    assert_eq!(workflow.trigger.context, RunContext::Workspace);
    let input = |id: &str| workflow.inputs.iter().find(|input| input.id == id).unwrap();
    assert!(input("task").required && matches!(input("task").kind, InputKind::Text));
    assert!(input("agent").required && matches!(input("agent").kind, InputKind::Harness));
    assert!(input("suite").required && matches!(input("suite").kind, InputKind::Test));

    let ids: Vec<&str> = workflow.steps.iter().map(|step| step.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "define",
            "brief",
            "gather",
            "context",
            "plan",
            "plan-ready",
            "approve-plan",
            "implement",
            "implement-ready",
            "verify",
            "await-review",
            "accept-review",
            "release",
            "ready",
        ]
    );
    let step = |id: &str| workflow.steps.iter().find(|step| step.id == id).unwrap();

    let brief = step("brief");
    assert!(matches!(
        &brief.action,
        Action::WaitSession {
            clarify: true,
            max_rounds: 5,
            session: SessionRef::Step(source),
            ..
        } if source == "define"
    ));
    assert_eq!(brief.produces[0].name, "brief");
    assert_eq!(brief.produces[0].source, ArtifactSource::Brief);

    assert_eq!(
        step("gather").consumes,
        ["brief"],
        "context gathering reads the brief"
    );
    assert_eq!(
        step("context").produces[0].source,
        ArtifactSource::File {
            path: "factory/context.md".into(),
        }
    );
    assert_eq!(
        step("plan").consumes,
        ["brief", "context", "changes-approve-plan"]
    );
    assert_eq!(
        step("plan-ready").produces[0].source,
        ArtifactSource::File {
            path: "factory/plan.md".into(),
        }
    );
    let prompt = |id: &str| match &step(id).action {
        Action::StartSession { prompt, .. } => prompt.clone().unwrap_or_default(),
        other => panic!("{id} is not a session: {other:?}"),
    };
    let gather = prompt("gather");
    assert!(
        gather.contains("Delete `factory/context.md`"),
        "gather does not clear the previous notes: {gather}"
    );
    let plan = prompt("plan");
    assert!(
        plan.contains("Delete `factory/plan.md`"),
        "plan does not clear the previous plan: {plan}"
    );
    let implement = prompt("implement");
    assert!(
        implement.contains("gh pr create --draft"),
        "implement does not open a draft pull request: {implement}"
    );
    assert!(
        !implement.contains("pr.number"),
        "implement still names the pull request, which may not exist yet: {implement}"
    );

    let plan_gate = step("approve-plan");
    assert!(plan_gate.gate);
    assert_eq!(plan_gate.consumes, ["plan"]);
    assert_eq!(plan_gate.max_attempts, Some(3));
    assert!(plan_gate
        .routes
        .iter()
        .any(|route| { route.when == OutcomeName::Approved && route.to == "implement" }));
    assert!(plan_gate
        .routes
        .iter()
        .any(|route| { route.when == OutcomeName::ChangesRequested && route.to == "plan" }));
    assert!(plan_gate
        .routes
        .iter()
        .any(|route| route.when == OutcomeName::Rejected && route.stops()));

    assert_eq!(
        step("implement").consumes,
        ["plan", "failure-verify", "changes-accept-review"]
    );

    let verify = step("verify");
    assert!(verify.gate);
    assert_eq!(verify.max_attempts, Some(3));
    assert!(matches!(
        &verify.action,
        Action::RunTest { test, .. } if test == "{{ inputs.suite }}"
    ));
    assert!(verify
        .routes
        .iter()
        .any(|route| route.when == OutcomeName::Passed && route.to == "await-review"));
    assert!(verify
        .routes
        .iter()
        .any(|route| route.when == OutcomeName::Failed && route.to == "implement"));

    assert!(matches!(
        step("await-review").action,
        Action::WaitPrActivity {
            kind: PrActivity::Review,
            timeout_secs: None,
        }
    ));

    let review_gate = step("accept-review");
    assert_eq!(review_gate.max_attempts, Some(3));
    assert!(review_gate
        .routes
        .iter()
        .any(|route| { route.when == OutcomeName::Approved && route.to == "release" }));
    assert!(review_gate
        .routes
        .iter()
        .any(|route| { route.when == OutcomeName::ChangesRequested && route.to == "implement" }));
    assert!(review_gate
        .routes
        .iter()
        .any(|route| route.when == OutcomeName::Rejected && route.stops()));

    let release = step("release");
    assert!(
        release.routes.is_empty(),
        "the release gate has no revision loop"
    );
    assert!(matches!(release.action, Action::Approve { .. }));
    assert_eq!(step("ready").needs, ["release"]);
    assert!(matches!(step("ready").action, Action::Notify { .. }));
}

#[test]
fn the_parsed_form_is_serialised_the_way_the_app_will_read_it() {
    let workflow = parse(BUILT_IN[0].1).unwrap();
    let json = serde_json::to_value(&workflow).unwrap();
    assert_eq!(json["trigger"]["kind"], "manual");
    assert_eq!(json["steps"][0]["action"], "start_session");
    assert_eq!(json["steps"][0]["skipMemory"], false);
    assert_eq!(
        json["steps"][1]["session"],
        serde_json::json!({ "step": "review" })
    );
    assert_eq!(json["steps"][1]["timeoutSecs"], 7200);
    assert_eq!(json["steps"][4]["session"], "origin");
}

// Shape and types: serde's pass, with the position kept and its wording tidied.

#[test]
fn an_unknown_key_is_named_at_its_line() {
    let found = problem(&BASE.replace("    prompt: Hello", "    promt: Hello"));
    assert!(found.starts_with("15:5: Unknown field `promt`"), "{found}");
}

#[test]
fn a_missing_key_is_reported() {
    let found = problem(&BASE.replace("name: Demo\n", ""));
    assert!(found.contains("Missing field `name`"), "{found}");
}

#[test]
fn a_key_given_twice_is_reported_plainly() {
    let found = problem(&BASE.replace("name: Demo\n", "name: Demo\nname: Again\n"));
    assert_eq!(found, "3:1: `name` is given twice.");
}

#[test]
fn a_value_of_the_wrong_type_is_reported() {
    let found = problem(&BASE.replace("version: 1", "version: one"));
    assert!(found.starts_with("3:10:"), "{found}");
}

#[test]
fn a_file_from_a_later_version_says_so_instead_of_blaming_its_new_keys() {
    let text = BASE.replace("version: 1", "version: 2\nretries: 3");
    assert_eq!(
        problem(&text),
        "3:10: This file is for a later Yardsort (workflow version 2); this one reads version 1. \
         Update Yardsort to run it."
    );
    let invalid = parse(&text).unwrap_err();
    assert_eq!(
        invalid.id.as_deref(),
        Some("demo"),
        "it is still listed by its id"
    );
    assert_eq!(invalid.name.as_deref(), Some("Demo"));
}

#[test]
fn an_unparseable_file_still_gives_its_name_when_it_can() {
    let invalid = parse(&BASE.replace("kind: manual", "kind: [manual]")).unwrap_err();
    assert_eq!(
        invalid.id.as_deref(),
        Some("demo"),
        "shape is wrong, but it is YAML"
    );
    let invalid = parse("id: demo\nnot: [a workflow").unwrap_err();
    assert_eq!(
        invalid.id, None,
        "not YAML at all, so nothing can be read from it"
    );
    let invalid = parse("not: [a workflow").unwrap_err();
    assert_eq!(invalid.id, None);
}

// The file as a whole.

#[test]
fn the_version_must_be_this_one() {
    assert_eq!(
        problem(&BASE.replace("version: 1", "version: 0")),
        "3:10: `version` is 1 for this Yardsort."
    );
}

#[test]
fn the_id_is_a_file_name() {
    let found = problem(&BASE.replace("id: demo", "id: My Demo"));
    assert!(
        found.starts_with("1:5: `My Demo` cannot be an id."),
        "{found}"
    );
    assert!(problems(&BASE.replace("id: demo", "id: 2-demo")).is_empty());
}

#[test]
fn the_name_is_not_empty() {
    assert_eq!(
        problem(&BASE.replace("name: Demo", "name: \"  \"")),
        "2:7: The name is empty."
    );
}

#[test]
fn only_a_manual_trigger_exists() {
    let found = problem(&BASE.replace("kind: manual", "kind: schedule"));
    assert!(
        found.starts_with("5:9: `schedule` is not a trigger"),
        "{found}"
    );
}

#[test]
fn only_a_workspace_context_exists() {
    let found = problem(&BASE.replace("kind: manual", "kind: manual\n  context: project"));
    assert!(
        found.starts_with("6:12: `project` is not a context"),
        "{found}"
    );
}

// Inputs.

#[test]
fn an_input_id_must_be_usable_in_a_variable() {
    let found = problem(&BASE.replace("id: note", "id: Note"));
    assert!(
        found.starts_with("9:9: `Note` cannot be an input's id."),
        "{found}"
    );
}

#[test]
fn input_ids_are_unique() {
    assert_eq!(
        problem(&BASE.replace("id: note", "id: who")),
        "9:9: There is already an input `who`, on line 7."
    );
}

#[test]
fn an_input_kind_is_one_of_three() {
    let found = problem(&BASE.replace("kind: text", "kind: number"));
    assert!(
        found.starts_with("10:11: `number` is not a kind of input."),
        "{found}"
    );
}

#[test]
fn a_choice_needs_options_and_only_a_choice_has_them() {
    let found = problem(&BASE.replace("kind: text", "kind: choice"));
    assert!(
        found.contains("The choice `note` needs `options`"),
        "{found}"
    );
    let found = problem(&BASE.replace("kind: text", "kind: text\n    options: [a, b]"));
    assert_eq!(found, "11:14: Only a `choice` input has `options`.");
    let found = problem(&BASE.replace("kind: text", "kind: choice\n    options: []"));
    assert_eq!(found, "11:14: A choice needs at least one option.");
}

#[test]
fn choice_options_are_unique_and_the_default_is_one_of_them() {
    let found = problem(&BASE.replace("kind: text", "kind: choice\n    options: [quick, quick]"));
    assert_eq!(found, "11:22: `quick` is listed twice.");
    let found = problem(&BASE.replace(
        "kind: text",
        "kind: choice\n    options: [quick, deep]\n    default: slow",
    ));
    assert_eq!(found, "12:14: `slow` is not one of the options.");
    let fine = BASE.replace(
        "kind: text",
        "kind: choice\n    options: [quick, deep]\n    default: deep",
    );
    assert_eq!(
        parse(&fine).unwrap().inputs[1].default.as_deref(),
        Some("deep")
    );
}

// Steps.

#[test]
fn a_workflow_has_steps() {
    let text = format!("{}steps: []\n", &BASE[..BASE.find("steps:").unwrap()]);
    assert_eq!(problem(&text), "11:8: A workflow needs at least one step.");
}

#[test]
fn step_ids_are_names_and_unique() {
    let found = problem(&BASE.replace("id: start", "id: Start"));
    assert!(
        found.starts_with("12:9: `Start` cannot be a step's id."),
        "{found}"
    );
    let reserved = problem(&with_steps(
        "  - id: stop\n    action: notify\n    title: Hi\n",
    ));
    assert!(
        reserved.contains("`stop` cannot be a step's id"),
        "{reserved}"
    );
    let found = problem(&with_steps(
        "  - id: start\n    action: notify\n    title: Hi\n",
    ));
    assert_eq!(found, "16:9: There is already a step `start`, on line 12.");
}

#[test]
fn an_unknown_action_lists_the_real_ones() {
    let found = problem(&with_steps("  - id: go\n    action: run_command\n"));
    assert!(
        found.starts_with("17:13: `run_command` is not an action."),
        "{found}"
    );
    assert!(found.contains("`wait_pr_activity`"), "{found}");
}

#[test]
fn a_run_test_file_from_before_gates_still_loads_and_is_not_a_gate() {
    let workflow = parse(include_str!("builtin/smoke-then-tell.yaml")).unwrap();
    let Action::RunTest { test, .. } = &workflow.steps[0].action else {
        panic!("the built-in starts with run_test");
    };
    assert!(
        !workflow.steps[0].is_gate(),
        "omitting gate keeps today's behaviour"
    );
    assert_eq!(test, "{{ inputs.suite }}");
}

#[test]
fn a_run_test_step_can_be_marked_as_a_gate() {
    let workflow = parse(&with_steps(
        "  - id: check\n    action: run_test\n    test: login\n    gate: true\n",
    ))
    .unwrap();
    let Action::RunTest { .. } = &workflow.steps[1].action else {
        panic!("run_test");
    };
    assert!(workflow.steps[1].gate);
    assert!(workflow.steps[1].is_gate());
}

#[test]
fn an_approval_is_a_gate_and_a_pull_request_wait_is_not() {
    let workflow = parse(&with_steps(
        "  - id: sign-off\n    action: approve\n    prompt: \"Look at {{ inputs.note }}.\"\n",
    ))
    .unwrap();
    let step = workflow
        .steps
        .iter()
        .find(|step| step.id == "sign-off")
        .unwrap();
    assert!(matches!(step.action, Action::Approve { .. }));
    assert!(step.is_gate());
    assert!(step.gate);

    let refused = problem(&with_steps(
        "  - id: sign-off\n    action: approve\n    gate: false\n",
    ));
    assert!(refused.contains("An approval step is a gate"), "{refused}");

    let waiting = parse(&with_steps("  - id: watch\n    action: wait_pr_activity\n")).unwrap();
    let watch = waiting
        .steps
        .iter()
        .find(|step| step.id == "watch")
        .unwrap();
    assert!(matches!(watch.action, Action::WaitPrActivity { .. }));
    assert!(!watch.is_gate());
}

#[test]
fn a_changes_prefix_is_reserved_for_the_request_artifact() {
    let found = problem(&with_steps(
        "  - id: sign-off\n    action: approve\n    produces:\n      - name: changes-sign-off\n        text: notes\n",
    ));
    assert!(found.contains("is reserved"), "{found}");
    assert!(found.contains("changes-sign-off"), "{found}");
}

#[test]
fn gate_on_another_action_is_named_as_misplaced() {
    let found = problem(&with_steps(
        "  - id: go\n    action: notify\n    title: Hi\n    gate: true\n",
    ));
    assert!(
        found.ends_with("`gate` is not something a `notify` step takes."),
        "{found}"
    );
}

#[test]
fn a_key_on_the_wrong_action_is_named_as_misplaced() {
    let found = problem(&with_steps(
        "  - id: go\n    action: notify\n    title: Hi\n    harness: claude\n",
    ));
    assert_eq!(
        found,
        "19:14: `harness` is not something a `notify` step takes."
    );
}

#[test]
fn each_action_names_what_it_cannot_do_without() {
    let found = problem(&BASE.replace("    harness: \"{{ inputs.who }}\"\n", ""));
    assert!(
        found.starts_with("13:13: A `start_session` step needs `harness`"),
        "{found}"
    );
    let found = problem(&with_steps("  - id: go\n    action: notify\n"));
    assert!(
        found.starts_with("17:13: A `notify` step needs `title`"),
        "{found}"
    );
    let found = problem(&with_steps(
        "  - id: go\n    action: send_to_session\n    session: origin\n",
    ));
    assert!(
        found.starts_with("17:13: A `send_to_session` step needs `prompt`"),
        "{found}"
    );
    let found = problem(&with_steps("  - id: go\n    action: wait_session\n"));
    assert!(
        found.starts_with("17:13: A `wait_session` step needs `session`"),
        "{found}"
    );
}

// The graph.

#[test]
fn a_need_names_a_step_and_close_misspellings_are_suggested() {
    let found = problem(&with_steps(
        "  - id: go\n    action: notify\n    needs: [strat]\n    title: Hi\n",
    ));
    assert_eq!(
        found,
        "18:13: There is no step `strat`. Did you mean `start`?"
    );
}

#[test]
fn a_step_cannot_need_itself_or_the_same_step_twice() {
    let found = problem(&with_steps(
        "  - id: go\n    action: notify\n    needs: [go]\n    title: Hi\n",
    ));
    assert_eq!(found, "18:13: `go` cannot need itself.");
    let found = problem(&with_steps(
        "  - id: go\n    action: notify\n    needs: [start, start]\n    title: Hi\n",
    ));
    assert_eq!(found, "18:20: `start` is listed twice.");
}

#[test]
fn a_circle_is_found_and_walked() {
    let found = problem(&with_steps(
        "  - id: a\n    action: notify\n    needs: [b]\n    title: A\n\
         \x20 - id: b\n    action: notify\n    needs: [c]\n    title: B\n\
         \x20 - id: c\n    action: notify\n    needs: [a]\n    title: C\n",
    ));
    assert_eq!(
        found,
        "18:13: These steps wait for each other, so none of them could ever start: a → b → c → a."
    );
}

// Harnesses and sessions.

#[test]
fn a_harness_is_an_id_or_one_harness_input() {
    assert!(problems(&BASE.replace("\"{{ inputs.who }}\"", "claude")).is_empty());
    let found = problem(&BASE.replace("\"{{ inputs.who }}\"", "Claude Code"));
    assert_eq!(
        found,
        "14:14: `Claude Code` is not a harness id, like `claude` or `codex`."
    );
    for bad in [
        "\"{{ inputs.note }}\"",
        "\"{{ inputs.who }}-beta\"",
        "\"{{ pr.title }}\"",
    ] {
        let found = problem(&BASE.replace("\"{{ inputs.who }}\"", bad));
        assert!(
            found.starts_with("14:14: `harness` is a harness id, or one"),
            "{bad}: {found}"
        );
    }
}

#[test]
fn a_session_is_origin_or_an_earlier_started_session() {
    let wait = |session: &str, needs: &str| {
        with_steps(&format!(
            "  - id: wait\n    action: wait_session\n    needs: [{needs}]\n    session: \"{session}\"\n"
        ))
    };
    assert!(problems(&wait("origin", "start")).is_empty());
    assert!(problems(&wait("{{ steps.start.session }}", "start")).is_empty());
    let found = problem(&wait("{{ steps.start.session }}", ""));
    assert!(
        found.contains("is used before step `start` is sure to have run"),
        "{found}"
    );
    let found = problem(&wait("{{ steps.strt.session }}", "start"));
    assert!(
        found.ends_with("There is no step `strt`. Did you mean `start`?"),
        "{found}"
    );
    let found = problem(&wait("{{ steps.start.run }}", "start"));
    assert!(found.contains("`session` is `origin`"), "{found}");
    let found = problem(&wait("the reviewer", "start"));
    assert!(found.contains("`session` is `origin`"), "{found}");
    let found = problem(&with_steps(
        "  - id: ping\n    action: notify\n    title: Hi\n\
         \x20 - id: wait\n    action: wait_session\n    needs: [ping]\n    \
         session: \"{{ steps.ping.session }}\"\n",
    ));
    assert!(
        found.contains("`ping` is not a `start_session` step"),
        "{found}"
    );
}

#[test]
fn what_to_wait_for_is_one_of_the_known_things() {
    let found = problem(&with_steps(
        "  - id: w\n    action: wait_session\n    session: origin\n    until: idle\n",
    ));
    assert!(
        found.starts_with("19:12: `idle` is not a thing to wait for."),
        "{found}"
    );
    let found = problem(&with_steps(
        "  - id: w\n    action: wait_pr_activity\n    kind: approval\n",
    ));
    assert!(
        found.starts_with("18:11: `approval` is not a kind of pull request activity"),
        "{found}"
    );
    let found = problem(&with_steps(
        "  - id: w\n    action: wait_pr_activity\n    timeout: soon\n",
    ));
    assert!(
        found.starts_with("18:14: `soon` is not a timeout."),
        "{found}"
    );
}

#[test]
fn a_timeout_ending_in_a_wide_character_is_a_problem_not_a_panic() {
    let text = with_steps("  - id: w\n    action: wait_pr_activity\n    timeout: 1秒\n");
    let found = problem(&text);
    assert!(
        found.starts_with("18:14: `1秒` is not a timeout."),
        "{found}"
    );
    // The catalog parses every file it lists, so one such file must not take the listing down.
    let dir = data_dir();
    write(dir.path(), "wide.yaml", &text);
    let listed = catalog(dir.path());
    assert!(!listed.iter().find(|e| e.id == "demo").unwrap().runnable());
}

// Variables.

#[test]
fn every_variable_names_something_that_exists() {
    let cases = [
        ("{{ workspace.brnch }}", "`workspace` has no `brnch`"),
        (
            "{{ pr }}",
            "`{{ pr }}` needs a field: `pr.number`, `pr.url`, `pr.title`, `pr.repo`, `pr.clone`.",
        ),
        ("{{ memory.entries }}", "`memory` is used whole"),
        (
            "{{ inputs.nte }}",
            "there is no input `nte`. Did you mean `note`?",
        ),
        (
            "{{ inputs.note.text }}",
            "write an input as `{{ inputs.<id> }}`",
        ),
        ("{{ steps.start }}", "write a step's result as"),
        ("{{ branch }}", "there is no `branch`. Variables start with"),
        (
            "{{ pr.clone }}",
            "is for a pull request a workspace tracks in another",
        ),
    ];
    for (variable, expected) in cases {
        let found = problem(&BASE.replace("prompt: Hello", &format!("prompt: \"{variable}\"")));
        assert!(found.contains(expected), "{variable}: {found}");
    }
    for fine in [
        "{{ project.root }}",
        "{{ workspace.base_branch }}",
        "{{ pr.url }}",
        "{{ inputs.note }}",
        "{{ memory }}",
        "{{ handoff }}",
    ] {
        let text = BASE.replace("prompt: Hello", &format!("prompt: \"{fine}\""));
        assert!(problems(&text).is_empty(), "{fine}: {:?}", problems(&text));
    }
}

#[test]
fn a_step_result_needs_that_step_among_its_needs_even_indirectly() {
    let text = |needs: &str, field: &str| {
        with_steps(&format!(
            "  - id: wait\n    action: wait_session\n    needs: [start]\n    session: origin\n\
             \x20 - id: say\n    action: notify\n    needs: [{needs}]\n    \
             title: \"{{{{ steps.start.{field} }}}}\"\n"
        ))
    };
    assert!(
        problems(&text("wait", "session")).is_empty(),
        "start is before wait"
    );
    let found = problem(&text("", "session"));
    assert!(
        found.contains("Add `start` to this step's `needs`"),
        "{found}"
    );
    let found = problem(&text("wait", "count"));
    assert!(
        found.contains("a `start_session` step leaves `session`, `run`."),
        "{found}"
    );
    let notify_result = with_steps(
        "  - id: ping\n    action: notify\n    title: Hi\n\
         \x20 - id: say\n    action: notify\n    needs: [ping]\n    \
         title: \"{{ steps.ping.anything }}\"\n",
    );
    assert!(problem(&notify_result).contains("a `notify` step leaves nothing to use."));
}

#[test]
fn a_problem_in_a_long_prompt_is_reported_at_its_own_line_and_column() {
    let text = BASE.replace(
        "prompt: Hello",
        "prompt: |\n      First line.\n      Second has {{ workspace.nme }} in it.\n      Third.",
    );
    let found = problem(&text);
    assert!(found.starts_with("17:18: `{{ workspace.nme }}`"), "{found}");
}

#[test]
fn a_malformed_placeholder_is_reported_where_it_opens() {
    let text = BASE.replace(
        "prompt: Hello",
        "prompt: |\n      Fine.\n      Then {{ pr.number | upper }} and {{ unclosed",
    );
    let found = problems(&text);
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found[0].starts_with("17:12: `{{ pr.number | upper }}` is not a variable."),
        "{}",
        found[0]
    );
    assert!(
        found[1].starts_with("17:40: `{{` is never closed"),
        "{}",
        found[1]
    );
}

#[test]
fn the_same_mistake_twice_is_reported_twice_each_at_its_own_line() {
    let text = BASE.replace(
        "prompt: Hello",
        "prompt: |\n      One {{ pr.nubmer }}.\n      Two {{ pr.nubmer }}.",
    );
    let found = problems(&text);
    let places: Vec<&str> = found.iter().map(|p| &p[..p.find(": ").unwrap()]).collect();
    assert_eq!(places, vec!["16:11", "17:11"], "{found:#?}");
}

#[test]
fn an_escaped_literal_is_not_blamed_for_the_real_mistake_after_it() {
    let text = BASE.replace(
        "prompt: Hello",
        "prompt: |\n      Literal \\{{ pr.nubmer }} here.\n      Real {{ pr.nubmer }} here.",
    );
    assert!(
        problem(&text).starts_with("17:12: `{{ pr.nubmer }}`"),
        "{}",
        problem(&text)
    );
    // In a double-quoted scalar the file has `\\{{` for the value's `\{{`; still a literal.
    let quoted = BASE.replace(
        "prompt: Hello",
        r#"prompt: "\\{{ pr.nubmer }} and {{ pr.nubmer }}""#,
    );
    assert!(
        problem(&quoted).starts_with("15:36: `{{ pr.nubmer }}`"),
        "{}",
        problem(&quoted)
    );
}

#[test]
fn every_problem_is_reported_in_file_order() {
    let text = with_steps(
        "  - id: go\n    action: notify\n    needs: [nothing]\n    title: \"{{ nope }}\"\n",
    )
    .replace("id: demo", "id: Demo!");
    let found = problems(&text);
    let lines: Vec<&str> = found.iter().map(|p| p.split(':').next().unwrap()).collect();
    assert_eq!(lines, vec!["1", "18", "19"], "{found:#?}");
    let invalid = parse(&text).unwrap_err();
    assert_eq!(
        invalid.name.as_deref(),
        Some("Demo"),
        "the name is kept for the listing"
    );
}

// The catalog: built-ins, and the user's files in a real directory.

fn data_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let folder = user_dir(dir);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn with_no_folder_there_are_just_the_built_ins() {
    let dir = data_dir();
    let listed = catalog(dir.path());
    assert_eq!(listed.len(), BUILT_IN.len());
    assert!(listed
        .iter()
        .all(|e| e.source == Source::BuiltIn && e.runnable()));
    assert_eq!(
        find(dir.path(), "code-review").unwrap().name,
        "Request code review"
    );
}

#[test]
fn a_users_files_are_listed_beside_the_built_ins_by_name() {
    let dir = data_dir();
    write(
        dir.path(),
        "demo.yaml",
        &BASE.replace("name: Demo", "name: Another"),
    );
    write(dir.path(), "notes.txt", "not a workflow");
    std::fs::create_dir_all(user_dir(dir.path()).join("nested.yaml")).unwrap();
    let listed = catalog(dir.path());
    let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Address reviews",
            "Address reviews elsewhere",
            "Another",
            "Fix failing checks",
            "Request code review",
            "Request code review elsewhere",
            "Review the branch",
            "Smoke test, then tell the agent",
            "Software factory",
        ]
    );
    assert!(matches!(
        &listed[2].source,
        Source::File { replaces_built_in: false, path } if path.ends_with("demo.yaml")
    ));
}

#[test]
fn a_file_with_a_built_ins_id_replaces_it_and_removing_it_brings_it_back() {
    let dir = data_dir();
    let path = write(
        dir.path(),
        "my-review.yml",
        &BASE
            .replace("id: demo", "id: code-review")
            .replace("name: Demo", "name: Mine"),
    );
    let listed = catalog(dir.path());
    assert_eq!(listed.len(), BUILT_IN.len(), "one code-review, not two");
    let mine = listed.iter().find(|e| e.id == "code-review").unwrap();
    assert_eq!(mine.name, "Mine");
    assert!(matches!(
        mine.source,
        Source::File {
            replaces_built_in: true,
            ..
        }
    ));
    assert_eq!(find(dir.path(), "code-review").unwrap().name, "Mine");
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        find(dir.path(), "code-review").unwrap().source,
        Source::BuiltIn
    );
}

#[test]
fn a_broken_replacement_shows_its_problems_rather_than_running_the_built_in() {
    let dir = data_dir();
    write(dir.path(), "code-review.yaml", "id: [\n");
    let found = find(dir.path(), "code-review").unwrap();
    assert!(!found.runnable());
    assert!(!found.problems.is_empty());
    assert!(matches!(
        found.source,
        Source::File {
            replaces_built_in: true,
            ..
        }
    ));
    assert_eq!(catalog(dir.path()).len(), BUILT_IN.len());
}

#[test]
fn two_files_with_one_id_run_the_first_and_flag_the_second() {
    let dir = data_dir();
    let first = write(dir.path(), "a.yaml", BASE);
    write(
        dir.path(),
        "b.yaml",
        &BASE.replace("name: Demo", "name: Also demo"),
    );
    let listed = catalog(dir.path());
    let demos: Vec<&Entry> = listed.iter().filter(|e| e.id == "demo").collect();
    assert_eq!(demos.len(), 2, "both are listed, so the clash can be seen");
    let flagged = demos.iter().find(|e| e.name == "Also demo").unwrap();
    assert!(!flagged.runnable());
    assert!(flagged.problems[0]
        .message
        .contains(&first.display().to_string()));
    let found = find(dir.path(), "demo").unwrap();
    assert_eq!(found.name, "Demo");
    assert!(found.runnable());
}

#[test]
fn a_file_that_is_too_big_or_not_text_is_listed_with_why() {
    let dir = data_dir();
    let big = "#".repeat(MAX_FILE_BYTES as usize + 1);
    write(dir.path(), "big.yaml", &big);
    let folder = user_dir(dir.path());
    std::fs::write(folder.join("binary.yaml"), [0xff, 0xfe, 0x00]).unwrap();
    let listed = catalog(dir.path());
    let big = listed.iter().find(|e| e.id == "big").unwrap();
    assert!(
        big.problems[0].message.contains("at most"),
        "{:?}",
        big.problems
    );
    let binary = listed.iter().find(|e| e.id == "binary").unwrap();
    assert!(
        binary.problems[0].message.contains("not UTF-8"),
        "{:?}",
        binary.problems
    );
}

#[test]
fn copying_a_built_in_makes_the_file_that_replaces_it() {
    let dir = data_dir();
    let path = copy(dir.path(), "code-review", None).unwrap();
    assert_eq!(path, user_dir(dir.path()).join("code-review.yaml"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("# Copied from Yardsort's built-in `code-review`.\n"),
        "{text}"
    );
    assert!(
        !text.contains("To change it, duplicate it"),
        "the built-in's own note is dropped"
    );
    assert_eq!(
        parse(&text).unwrap(),
        parse(BUILT_IN[0].1).unwrap(),
        "the copy is the same workflow"
    );
    let found = find(dir.path(), "code-review").unwrap();
    assert!(matches!(
        found.source,
        Source::File {
            replaces_built_in: true,
            ..
        }
    ));
    assert!(found.runnable());
}

#[test]
fn a_copy_never_overwrites_and_a_refusal_leaves_the_file_alone() {
    let dir = data_dir();
    let path = copy(dir.path(), "code-review", None).unwrap();
    std::fs::write(&path, "edited by hand\n").unwrap();
    let again = copy(dir.path(), "code-review", None).unwrap_err();
    assert_eq!(again.code, "workflow_exists", "{}", again.message);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited by hand\n");
    // Another file already claiming the id is caught before any file is made.
    write(
        dir.path(),
        "mine.yaml",
        &BASE.replace("id: demo", "id: taken"),
    );
    let clash = copy(dir.path(), "code-review", Some("taken")).unwrap_err();
    assert!(clash.message.contains("mine.yaml"), "{}", clash.message);
    assert!(!user_dir(dir.path()).join("taken.yaml").exists());
}

#[test]
fn a_copy_under_a_new_id_sits_beside_the_original() {
    let dir = data_dir();
    let path = copy(dir.path(), "code-review", Some("careful-review")).unwrap();
    assert!(path.ends_with("careful-review.yaml"));
    assert_eq!(
        parse(&std::fs::read_to_string(&path).unwrap()).unwrap().id,
        "careful-review"
    );
    let ids: Vec<String> = catalog(dir.path()).into_iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), BUILT_IN.len() + 1, "{ids:?}");
    assert_eq!(
        find(dir.path(), "code-review").unwrap().source,
        Source::BuiltIn
    );
}

#[test]
fn a_users_own_file_is_copied_only_under_a_new_id() {
    let dir = data_dir();
    write(dir.path(), "demo.yaml", BASE);
    let same = copy(dir.path(), "demo", None).unwrap_err();
    assert!(
        same.message.contains("already your file"),
        "{}",
        same.message
    );
    let path = copy(dir.path(), "demo", Some("demo-two")).unwrap();
    let copied = std::fs::read_to_string(&path).unwrap();
    assert_eq!(copied, BASE.replace("id: demo", "id: demo-two"));
    let bad = copy(dir.path(), "demo", Some("Demo Two")).unwrap_err();
    assert_eq!(bad.code, "workflow_invalid_id");
    assert_eq!(
        copy(dir.path(), "nope", None).unwrap_err().code,
        "workflow_not_found"
    );
}

#[test]
fn a_new_workflow_is_saved_under_its_id_and_never_over_anything() {
    let dir = data_dir();
    let path = save(dir.path(), None, BASE).unwrap();
    assert_eq!(path, user_dir(dir.path()).join("demo.yaml"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), BASE);
    let again = save(dir.path(), None, BASE).unwrap_err();
    assert_eq!(again.code, "workflow_exists");
    let nameless = save(dir.path(), None, "name: No id\n").unwrap_err();
    assert_eq!(nameless.code, "workflow_invalid_id");
}

#[test]
fn an_edit_replaces_its_own_file_even_with_problems_and_only_its_own() {
    let dir = data_dir();
    let path = write(dir.path(), "mine.yml", BASE);
    let broken = BASE.replace("prompt: Hello", "prompt: \"{{ nope }}\"");
    assert_eq!(save(dir.path(), Some(&path), &broken).unwrap(), path);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        broken,
        "saved as written"
    );
    let listed = find(dir.path(), "demo").unwrap();
    assert!(!listed.runnable(), "listed with its problem");
    assert!(
        !user_dir(dir.path()).join("mine.yaml.saving").exists(),
        "the temporary file is gone"
    );

    let elsewhere = dir.path().join("settings.toml");
    std::fs::write(&elsewhere, "keep me").unwrap();
    let refused = save(dir.path(), Some(&elsewhere), BASE).unwrap_err();
    assert_eq!(refused.code, "workflow_not_yours");
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "keep me");
    let refused = remove(dir.path(), &elsewhere).unwrap_err();
    assert_eq!(refused.code, "workflow_not_yours");
    assert!(elsewhere.exists());
}

#[test]
fn an_id_another_file_has_is_refused() {
    let dir = data_dir();
    write(dir.path(), "a.yaml", BASE);
    let b = write(dir.path(), "b.yaml", &BASE.replace("id: demo", "id: other"));
    let error = save(dir.path(), Some(&b), BASE).unwrap_err();
    assert_eq!(error.code, "workflow_exists");
    assert!(error.message.contains("a.yaml"), "{}", error.message);
    assert!(
        std::fs::read_to_string(&b).unwrap().contains("id: other"),
        "unchanged"
    );
}

#[test]
fn removing_a_replacement_brings_the_built_in_back() {
    let dir = data_dir();
    let path = copy(dir.path(), "code-review", None).unwrap();
    assert!(matches!(
        find(dir.path(), "code-review").unwrap().source,
        Source::File { .. }
    ));
    remove(dir.path(), &path).unwrap();
    assert!(!path.exists());
    assert_eq!(
        find(dir.path(), "code-review").unwrap().source,
        Source::BuiltIn
    );
}

#[test]
fn chart_source_lines_follow_yaml_ids_not_lookalikes_in_prompts() {
    let text = "id: same\ninputs:\n  - id: same\nsteps:\n  - action: notify\n    'id': \"same\"\n    body: |\n      - id: decoy\n  - { action: notify, id: second }\n";
    assert_eq!(
        super::step_lines(text),
        std::collections::BTreeMap::from([("same".into(), 6), ("second".into(), 9)])
    );
    let edited = format!("# comment\n{text}");
    assert_eq!(super::step_lines(&edited)["same"], 7);
    assert!(super::step_lines("steps: [").is_empty());
    assert!(super::step_lines("steps: [{id: same}, {id: same}]").is_empty());
}

fn handoff(steps: &str) -> String {
    format!("id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n{steps}")
}

#[test]
fn a_step_hands_a_named_artifact_to_one_that_names_it() {
    let workflow = parse(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n  - id: plan\n    action: notify\n    \
         needs: [draft]\n    consumes: [spec]\n    title: \"Read {{ artifacts.spec }}\"\n",
    ))
    .unwrap();
    assert!(workflow.steps[0].consumes.is_empty());
    assert_eq!(workflow.steps[0].produces.len(), 1);
    assert_eq!(workflow.steps[0].produces[0].name, "spec");
    assert_eq!(
        workflow.steps[0].produces[0].source,
        super::ArtifactSource::File {
            path: "SPEC.md".into()
        }
    );
    assert_eq!(workflow.steps[1].consumes, ["spec"]);
    assert!(problems(BASE).is_empty());
    assert!(workflow
        .steps
        .iter()
        .all(|step| step.produces.is_empty() || step.id == "draft"));
}

#[test]
fn an_artifact_name_has_to_be_produced_by_a_step_this_one_needs() {
    let unknown = problem(&handoff(
        "  - id: plan\n    action: notify\n    consumes: [plan]\n    title: Next\n",
    ));
    assert!(unknown.contains("no artifact `plan`"), "{unknown}");

    let unneeded = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n  - id: plan\n    action: notify\n    \
         consumes: [spec]\n    title: Next\n",
    ));
    assert!(unneeded.contains("not sure to have run"), "{unneeded}");

    let twice = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: A.md\n      - name: spec\n        path: B.md\n",
    ));
    assert!(twice.contains("listed twice"), "{twice}");

    let shared = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: A.md\n  - id: other\n    action: notify\n    \
         title: Other\n    produces:\n      - name: spec\n        path: B.md\n",
    ));
    assert!(shared.contains("already produces `spec`"), "{shared}");

    let again = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n  - id: plan\n    action: notify\n    \
         needs: [draft]\n    consumes: [spec, spec]\n    title: Next\n",
    ));
    assert!(again.contains("listed twice"), "{again}");

    let itself = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n    consumes: [spec]\n",
    ));
    assert!(itself.contains("produces itself"), "{itself}");

    let bare = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n  - id: plan\n    action: notify\n    \
         needs: [draft]\n    title: \"{{ artifacts.spec }}\"\n",
    ));
    assert!(
        bare.contains("does not list `spec` under `consumes`"),
        "{bare}"
    );
}

#[test]
fn an_artifact_source_is_checked_before_the_run() {
    let secret = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: .env\n",
    ));
    assert!(secret.contains("looks like a secret"), "{secret}");

    let outside = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: ../secret.md\n",
    ));
    assert!(outside.contains("outside the workspace"), "{outside}");

    let raw = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n        raw: true\n",
    ));
    assert!(raw.contains("`raw` is only for a transcript"), "{raw}");

    let field = problem(&handoff(
        "  - id: draft\n    action: notify\n    title: Drafted\n    produces:\n      \
         - name: report\n        field: summary\n",
    ));
    assert!(field.contains("leaves nothing"), "{field}");

    let bare = problem(&handoff(
        "  - id: wait\n    action: wait_session\n    session: origin\n    produces:\n      \
         - name: discussion\n        session: origin\n",
    ));
    assert!(
        bare.contains("only when the step says `from: transcript`"),
        "{bare}"
    );

    let raw_only = problem(&handoff(
        "  - id: wait\n    action: wait_session\n    session: origin\n    produces:\n      \
         - name: discussion\n        raw: true\n",
    ));
    assert!(
        raw_only.contains("only when the step says `from: transcript`"),
        "{raw_only}"
    );

    let early = problem(&handoff(
        "  - id: start\n    action: start_session\n    harness: claude\n    produces:\n      \
         - name: spec\n        path: SPEC.md\n",
    ));
    assert!(
        early.contains("cannot hand on a file or a transcript"),
        "{early}"
    );

    let sent = problem(&handoff(
        "  - id: nudge\n    action: send_to_session\n    session: origin\n    prompt: Go\n    \
         produces:\n      - name: discussion\n        from: transcript\n",
    ));
    assert!(
        sent.contains("cannot hand on a file or a transcript"),
        "{sent}"
    );

    let workflow = parse(&handoff(
        "  - id: wait\n    action: wait_session\n    session: origin\n    produces:\n      \
         - name: discussion\n        from: transcript\n",
    ))
    .unwrap();
    assert_eq!(
        workflow.steps[0].produces[0].source,
        super::ArtifactSource::Transcript {
            session: super::SessionRef::Origin,
            raw: false,
        }
    );
}

const BRANCHED: &str = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: plan
    action: notify
    title: Plan
    consumes: [changes-sign-off]
  - id: sign-off
    needs: [plan]
    action: approve
    prompt: Look at the plan.
    on:
      approved: implement
      rejected: stop
      changes_requested: plan
    max_attempts: 3
  - id: implement
    needs: [sign-off]
    action: notify
    title: Build
  - id: verify
    needs: [implement]
    action: run_test
    test: login
    gate: true
    on:
      passed: ship
      failed: implement
    max_attempts: 3
  - id: ship
    needs: [verify]
    action: notify
    title: Ship
"#;

#[test]
fn routes_name_every_outcome_and_a_loop_names_its_limit() {
    let workflow = parse(BRANCHED).unwrap();
    let sign = workflow
        .steps
        .iter()
        .find(|step| step.id == "sign-off")
        .unwrap();
    assert_eq!(sign.max_attempts, Some(3));
    assert_eq!(sign.routes.len(), 3);
    assert!(sign
        .routes
        .iter()
        .any(|route| route.when == OutcomeName::Approved && route.to == "implement"));
    assert!(sign
        .routes
        .iter()
        .any(|route| route.stops() && route.when == OutcomeName::Rejected));
    let verify = workflow
        .steps
        .iter()
        .find(|step| step.id == "verify")
        .unwrap();
    assert_eq!(
        verify
            .routes
            .iter()
            .map(|route| route.when)
            .collect::<Vec<_>>(),
        vec![OutcomeName::Failed, OutcomeName::Passed]
    );
}

#[test]
fn a_branch_to_a_missing_step_is_refused() {
    let found = problem(&BRANCHED.replace("approved: implement", "approved: missing"));
    assert!(found.contains("There is no step `missing`"), "{found}");
}

#[test]
fn a_step_no_branch_can_reach_is_refused() {
    let found = problem(
        "id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: sign-off\n    \
         action: approve\n    prompt: Look\n    on:\n      approved: implement\n      \
         rejected: stop\n      changes_requested: stop\n  - id: implement\n    needs: [sign-off]\n    \
         action: notify\n    title: Build\n  - id: leftover\n    needs: [sign-off]\n    \
         action: notify\n    title: No\n",
    );
    assert!(found.contains("Step `leftover` cannot run"), "{found}");
}

#[test]
fn a_loop_without_a_limit_is_refused() {
    let found = problem(&BRANCHED.replacen(
        "      changes_requested: plan\n    max_attempts: 3\n",
        "      changes_requested: plan\n",
        1,
    ));
    assert!(
        found.contains("Routing back needs `max_attempts`"),
        "{found}"
    );
}

#[test]
fn a_limit_above_the_cap_or_without_a_loop_is_refused() {
    let high = problem(&BRANCHED.replacen("max_attempts: 3", "max_attempts: 11", 1));
    assert!(
        high.contains("`max_attempts` must be from 2 to 10"),
        "{high}"
    );
    let idle = problem(
        "id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: sign-off\n    \
         action: approve\n    prompt: Look\n    max_attempts: 3\n",
    );
    assert!(
        idle.contains("`max_attempts` is set, and no route comes back"),
        "{idle}"
    );
}

#[test]
fn a_branch_must_name_every_outcome_and_only_those() {
    let missing = problem(&BRANCHED.replace("      rejected: stop\n", ""));
    assert!(missing.contains("Say where `rejected` goes"), "{missing}");
    let odd = problems(&BRANCHED.replace("rejected: stop", "nope: stop"));
    assert!(
        odd.iter()
            .any(|line| line.contains("`nope` is not an outcome")),
        "{odd:?}"
    );
    let notify = problem(
        "id: demo\nname: Demo\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: ping\n    \
         action: notify\n    title: Hi\n    on:\n      passed: stop\n",
    );
    assert!(
        notify.contains("`on` is not something a `notify` step takes"),
        "{notify}"
    );
}

#[test]
fn a_forward_failure_may_be_consumed() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: verify
    action: run_test
    test: login
    on:
      passed: stop
      failed: report
  - id: report
    needs: [verify]
    action: notify
    title: Report
    consumes: [failure-verify]
"#;
    let workflow = parse(source).unwrap();
    assert_eq!(
        workflow.steps[1].consumes,
        vec!["failure-verify".to_owned()]
    );
    let refused = problems(&source.replace("failed: report", "failed: stop"));
    assert!(
        refused.iter().any(|line| line.contains(
            "Step `verify` does not route `failed` back to `report`, so `failure-verify` is never written"
        )),
        "{refused:?}"
    );
}

#[test]
fn feedback_is_only_consumed_when_that_outcome_comes_back() {
    let found = problems(&BRANCHED.replacen(
        "      changes_requested: plan\n    max_attempts: 3\n",
        "      changes_requested: stop\n",
        1,
    ));
    assert!(
        found
            .iter()
            .any(|line| line.contains("does not route `changes_requested` back to `plan`")),
        "{found:?}"
    );
    let reserved = problem(&with_steps(
        "  - id: sign-off\n    action: approve\n    produces:\n      - name: failure-sign-off\n        text: notes\n",
    ));
    assert!(
        reserved.contains("`failure-sign-off` is reserved"),
        "{reserved}"
    );
}

#[test]
fn a_clarifying_wait_names_its_rounds_and_a_brief_comes_only_from_it() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: claude
    prompt: Look
  - id: discover
    needs: [start]
    action: wait_session
    session: "{{ steps.start.session }}"
    clarify: true
    produces:
      - name: brief
        from: brief
"#;
    let workflow = parse(source).unwrap();
    match &workflow.steps[1].action {
        Action::WaitSession {
            clarify,
            max_rounds,
            ..
        } => {
            assert!(*clarify);
            assert_eq!(*max_rounds, 3);
        }
        other => panic!("expected a wait, got {other:?}"),
    }
    assert!(matches!(
        workflow.steps[1].produces[0].source,
        ArtifactSource::Brief
    ));
    let limited = source.replace("clarify: true", "clarify: true\n    max_rounds: 2");
    match &parse(&limited).unwrap().steps[1].action {
        Action::WaitSession { max_rounds, .. } => assert_eq!(*max_rounds, 2),
        other => panic!("expected a wait, got {other:?}"),
    }
    let bare = problem(&source.replace("    clarify: true\n", ""));
    assert!(
        bare.contains("A brief comes from a `wait_session`"),
        "{bare}"
    );
    let extra = problem(&with_steps(
        "  - id: discover\n    needs: [start]\n    action: wait_session\n    \
         session: \"{{ steps.start.session }}\"\n    max_rounds: 4\n",
    ));
    assert!(
        extra.contains("`max_rounds` needs `clarify: true`"),
        "{extra}"
    );
    let high = problem(&source.replace("clarify: true", "clarify: true\n    max_rounds: 11"));
    assert!(high.contains("`max_rounds` must be from 1 to 10"), "{high}");
}

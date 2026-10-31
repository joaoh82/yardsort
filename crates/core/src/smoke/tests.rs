use super::*;

/// A valid file, for the tests to break one thing at a time.
const BASE: &str = r#"id: demo
name: Demo
version: 1
inputs:
  - id: user
    kind: text
    default: demo
steps:
  - id: version
    action: run
    program: node
    args: [--version]
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
    let test = parse(BASE).unwrap();
    assert_eq!(test.id, "demo");
    assert_eq!(
        test.inputs[0].label, "user",
        "the label falls back to the id"
    );
    assert_eq!(test.timeout_secs, DEFAULT_RUN_TIMEOUT_SECS);
    assert!(test.setup.is_empty() && test.teardown.is_empty());
    assert_eq!(
        test.steps[0].action,
        Action::Run {
            program: "node".into(),
            args: vec!["--version".into()],
            cwd: None,
            env: Default::default(),
            stdin: None,
            expect: ProcessExpect::default(),
            timeout_secs: None,
        }
    );
}

#[test]
fn the_example_is_valid_and_uses_every_phase() {
    let test = parse(EXAMPLE).unwrap_or_else(|e| panic!("{:#?}", e.problems));
    assert_eq!(test.id, "health");
    assert_eq!(test.setup.len(), 2);
    assert_eq!(test.steps.len(), 4);
    assert!(matches!(test.setup[0].action, Action::Start { .. }));
    assert_eq!(
        test.setup[1].action,
        Action::WaitFor {
            target: WaitTarget::Url {
                url: "http://localhost:{{ run.port }}/health".into()
            },
            interval_secs: 1,
            timeout_secs: Some(60),
        }
    );
    let Action::Http { expect, .. } = &test.steps[0].action else {
        panic!("an http step");
    };
    assert_eq!(expect.status, StatusExpect::Exact(200));
    assert_eq!(expect.json.as_deref(), Some(r#"{"status":"ok"}"#));
    let Action::Browser { actions, .. } = &test.steps[2].action else {
        panic!("a browser step");
    };
    assert_eq!(
        actions[1],
        BrowserAction::ExpectText {
            text: "{{ inputs.greeting }}".into()
        }
    );
    assert!(matches!(test.steps[3].action, Action::Manual { .. }));
}

/// The guide is what an agent is told. Every YAML block in it is a whole file, and every one
/// of them must be a valid test, so the two cannot drift.
#[test]
fn every_example_in_the_guide_is_valid() {
    let mut blocks = 0;
    let mut rest = GUIDE;
    while let Some(start) = rest.find("```yaml\n") {
        let body = &rest[start + "```yaml\n".len()..];
        let end = body.find("\n```").expect("a closed block");
        let block = &body[..end];
        blocks += 1;
        if let Err(invalid) = parse(block) {
            panic!(
                "guide block {blocks} is not valid:\n{block}\n{:#?}",
                invalid.problems
            );
        }
        rest = &body[end..];
    }
    assert!(blocks >= 3, "the guide should show whole files: {blocks}");
    assert!(GUIDE.contains("ys test validate"));
}

#[test]
fn the_parsed_form_is_serialised_the_way_the_app_will_read_it() {
    let test = parse(EXAMPLE).unwrap();
    let json = serde_json::to_value(&test).unwrap();
    assert_eq!(json["setup"][0]["action"], "start");
    assert_eq!(json["setup"][1]["target"]["kind"], "url");
    assert_eq!(json["steps"][0]["expect"]["status"]["kind"], "exact");
    assert_eq!(json["steps"][0]["expect"]["status"]["value"], 200);
    assert_eq!(json["steps"][1]["expect"]["exit"]["kind"], "code");
    assert_eq!(json["steps"][2]["actions"][0]["kind"], "goto");
    assert_eq!(json["steps"][2]["screenshots"], "on_failure");
    assert_eq!(json["timeoutSecs"], 1800);
}

// Shape and types: serde's pass, with the position kept and its wording tidied.

#[test]
fn an_unknown_key_is_named_at_its_line() {
    let text = BASE.replace("version: 1", "version: 1\noops: true");
    let found = problem(&text);
    assert!(found.starts_with("4:1:"), "{found}");
    assert!(found.contains("oops"), "{found}");
}

#[test]
fn a_missing_key_is_reported() {
    let found = problem(&BASE.replace("name: Demo\n", ""));
    assert!(found.contains("name"), "{found}");
}

#[test]
fn a_file_from_a_later_version_says_so_instead_of_blaming_its_new_keys() {
    let text = BASE.replace("version: 1", "version: 2\nretries: 3");
    assert_eq!(
        problem(&text),
        "3:10: This file is for a later Yardsort (smoke test version 2); this one reads version \
         1. Update Yardsort to run it."
    );
}

#[test]
fn an_unparseable_file_still_gives_its_name_when_it_can() {
    let text = "id: demo\nname: Demo\nversion: 1\nsteps: 3\n";
    let invalid = parse(text).unwrap_err();
    assert_eq!(invalid.id.as_deref(), Some("demo"));
    assert_eq!(invalid.name.as_deref(), Some("Demo"));
    assert_eq!(invalid.problems.len(), 1);
}

// Names.

#[test]
fn the_id_is_a_file_name() {
    for bad in ["Login", "my test", "a/b", "-x"] {
        let found = problem(&BASE.replace("id: demo", &format!("id: {bad:?}")));
        assert!(found.starts_with("1:5:"), "{bad}: {found}");
        assert!(found.contains("cannot be an id"), "{bad}: {found}");
    }
}

#[test]
fn the_version_must_be_this_one() {
    assert!(problem(&BASE.replace("version: 1", "version: 0")).contains("`version` is 1"));
}

#[test]
fn a_test_has_steps() {
    let found = problem("id: demo\nname: Demo\nversion: 1\nsteps: []\n");
    assert!(found.contains("at least one step"), "{found}");
}

#[test]
fn step_ids_are_unique_across_setup_steps_and_teardown() {
    let text = format!("{BASE}teardown:\n  - id: version\n    action: run\n    program: echo\n");
    let found = problem(&text);
    assert!(
        found.contains("already a step `version`, on line 9"),
        "{found}"
    );
}

#[test]
fn a_harness_input_is_refused_with_the_reason() {
    let found = problem(&BASE.replace("kind: text", "kind: harness"));
    assert!(found.contains("starts no agent"), "{found}");
}

#[test]
fn a_choice_needs_options_and_the_default_must_be_one() {
    let found = problem(&BASE.replace("kind: text", "kind: choice"));
    assert!(found.contains("needs `options`"), "{found}");
    let text = BASE.replace(
        "kind: text\n    default: demo",
        "kind: choice\n    options: [a, b]\n    default: c",
    );
    assert!(problem(&text).contains("not one of the options"));
}

#[test]
fn the_run_timeout_is_a_duration() {
    let text = BASE.replace("version: 1", "version: 1\ntimeout: soon");
    assert!(problem(&text).contains("not a timeout"));
    let test = parse(&BASE.replace("version: 1", "version: 1\ntimeout: 2h")).unwrap();
    assert_eq!(test.timeout_secs, 7200);
}

// Actions and their keys.

#[test]
fn an_unknown_action_lists_the_real_ones() {
    let found = problem(&BASE.replace("action: run", "action: exec"));
    assert!(found.contains("`exec` is not an action"), "{found}");
    assert!(
        found.contains("`wait_for`") && found.contains("`manual`"),
        "{found}"
    );
}

#[test]
fn a_key_on_the_wrong_action_is_named_as_misplaced() {
    let found = problem(&with_steps(
        "  - id: ask\n    action: manual\n    prompt: Look\n    timeout: 5m\n",
    ));
    assert!(
        found.contains("`timeout` is not something a `manual` step takes"),
        "{found}"
    );
}

#[test]
fn each_action_names_what_it_cannot_do_without() {
    for (step, needs) in [
        ("action: run\n", "`program`"),
        ("action: shell\n", "`line`"),
        ("action: start\n", "`program`"),
        ("action: wait_for\n", "one thing"),
        ("action: http\n", "the request"),
        ("action: browser\n", "`do`"),
        ("action: manual\n", "`prompt`"),
    ] {
        let found = problem(&with_steps(&format!("  - id: x\n    {step}")));
        assert!(found.contains(needs), "{step}: {found}");
    }
}

#[test]
fn a_shell_line_cannot_hold_a_variable() {
    let found = problem(&with_steps(
        "  - id: curl\n    action: shell\n    line: curl localhost:{{ run.port }} | jq .\n",
    ));
    assert!(found.starts_with("15:11:"), "{found}");
    assert!(found.contains("$YS_RUN_PORT"), "{found}");
    let ok = with_steps(
        "  - id: curl\n    action: shell\n    line: curl localhost:$YS_RUN_PORT | jq .\n",
    );
    assert!(problems(&ok).is_empty(), "{:?}", problems(&ok));
}

#[test]
fn cwd_stays_inside_the_workspace() {
    for bad in ["../other", "/tmp", "{{ run.dir }}", "a/../../b"] {
        let found = problem(&with_steps(&format!(
            "  - id: x\n    action: run\n    program: ls\n    cwd: {bad:?}\n"
        )));
        assert!(found.contains("inside the workspace"), "{bad}: {found}");
    }
    assert!(problems(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    cwd: packages/api\n"
    ))
    .is_empty());
}

#[test]
fn an_environment_name_is_checked() {
    let found = problem(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    env:\n      1BAD: yes\n",
    ));
    assert!(
        found.contains("`1BAD` is not an environment variable name"),
        "{found}"
    );
}

#[test]
fn exit_is_a_code_or_nonzero() {
    let found = problem(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    expect:\n      exit: bad\n",
    ));
    assert!(found.contains("Use a number, or `nonzero`"), "{found}");
    let test = parse(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    expect:\n      exit: nonzero\n",
    ))
    .unwrap();
    let Action::Run { expect, .. } = &test.steps[1].action else {
        panic!()
    };
    assert_eq!(expect.exit, ExitExpect::Nonzero);
}

#[test]
fn stdout_json_keeps_the_type_of_the_expected_value() {
    let equals = |value: &str| {
        let test = parse(&with_steps(&format!(
            "  - id: x\n    action: run\n    program: ls\n    expect:\n      stdout_json: {{ path: count, equals: {value} }}\n"
        )))
        .unwrap();
        let Action::Run { expect, .. } = &test.steps[1].action else {
            panic!()
        };
        expect.stdout_json.clone().unwrap().equals
    };
    assert_eq!(equals("1"), "1");
    assert_eq!(equals("'1'"), "\"1\"");
    assert_eq!(equals("ok"), "\"ok\"");
    assert_eq!(equals("true"), "true");
    assert_eq!(equals("[1, 2]"), "[1,2]");
    assert_eq!(equals("\"{{ inputs.user }}\""), "\"{{ inputs.user }}\"");
    let found = problem(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    expect:\n      stdout_json: { path: count, equals: \"{{ inputs.nope }}\" }\n",
    ));
    assert!(found.contains("no input `nope`"), "{found}");
}

#[test]
fn an_http_expectation_on_a_run_step_is_named_as_such() {
    let found = problem(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    expect:\n      status: 200\n",
    ));
    assert!(
        found.contains("`status` is an expectation for an `http` step"),
        "{found}"
    );
    let found = problem(&with_steps(
        "  - id: x\n    action: http\n    get: http://localhost/\n    expect:\n      exit: 0\n",
    ));
    assert!(
        found.contains("`exit` is an expectation for a `run` or `shell` step"),
        "{found}"
    );
}

#[test]
fn a_regular_expression_must_compile() {
    let found = problem(&with_steps(
        "  - id: x\n    action: run\n    program: ls\n    expect:\n      stdout_matches: '('\n",
    ));
    assert!(found.contains("not a regular expression"), "{found}");
}

#[test]
fn wait_for_waits_for_exactly_one_thing() {
    let found = problem(&with_steps(
        "  - id: x\n    action: wait_for\n    port: 80\n    url: http://localhost/\n",
    ));
    assert!(
        found.contains("`url` is a second thing to wait for"),
        "{found}"
    );
    for target in ["port: 80", "url: http://localhost/", "file: ready.txt"] {
        let found = problem(&with_steps(&format!(
            "  - id: x\n    action: wait_for\n    {target}\n    args: [\"{{{{ inputs.nope }}}}\"]\n"
        )));
        assert!(
            found.contains("`args` goes with `program`"),
            "{target}: {found}"
        );
    }
    let found = problem(&with_steps(
        "  - id: x\n    action: wait_for\n    program: test\n    args: [-f, \"{{ inputs.nope }}\"]\n",
    ));
    assert!(found.contains("no input `nope`"), "{found}");
    let found = problem(&with_steps(
        "  - id: x\n    action: wait_for\n    port: 70000\n",
    ));
    assert!(found.contains("not a port"), "{found}");
    let found = problem(&with_steps(
        "  - id: x\n    action: wait_for\n    port: eighty\n",
    ));
    assert!(found.contains("not a port"), "{found}");
    let test = parse(&with_steps(
        "  - id: x\n    action: wait_for\n    port: \"{{ run.port }}\"\n    interval: 2s\n",
    ))
    .unwrap();
    assert_eq!(
        test.steps[1].action,
        Action::WaitFor {
            target: WaitTarget::Port {
                port: "{{ run.port }}".into()
            },
            interval_secs: 2,
            timeout_secs: None,
        }
    );
}

#[test]
fn an_http_step_makes_one_request_with_one_body() {
    let found = problem(&with_steps(
        "  - id: x\n    action: http\n    get: http://a/\n    post: http://b/\n",
    ));
    assert!(found.contains("`get` and `post` are two"), "{found}");
    let found = problem(&with_steps(
        "  - id: x\n    action: http\n    post: http://a/\n    body: hi\n    json: { a: 1 }\n",
    ));
    assert!(found.contains("Not both"), "{found}");
    let found = problem(&with_steps(
        "  - id: x\n    action: http\n    get: http://a/\n    expect:\n      status: 7xx\n",
    ));
    assert!(found.contains("not a status"), "{found}");
    let test = parse(&with_steps(
        "  - id: x\n    action: http\n    get: http://a/\n    expect:\n      status: 4xx\n",
    ))
    .unwrap();
    let Action::Http { expect, .. } = &test.steps[1].action else {
        panic!()
    };
    assert_eq!(expect.status, StatusExpect::Class(4));
}

#[test]
fn browser_actions_are_one_key_each_from_the_vocabulary_with_the_right_shape() {
    let step = |item: &str| {
        with_steps(&format!(
            "  - id: x\n    action: browser\n    do:\n      - {item}\n"
        ))
    };
    let found = problem(&step("clack: \"#go\""));
    assert!(
        found.contains("`clack` is not a browser action. Did you mean `click`?"),
        "{found}"
    );
    let found = problem(&step("fill: \"#name\""));
    assert!(
        found.contains("`fill` takes `fill: { selector: \"…\", text: \"…\" }`"),
        "{found}"
    );
    let found = problem(&step("fill: { selector: \"#name\", value: x }"));
    assert!(found.contains("`fill` has no `value`"), "{found}");
    let found = problem(&step("fill: { selector: \"\", text: hi }"));
    assert!(found.contains("`fill` needs `selector`"), "{found}");
    let found = problem(&step("wait_for: { text: \"\" }"));
    assert!(found.contains("`wait_for` needs `text`"), "{found}");
    // Empty data is data: clearing a field, choosing the blank option.
    let test = parse(&step(
        "fill: { selector: \"#name\", text: \"\" }\n      - select: { selector: \"#plan\", value: \"\" }",
    ))
    .unwrap();
    let Action::Browser { actions, .. } = &test.steps[1].action else {
        panic!()
    };
    assert_eq!(
        actions[0],
        BrowserAction::Fill {
            selector: "#name".into(),
            text: String::new()
        }
    );
    assert_eq!(
        actions[1],
        BrowserAction::Select {
            selector: "#plan".into(),
            value: String::new()
        }
    );
    let found = problem(&step("goto: \"\""));
    assert!(found.contains("`goto` takes an address"), "{found}");
    let found = problem(&step("screenshots: never\n      - goto: http://a/"));
    assert!(
        found.contains("`screenshots` is not a browser action"),
        "{found}"
    );
    let found = problem(&with_steps("  - id: x\n    action: browser\n    do: []\n"));
    assert!(found.contains("`do` is empty"), "{found}");
    let found = problem(&with_steps(
        "  - id: x\n    action: browser\n    do: [goto: http://a/]\n    viewport: wide\n",
    ));
    assert!(found.contains("not a viewport"), "{found}");
    let test = parse(&with_steps(
        "  - id: x\n    action: browser\n    viewport: 1280x800\n    do:\n      - wait_for: { text: Ready }\n      - wait_for: \"#app\"\n      - press: Enter\n",
    ))
    .unwrap();
    let Action::Browser {
        actions, viewport, ..
    } = &test.steps[1].action
    else {
        panic!()
    };
    assert_eq!(
        viewport,
        &Some(Viewport {
            width: 1280,
            height: 800
        })
    );
    assert_eq!(
        actions[0],
        BrowserAction::WaitFor {
            selector: None,
            text: Some("Ready".into())
        }
    );
    assert_eq!(
        actions[1],
        BrowserAction::WaitFor {
            selector: Some("#app".into()),
            text: None
        }
    );
}

// Variables.

#[test]
fn every_variable_names_something_that_exists() {
    let cases = [
        ("{{ workspace.brnch }}", "`workspace` has no `brnch`"),
        ("{{ workspace }}", "needs a field"),
        (
            "{{ inputs.usr }}",
            "there is no input `usr`. Did you mean `user`?",
        ),
        ("{{ env.bad-name }}", "not an environment variable name"),
        ("{{ steps.nope.exit }}", "there is no step `nope`"),
        (
            "{{ steps.version.status }}",
            "a `run` step leaves `exit`, `stdout`, `stderr`",
        ),
        ("{{ pr.number }}", "there is no `pr`"),
        ("{{ memory }}", "there is no `memory`"),
    ];
    for (var, expected) in cases {
        let found = problem(&with_steps(&format!(
            "  - id: x\n    action: run\n    program: echo\n    args: [\"{var}\"]\n"
        )));
        assert!(found.contains(expected), "{var}: {found}");
        assert!(found.starts_with("16:13:"), "{var}: {found}");
    }
    let ok = with_steps(
        "  - id: x\n    action: run\n    program: echo\n    args: [\"{{ run.port }} {{ env.HOME }} {{ steps.version.stdout }} {{ inputs.user }}\"]\n",
    );
    assert!(problems(&ok).is_empty(), "{:?}", problems(&ok));
}

#[test]
fn a_step_result_must_come_from_an_earlier_step() {
    let text = "id: demo\nname: Demo\nversion: 1\nsetup:\n  - id: early\n    action: run\n    program: echo\n    args: [\"{{ steps.late.exit }}\"]\nsteps:\n  - id: late\n    action: run\n    program: echo\n    args: [\"{{ steps.early.exit }}\"]\n";
    let found = problem(text);
    assert!(found.contains("used before step `late` has run"), "{found}");
}

#[test]
fn a_malformed_placeholder_is_reported_where_it_opens() {
    let found = problem(&with_steps(
        "  - id: x\n    action: manual\n    prompt: |\n      Check it.\n      Then {{ oops\n",
    ));
    assert!(found.starts_with("17:12:"), "{found}");
    assert!(found.contains("never closed"), "{found}");
}

#[test]
fn a_variable_in_a_browser_action_is_reported_on_its_line() {
    let found = problem(&with_steps(
        "  - id: x\n    action: browser\n    do:\n      - goto: http://a/\n      - fill: { selector: \"#u\", text: \"{{ inputs.who }}\" }\n",
    ));
    assert!(found.starts_with("17:40:"), "{found}");
    assert!(found.contains("no input `who`"), "{found}");
}

// The catalog.

fn worktree() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write(worktree: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    let folder = dir(worktree);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_worktree_without_tests_has_an_empty_catalog() {
    let tree = worktree();
    assert!(catalog(tree.path()).is_empty());
}

#[test]
fn the_catalog_lists_every_file_sorted_by_name_with_its_problems() {
    let tree = worktree();
    write(
        tree.path(),
        "zeta.yaml",
        &BASE
            .replace("id: demo", "id: zeta")
            .replace("name: Demo", "name: Alpha"),
    );
    write(
        tree.path(),
        "broken.yml",
        &BASE.replace("action: run", "action: exec"),
    );
    write(tree.path(), "notes.txt", "not a test");
    let listed = catalog(tree.path());
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "zeta");
    assert_eq!(listed[0].name, "Alpha");
    assert!(listed[0].runnable());
    assert_eq!(listed[1].id, "demo", "a broken file keeps the id it says");
    assert!(!listed[1].runnable());
    assert_eq!(listed[1].problems.len(), 1);
    assert!(find(tree.path(), "zeta").is_some());
    assert!(find(tree.path(), "demo").is_some_and(|e| !e.runnable()));
    assert!(find(tree.path(), "nope").is_none());
}

#[test]
fn a_file_that_is_not_yaml_takes_the_id_of_its_name() {
    let tree = worktree();
    write(tree.path(), "login.yaml", "steps: [");
    let listed = catalog(tree.path());
    assert_eq!(listed[0].id, "login");
    assert_eq!(listed[0].name, "login");
    assert!(!listed[0].runnable());
}

#[test]
fn two_files_with_one_id_use_the_first_by_name_and_report_the_second() {
    let tree = worktree();
    let first = write(tree.path(), "a.yaml", BASE);
    write(tree.path(), "b.yaml", BASE);
    let listed = catalog(tree.path());
    assert_eq!(listed.len(), 2);
    let second = listed.iter().find(|e| e.path.ends_with("b.yaml")).unwrap();
    assert!(!second.runnable());
    assert!(
        second.problems[0]
            .message
            .contains(&first.display().to_string()),
        "{}",
        second.problems[0]
    );
    assert!(find(tree.path(), "demo").unwrap().path.ends_with("a.yaml"));
}

#[test]
fn a_file_too_big_or_not_text_is_refused_with_the_reason() {
    let tree = worktree();
    let folder = dir(tree.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("big.yaml"),
        vec![b'#'; MAX_FILE_BYTES as usize + 1],
    )
    .unwrap();
    std::fs::write(folder.join("bytes.yaml"), [0xff, 0xfe, 0x00]).unwrap();
    let listed = catalog(tree.path());
    let big = listed.iter().find(|e| e.id == "big").unwrap();
    assert!(
        big.problems[0].message.contains("at most"),
        "{}",
        big.problems[0]
    );
    let bytes = listed.iter().find(|e| e.id == "bytes").unwrap();
    assert!(
        bytes.problems[0].message.contains("UTF-8"),
        "{}",
        bytes.problems[0]
    );
}

#[test]
fn an_entry_knows_whether_it_needs_a_browser() {
    let tree = worktree();
    write(tree.path(), "plain.yaml", BASE);
    write(tree.path(), "health.yaml", EXAMPLE);
    assert!(!find(tree.path(), "demo").unwrap().uses_browser());
    assert!(find(tree.path(), "health").unwrap().uses_browser());
}

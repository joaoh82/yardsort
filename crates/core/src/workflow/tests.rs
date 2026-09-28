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
        }
    );
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
            "`{{ pr }}` needs a field: `pr.number`, `pr.url`, `pr.title`.",
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
    assert_eq!(names, vec!["Another", "Request code review"]);
    assert!(matches!(
        &listed[0].source,
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
    assert_eq!(listed.len(), 1, "one code-review, not two");
    assert_eq!(listed[0].name, "Mine");
    assert!(matches!(
        listed[0].source,
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
    assert_eq!(catalog(dir.path()).len(), 1);
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
    assert_eq!(ids.len(), 2, "{ids:?}");
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

//! A test in words: one line per step, for the person who decides whether it runs. The
//! approval card is the sandbox, so what it says has to be the whole of what the step does,
//! with every variable left as written — `{{ run.port }}` is a port the reader cannot know yet,
//! and `{{ env.TEST_PASSWORD }}` is a secret the card must not print.

use serde::Serialize;
use specta::Type;

use super::exec::duration_words;
use super::{
    Action, BrowserAction, ExitExpect, HttpExpect, Phase, ProcessExpect, StatusExpect, Step, Test,
    WaitTarget,
};

/// One step, as the card lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StepWords {
    pub id: String,
    /// `setup`, `steps` or `teardown`.
    pub phase: String,
    /// The action's name in the file.
    pub action: String,
    /// What the step does, in a sentence.
    pub words: String,
}

/// Every step of a test, in the order it runs.
pub fn test_words(test: &Test) -> Vec<StepWords> {
    test.all_steps()
        .map(|(phase, step)| StepWords {
            id: step.id.clone(),
            phase: match phase {
                Phase::Setup => "setup",
                Phase::Steps => "steps",
                Phase::Teardown => "teardown",
            }
            .to_owned(),
            action: step.action.name().to_owned(),
            words: step_words(step),
        })
        .collect()
}

/// What one step does.
pub fn step_words(step: &Step) -> String {
    match &step.action {
        Action::Run {
            program,
            args,
            cwd,
            env,
            stdin,
            expect,
            timeout_secs,
        } => {
            let mut words = format!("run `{}`", command_line(program, args));
            words.push_str(&place(cwd.as_deref(), env));
            if let Some(stdin) = stdin {
                words.push_str(&feeding(stdin));
            }
            words.push_str(&expecting(&process_expectations(expect)));
            words.push_str(&within(*timeout_secs));
            words
        }
        Action::Shell {
            line,
            cwd,
            env,
            expect,
            timeout_secs,
        } => {
            let mut words = format!("shell `{line}`");
            words.push_str(&place(cwd.as_deref(), env));
            words.push_str(&expecting(&process_expectations(expect)));
            words.push_str(&within(*timeout_secs));
            words
        }
        Action::Start {
            program,
            args,
            cwd,
            env,
        } => {
            let mut words = format!(
                "start `{}` and keep it running until the run ends",
                command_line(program, args)
            );
            words.push_str(&place(cwd.as_deref(), env));
            words
        }
        Action::WaitFor {
            target,
            interval_secs,
            timeout_secs,
        } => {
            let mut words = match target {
                WaitTarget::Port { port } => format!("wait until port {port} answers"),
                WaitTarget::Url { url } => format!("wait until {url} answers 2xx"),
                WaitTarget::File { file } => format!("wait until the file {file} exists"),
                WaitTarget::Command { program, args } => {
                    format!("wait until `{}` exits 0", command_line(program, args))
                }
            };
            if *interval_secs != 1 {
                words.push_str(&format!(
                    ", looking every {}",
                    duration_words(u64::from(*interval_secs))
                ));
            }
            words.push_str(&within(*timeout_secs));
            words
        }
        Action::Http {
            method,
            url,
            headers,
            body,
            json,
            expect,
            timeout_secs,
        } => {
            let mut words = format!("{} {url}", method.name().to_uppercase());
            if !headers.is_empty() {
                words.push_str(&format!(
                    ", with the header{} {}",
                    plural(headers.len()),
                    headers.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if let Some(json) = json {
                words.push_str(&format!(", with the JSON body `{}`", json.trim()));
            } else if let Some(body) = body {
                words.push_str(&format!(", with the body `{}`", body.trim()));
            }
            words.push_str(&expecting(&http_expectations(expect)));
            words.push_str(&within(*timeout_secs));
            words
        }
        Action::Browser {
            actions,
            headed,
            timeout_secs,
            ..
        } => {
            let mut words = if *headed {
                "in a browser window: ".to_owned()
            } else {
                "in a headless browser: ".to_owned()
            };
            words.push_str(
                &actions
                    .iter()
                    .map(browser_action)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            words.push_str(&within(*timeout_secs));
            words
        }
        Action::Manual { prompt } => format!("ask you: {}", prompt.trim()),
    }
}

/// `bun run seed --user "{{ inputs.user }}"`: the program and its arguments as one line, each
/// argument that has a space or nothing in it quoted, so the words say where one ends.
pub fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quoted)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quoted(arg: &str) -> String {
    if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

fn place(cwd: Option<&str>, env: &std::collections::BTreeMap<String, String>) -> String {
    let mut words = String::new();
    if let Some(cwd) = cwd {
        words.push_str(&format!(" in {cwd}"));
    }
    if !env.is_empty() {
        // The value is part of what the step does — a literal shows, a `{{ env.X }}` stays a
        // template, so a secret is still named and never printed.
        words.push_str(&format!(
            ", with {} set",
            env.iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    words
}

/// What a `run` step sends on the process's standard input, shown whole: this is the script an
/// interpreter like `sh` or `psql` runs, so the card has to disclose it, not just say it exists.
fn feeding(stdin: &str) -> String {
    format!(", with `{}` on its input", stdin.trim_end())
}

fn expecting(expectations: &[String]) -> String {
    if expectations.is_empty() {
        String::new()
    } else {
        format!("; expect {}", expectations.join(", "))
    }
}

fn within(timeout_secs: Option<u32>) -> String {
    match timeout_secs {
        Some(secs) => format!(", within {}", duration_words(u64::from(secs))),
        None => String::new(),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn process_expectations(expect: &ProcessExpect) -> Vec<String> {
    let mut out = vec![match expect.exit {
        ExitExpect::Code(code) => format!("exit {code}"),
        ExitExpect::Nonzero => "a nonzero exit".to_owned(),
    }];
    if let Some(text) = &expect.stdout_contains {
        out.push(format!("stdout containing {text:?}"));
    }
    if let Some(regex) = &expect.stdout_matches {
        out.push(format!("stdout matching /{regex}/"));
    }
    if expect.stderr_empty {
        out.push("nothing on stderr".to_owned());
    }
    if let Some(json) = &expect.stdout_json {
        out.push(format!("stdout JSON with {} = {}", json.path, json.equals));
    }
    out
}

fn http_expectations(expect: &HttpExpect) -> Vec<String> {
    let mut out = vec![match expect.status {
        StatusExpect::Exact(code) => format!("status {code}"),
        StatusExpect::Class(class) => format!("status {class}xx"),
    }];
    if let Some(json) = &expect.json {
        out.push(format!("a JSON body with {json}"));
    }
    if let Some(text) = &expect.body_contains {
        out.push(format!("a body containing {text:?}"));
    }
    if let Some(regex) = &expect.body_matches {
        out.push(format!("a body matching /{regex}/"));
    }
    if let Some(header) = &expect.header {
        out.push(format!(
            "the header {} containing {:?}",
            header.name, header.contains
        ));
    }
    out
}

fn browser_action(action: &BrowserAction) -> String {
    match action {
        BrowserAction::Goto { url } => format!("go to {url}"),
        BrowserAction::Click { selector } => format!("click {selector}"),
        BrowserAction::Fill { selector, text } => format!("fill {selector} with {text:?}"),
        BrowserAction::Press { key } => format!("press {key}"),
        BrowserAction::Select { selector, value } => format!("select {value:?} in {selector}"),
        BrowserAction::WaitFor { selector, text } => match (selector, text) {
            (Some(selector), _) => format!("wait for {selector}"),
            (None, Some(text)) => format!("wait for the text {text:?}"),
            (None, None) => "wait".to_owned(),
        },
        BrowserAction::ExpectText { text } => format!("expect the text {text:?}"),
        BrowserAction::ExpectVisible { selector } => format!("expect {selector} visible"),
        BrowserAction::ExpectHidden { selector } => format!("expect {selector} hidden"),
        BrowserAction::ExpectUrl { pattern } => format!("expect the address to match {pattern}"),
        BrowserAction::ExpectTitle { title } => format!("expect the title {title:?}"),
        BrowserAction::Screenshot { name } => format!("take a screenshot named {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smoke::parse;

    fn words_of(file: &str) -> Vec<StepWords> {
        let test = parse(file).unwrap_or_else(|problems| panic!("{problems:?}"));
        test_words(&test)
    }

    #[test]
    fn every_step_of_the_example_is_a_sentence_with_its_variables_left_as_written() {
        let words = words_of(crate::smoke::EXAMPLE);
        let by_id: std::collections::BTreeMap<&str, &StepWords> =
            words.iter().map(|w| (w.id.as_str(), w)).collect();
        assert_eq!(words.first().map(|w| w.phase.as_str()), Some("setup"));
        assert!(words.iter().any(|w| w.phase == "steps"));
        for step in &words {
            assert!(!step.words.is_empty(), "{} has words", step.id);
        }
        // The example's server, request and question, each in the words the card shows.
        let server = &by_id["server"];
        assert_eq!(server.action, "start");
        assert!(
            server.words.starts_with("start `") && server.words.contains("{{ run.port }}"),
            "{}",
            server.words
        );
        let health = &by_id["health"];
        assert!(
            health.words.starts_with("GET http://") && health.words.contains("; expect status"),
            "{}",
            health.words
        );
        let manual = words
            .iter()
            .find(|w| w.action == "manual")
            .expect("the example asks a person something");
        assert!(manual.words.starts_with("ask you: "), "{}", manual.words);
    }

    #[test]
    fn expectations_places_and_limits_are_all_said() {
        let words = words_of(
            "id: t\nname: T\nversion: 1\nsteps:\n\
             - id: seed\n  action: run\n  program: bun\n  args: [run, seed, --user, \"a b\", \"\"]\n  cwd: api\n\
             \x20 env: { DATABASE_URL: \"{{ env.TEST_DATABASE_URL }}\" }\n  stdin: hi\n  timeout: 90s\n\
             \x20 expect: { exit: nonzero, stdout_contains: seeded, stderr_empty: true, stdout_json: { path: n, equals: 1 } }\n\
             - id: ready\n  action: wait_for\n  port: \"{{ run.port }}\"\n  interval: 2s\n  timeout: 1m\n\
             - id: api\n  action: http\n  post: http://localhost:{{ run.port }}/x\n  headers: { Authorization: x }\n  json: { a: 1 }\n\
             \x20 expect: { status: 201, json: { ok: true }, body_contains: done, header: { name: content-type, contains: json } }\n\
             - id: look\n  action: browser\n  do:\n    - goto: http://localhost:{{ run.port }}/\n    - fill: { selector: \"input[name=q]\", text: hi }\n    - click: \"text=Go\"\n    - expect_url: \"**/done\"\n    - screenshot: done\n",
        );
        assert_eq!(
            words[0].words,
            "run `bun run seed --user \"a b\" \"\"` in api, with \
             DATABASE_URL={{ env.TEST_DATABASE_URL }} set, with `hi` on its input; expect a \
             nonzero exit, stdout containing \"seeded\", nothing on stderr, stdout JSON with \
             n = 1, within 1m 30s"
        );
        assert_eq!(
            words[1].words,
            "wait until port {{ run.port }} answers, looking every 2s, within 1m 0s"
        );
        assert_eq!(
            words[2].words,
            "POST http://localhost:{{ run.port }}/x, with the header Authorization, with the JSON \
             body `{\"a\":1}`; expect status 201, a JSON body with {\"ok\":true}, a body \
             containing \"done\", the header content-type containing \"json\""
        );
        assert_eq!(
            words[3].words,
            "in a headless browser: go to http://localhost:{{ run.port }}/, fill input[name=q] \
             with \"hi\", click text=Go, expect the address to match **/done, take a screenshot \
             named done"
        );
    }

    #[test]
    fn a_secret_is_named_not_printed() {
        // The value of an environment name is not in the file; the card shows the name and the
        // template, never what it stands for.
        let words = words_of(
            "id: t\nname: T\nversion: 1\nsteps:\n\
             - id: s\n  action: shell\n  line: echo $YS_ENV_TOKEN\n  env: { TOKEN: \"{{ env.TOKEN }}\" }\n",
        );
        assert_eq!(
            words[0].words,
            "shell `echo $YS_ENV_TOKEN`, with TOKEN={{ env.TOKEN }} set; expect exit 0"
        );
    }

    #[test]
    fn a_script_sent_to_an_interpreter_is_shown_whole() {
        // The card is the only chance to read a script before it runs, so what `sh` is fed has
        // to be in the words, not hidden behind "with text on its input".
        let words = words_of(
            "id: t\nname: T\nversion: 1\nsteps:\n\
             - id: s\n  action: run\n  program: sh\n  stdin: |\n    rm -rf /tmp/x\n    echo done\n",
        );
        assert_eq!(
            words[0].words,
            "run `sh`, with `rm -rf /tmp/x\necho done` on its input; expect exit 0"
        );
    }

    #[test]
    fn an_http_body_is_shown_whole() {
        let words = words_of(
            "id: t\nname: T\nversion: 1\nsteps:\n\
             - id: s\n  action: http\n  post: http://localhost/x\n  body: \"DROP TABLE users\"\n",
        );
        assert_eq!(
            words[0].words,
            "POST http://localhost/x, with the body `DROP TABLE users`; expect status 2xx"
        );
    }
}

//! The Floor Manager's permission, enforced here rather than trusted to the agent.
//!
//! The launch sets [`yardsort_core::coordinator::ENV`]. While it is set, a command is allowed,
//! refused, or held until the app records a yes, according to the level in settings. The level
//! is read from the file on every command, so changing it in Settings applies to the next one.
//! A shell that is not the Floor Manager does not have the variable, and is not gated: this is
//! the user's own `ys`.

use std::path::PathBuf;

use yardsort_core::coordinator::{self, Action, Decision, FloorManagerLevel};

use crate::commands::{
    activity, event, memory, pr, project, schedule, session, task, test, trigger, workflow,
    workspace,
};
use crate::{Command, Failure, Yardsort};

/// Refuse or confirm before the command runs. A read is not touched, and neither is a command
/// whose environment is not the Floor Manager's.
pub(crate) fn enforce(data_dir: &Option<PathBuf>, command: &mut Command) -> Result<(), Failure> {
    if std::env::var_os(coordinator::ENV).is_none() {
        return Ok(());
    }
    let action = effect(command);
    if action == Action::Read {
        return Ok(());
    }
    let line = command_line();
    // Destroying is refused at every level, so it does not need the settings file: a missing
    // or unreadable one must not let a delete through.
    if action == Action::Deny {
        return match coordinator::decide(FloorManagerLevel::Ask, action, &line) {
            Decision::Refuse(message) => Err(Failure::new(message)),
            // `decide` refuses `Action::Deny` at every level. The other two arms exist so a
            // future change that lets one through is a panic in tests, not a quiet delete.
            Decision::Allow | Decision::Confirm => {
                unreachable!("a denied command is refused at every level")
            }
        };
    }
    let ys = Yardsort::open(data_dir.clone())?;
    // A file we could not read must not fall through to the default, which is ask first: that
    // would quietly loosen read only. A missing file is not this case.
    if let Some(problem) = &ys.settings_problem {
        return Err(Failure::new(format!(
            "The Floor Manager's settings could not be read, so this was treated as read only \
             and was not run: {line}\n{problem}"
        )));
    }
    match coordinator::decide(ys.settings.floor_manager.level, action, &line) {
        Decision::Allow => Ok(()),
        Decision::Refuse(message) => Err(Failure::new(message)),
        Decision::Confirm => {
            let shown = confirmation_text(command, &line)?;
            coordinator::ask(&ys.data_dir, &shown).map_err(Failure::new)
        }
    }
}

/// What ask first puts in the dialog. A comment, or a workflow file, read from a file is part of
/// the command the user is allowing, so the text is in the request. `-` is refused: standard
/// input is not knowable until it is consumed, and consuming it would leave the command with
/// nothing to post or save.
///
/// The text is read once, here, and the command then carries that string. Posting or saving
/// reads the file again otherwise, and a change between the dialog and the write would send
/// something other than what was allowed.
fn confirmation_text(command: &mut Command, line: &str) -> Result<String, Failure> {
    let Some(file) = body_file(command) else {
        return Ok(line.to_owned());
    };
    let workflow = matches!(command, Command::Workflow(workflow::Command::Save { .. }));
    if file == "-" {
        return Err(Failure::new(if workflow {
            "Not run: ys workflow save - would save whatever is on standard input, and ask \
             first would show only the dash. Write the file somewhere and name it, so the \
             confirmation shows the workflow that would be saved."
        } else {
            "Not run: --body-file - would post whatever is on standard input, and ask first \
             would show only the flag. Pass the text as an argument, or name a file, so the \
             confirmation shows what would be posted."
        }));
    }
    let text = std::fs::read_to_string(&file).map_err(|error| {
        Failure::new(format!(
            "Not run: the text in {file} could not be read, so nothing was {}: {error}",
            if workflow { "saved" } else { "posted" }
        ))
    })?;
    // A workflow decides what is sent to agents, so the dialog shows all of it: the snapshot
    // that would be saved, not a start and a pointer to a file that may change. One too big to
    // save is refused here, before anybody is asked.
    if workflow && text.len() as u64 > yardsort_core::workflow::MAX_FILE_BYTES {
        return Err(Failure::new(format!(
            "Not run: {file} is larger than a workflow file may be ({} bytes), so it would not \
             be saved.",
            yardsort_core::workflow::MAX_FILE_BYTES
        )));
    }
    remember_body(command, text.clone());
    let (shown, cut) = if workflow {
        (show_text(&text), false)
    } else {
        preview_body(&text)
    };
    let heading = if workflow {
        "The workflow that would be saved:"
    } else {
        "The text that would be posted:"
    };
    let mut out = format!("{line}\n\n{heading}\n{shown}");
    if cut {
        out.push_str("\n\nThe rest is in the file.");
    }
    Ok(out)
}

/// Keep the text that was shown, in place of the file it was read from.
fn remember_body(command: &mut Command, text: String) {
    match command {
        Command::Task(task::Command::Create {
            body, body_file, ..
        }) => {
            *body = Some(text);
            *body_file = None;
        }
        Command::Task(task::Command::Comment {
            text: slot,
            body_file,
            ..
        }) => {
            *slot = Some(text);
            *body_file = None;
        }
        Command::Workflow(workflow::Command::Save { allowed, .. }) => *allowed = Some(text),
        _ => {}
    }
}

/// The file whose text the confirmation must show. A workflow path that is not UTF-8 is still
/// one: spelled lossily it cannot be read, so the command is refused rather than shown bare.
fn body_file(command: &Command) -> Option<String> {
    match command {
        Command::Task(task::Command::Create { body_file, .. })
        | Command::Task(task::Command::Comment { body_file, .. }) => body_file.clone(),
        Command::Workflow(workflow::Command::Save { file, .. }) => {
            Some(file.to_string_lossy().into_owned())
        }
        _ => None,
    }
}

/// How much of a comment the dialog shows. The rest stays in the file, and the request says so.
const BODY_PREVIEW: usize = 8 * 1024;

/// The comment, with control characters and bidi overrides escaped, and newlines kept so it
/// can be read. Quotes and backslashes stay as they are: this is the text that would be
/// posted, not a shell argument. A bidi override in this text can reorder the buttons that
/// follow it.
fn preview_body(text: &str) -> (String, bool) {
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if next > BODY_PREVIEW {
            break;
        }
        end = next;
    }
    (show_text(&text[..end]), end < text.len())
}

fn show_text(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        if is_hidden(character) && character != '\n' && character != '\t' {
            out.push_str(&format!("\\u{{{:04x}}}", character as u32));
        } else {
            out.push(character);
        }
    }
    out
}

/// What this command does. Exhaustive on purpose: a new subcommand has to be named as a read,
/// a start or write, or something the Floor Manager may never do.
fn effect(command: &Command) -> Action {
    match command {
        Command::Doctor | Command::Logs { .. } => Action::Read,
        // Attaching puts another session's terminal on this one, which is typing into it.
        Command::Attach { .. } => Action::Deny,
        Command::Activity(command) => match command {
            activity::Command::List { .. } | activity::Command::Export { .. } => Action::Read,
        },
        Command::Project(command) => match command {
            project::Command::List => Action::Read,
            project::Command::Add { .. } => Action::Act,
        },
        Command::Workspace(command) => match command {
            workspace::Command::List { .. }
            | workspace::Command::Handoff { .. }
            | workspace::Command::Context { .. } => Action::Read,
            // `--print` writes the message and stops. Nothing is created.
            workspace::Command::Fork { print: true, .. } => Action::Read,
            workspace::Command::Delete { .. } => Action::Deny,
            workspace::Command::New { .. }
            | workspace::Command::Fork { .. }
            | workspace::Command::Done { .. } => Action::Act,
        },
        Command::Session(command) => match command {
            session::Command::List { .. } => Action::Read,
            session::Command::Resume { .. } | session::Command::Fork { .. } => Action::Act,
        },
        Command::Memory(command) => match command {
            memory::Command::List { .. } | memory::Command::Search { .. } => Action::Read,
            memory::Command::Propose { .. } => Action::Act,
        },
        Command::Task(command) => match command {
            task::Command::List { .. } | task::Command::Show { .. } => Action::Read,
            task::Command::Start { print: true, .. } => Action::Read,
            // Without `--yes` it says what it would do and stops. That is a look.
            task::Command::Close { yes: false, .. } | task::Command::Reopen { yes: false, .. } => {
                Action::Read
            }
            task::Command::Close { yes: true, .. } => Action::Deny,
            task::Command::Create { .. }
            | task::Command::Comment { .. }
            | task::Command::Edit { .. }
            | task::Command::Reopen { .. }
            | task::Command::Start { .. } => Action::Act,
        },
        Command::Pr(command) => match command {
            pr::Command::List { .. } | pr::Command::Tracked { .. } => Action::Read,
            pr::Command::Checkout { .. }
            | pr::Command::Track { .. }
            | pr::Command::Untrack { .. } => Action::Act,
        },
        Command::Workflow(command) => match command {
            workflow::Command::List
            | workflow::Command::Show { .. }
            | workflow::Command::Runs { .. }
            | workflow::Command::Validate { .. } => Action::Read,
            workflow::Command::Copy { .. }
            | workflow::Command::Save { .. }
            | workflow::Command::Run { .. }
            | workflow::Command::Cancel { .. }
            | workflow::Command::Continue { .. }
            | workflow::Command::Ask { .. }
            | workflow::Command::Summary { .. } => Action::Act,
            workflow::Command::Approve { .. }
            | workflow::Command::Reject { .. }
            | workflow::Command::RequestChanges { .. }
            | workflow::Command::Answer { .. } => Action::Deny,
        },
        Command::Test(command) => match command {
            test::Command::List { .. }
            | test::Command::Show { .. }
            | test::Command::Validate { .. }
            | test::Command::Example
            | test::Command::Guide
            | test::Command::Wait { .. }
            | test::Command::Runs { .. }
            | test::Command::Log { .. } => Action::Read,
            test::Command::Run { .. } | test::Command::Cancel { .. } => Action::Act,
        },
        Command::Schedule(command) => match command {
            schedule::Command::List
            | schedule::Command::Show { .. }
            | schedule::Command::Runs { .. } => Action::Read,
            schedule::Command::Add(_)
            | schedule::Command::Pause { .. }
            | schedule::Command::Resume { .. }
            | schedule::Command::Run { .. }
            | schedule::Command::Remove { .. } => Action::Act,
        },
        Command::Trigger(command) => match command {
            trigger::Command::List
            | trigger::Command::Presets
            | trigger::Command::Show { .. }
            | trigger::Command::Fires { .. } => Action::Read,
            trigger::Command::Add(_)
            | trigger::Command::Pause { .. }
            | trigger::Command::Resume { .. }
            | trigger::Command::Reset { .. }
            | trigger::Command::Remove { .. } => Action::Act,
        },
        Command::Event(command) => match command {
            event::Command::Emit { .. } => Action::Act,
        },
    }
}

/// The command line as a person reads it. The program's own path is `ys`, so the dialog shows
/// the command rather than wherever this binary lives.
fn command_line() -> String {
    let mut args = std::env::args();
    let _program = args.next();
    std::iter::once("ys".to_owned())
        .chain(args.map(|arg| quote(&arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(arg: &str) -> String {
    let needs_quotes = arg.is_empty() || arg.chars().any(must_quote);
    if !needs_quotes {
        return arg.to_owned();
    }
    let mut out = String::from("\"");
    for character in arg.chars() {
        push_visible(character, &mut out, true);
    }
    out.push('"');
    out
}

/// A character that would split the command line, or change how the dialog reads it.
fn must_quote(character: char) -> bool {
    character.is_whitespace()
        || matches!(character, '"' | '\\' | '\'' | '$' | '`')
        || is_hidden(character)
}

/// Newlines, other controls, and bidi overrides. Left in a command line, a newline looks like
/// a second command and a bidi override can reverse the text of the dialog.
fn is_hidden(character: char) -> bool {
    let code = character as u32;
    (character.is_control() && character != '\t')
        || (0x202A..=0x202E).contains(&code)
        || (0x2066..=0x2069).contains(&code)
        || matches!(code, 0x200E | 0x200F | 0x061C)
}

/// Write `character` so it cannot change the surrounding text. Inside a quoted argument,
/// newline, return and tab become the two-character escapes. Elsewhere a newline or tab is
/// kept by the caller.
fn push_visible(character: char, out: &mut String, quoted: bool) {
    match character {
        '\\' | '"' => {
            out.push('\\');
            out.push(character);
        }
        '\n' if quoted => out.push_str("\\n"),
        '\r' if quoted => out.push_str("\\r"),
        '\t' if quoted => out.push_str("\\t"),
        other if is_hidden(other) || (quoted && other.is_control()) => {
            out.push_str(&format!("\\u{{{:04x}}}", other as u32));
        }
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{quote, show_text};

    #[test]
    fn a_command_line_cannot_hide_a_second_command_or_reorder_the_dialog() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("has space"), "\"has space\"");
        let newline = quote("one\ntwo");
        assert!(!newline.contains('\n'), "{newline}");
        assert!(newline.contains("\\n"), "{newline}");
        let bidi = quote("ok\u{202e}evil");
        assert!(!bidi.contains('\u{202e}'), "{bidi}");
        assert!(bidi.contains("\\u{202e}"), "{bidi}");
        assert_eq!(quote("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }

    #[test]
    fn a_comment_preview_keeps_quotes_and_backslashes_and_escapes_what_can_reorder_it() {
        assert_eq!(show_text("say \"hi\" \\ there"), "say \"hi\" \\ there");
        assert_eq!(show_text("one\ntwo\tend"), "one\ntwo\tend");
        let bidi = show_text("ok\u{202e}evil");
        assert!(!bidi.contains('\u{202e}'), "{bidi}");
        assert!(bidi.contains("\\u{202e}"), "{bidi}");
        assert!(!show_text("a\rb").contains('\r'));
    }
}

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
pub(crate) fn enforce(data_dir: &Option<PathBuf>, command: &Command) -> Result<(), Failure> {
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
    match coordinator::decide(ys.settings.floor_manager.level, action, &line) {
        Decision::Allow => Ok(()),
        Decision::Refuse(message) => Err(Failure::new(message)),
        Decision::Confirm => coordinator::ask(&ys.data_dir, &line).map_err(Failure::new),
    }
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
            | workflow::Command::Run { .. }
            | workflow::Command::Cancel { .. } => Action::Act,
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
    if arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '\'' | '$' | '`'))
    {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

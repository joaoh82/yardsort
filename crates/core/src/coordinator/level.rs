//! How much the Floor Manager may do, and the instructions that say so.
//!
//! The level lives in settings. `ys` reads it on every command, so a change applies to the next
//! one. The instructions are rewritten when the setting is saved and again at each start; an
//! agent that has already read them picks up the new wording the next time it starts.

use serde::{Deserialize, Serialize};
use specta::Type;

/// How much the Floor Manager may do through `ys` on its own.
///
/// Destructive commands are refused at every level. See [`crate::coordinator::Action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Type)]
#[serde(rename_all = "kebab-case")]
pub enum FloorManagerLevel {
    /// Look. Every command that starts something or writes is refused.
    #[serde(alias = "read_only", alias = "readonly")]
    ReadOnly,
    /// Starts and writes wait for a yes in the app. The default.
    #[default]
    Ask,
    /// May start agents, workspaces, workflows, tests and checkouts, and other writes that
    /// are not destructive, without asking.
    Trusted,
}

impl FloorManagerLevel {
    /// The word stored in `settings.toml` and shown to the agent.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::Trusted => "trusted",
        }
    }
}

/// What a Floor Manager's `ys` command does, for the permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Looking. Allowed at every level.
    Read,
    /// Starting or writing. Read only refuses, ask first confirms, trusted allows.
    Act,
    /// Refused at every level: delete, force, close, attach.
    Deny,
}

/// What `ys` should do with a command once it knows the level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Ask first: do not run it until the app records a yes.
    Confirm,
    Refuse(String),
}

/// The instructions for `level`, as written into `AGENTS.md`.
pub fn instructions(level: FloorManagerLevel) -> String {
    let permission = match level {
        FloorManagerLevel::ReadOnly => include_str!("read-only.md"),
        FloorManagerLevel::Ask => include_str!("ask.md"),
        FloorManagerLevel::Trusted => include_str!("trusted.md"),
    };
    format!(
        "{}\n\n{}\n\n{}\n\n{}\n",
        include_str!("AGENTS.md").trim_end(),
        permission.trim(),
        include_str!("commands.md").trim(),
        include_str!("never.md").trim()
    )
}

/// Allow, confirm or refuse `command` at `level`. `command` is the line the message quotes.
pub fn decide(level: FloorManagerLevel, action: Action, command: &str) -> Decision {
    match action {
        Action::Read => Decision::Allow,
        Action::Deny => Decision::Refuse(format!(
            "The Floor Manager cannot do this, at any permission: {command}\n\
             Deleting a workspace (with or without --force), closing a task, and attaching \
             to another terminal are refused here. Merging, approving and editing project \
             files are not ys commands; they stay off limits, and a shell command that is \
             not ys is not this gate."
        )),
        Action::Act => match level {
            FloorManagerLevel::ReadOnly => Decision::Refuse(format!(
                "The Floor Manager is set to read only, so this was not run: {command}\n\
                 Starting or writing needs Floor Manager permission changed in Settings."
            )),
            FloorManagerLevel::Ask => Decision::Confirm,
            FloorManagerLevel::Trusted => Decision::Allow,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_round_trips_through_its_word_and_the_aliases() {
        for level in [
            FloorManagerLevel::ReadOnly,
            FloorManagerLevel::Ask,
            FloorManagerLevel::Trusted,
        ] {
            let parsed: FloorManagerLevel =
                serde_json::from_value(serde_json::json!(level.as_str())).unwrap();
            assert_eq!(parsed, level);
        }
        let parsed: FloorManagerLevel =
            serde_json::from_value(serde_json::json!("read_only")).unwrap();
        assert_eq!(parsed, FloorManagerLevel::ReadOnly);
    }

    #[test]
    fn read_only_refuses_a_start_ask_confirms_trusted_allows_and_destroy_is_always_refused() {
        let command = "ys workspace new demo \"fix it\"";
        assert!(matches!(
            decide(FloorManagerLevel::ReadOnly, Action::Act, command),
            Decision::Refuse(message) if message.contains("read only") && message.contains(command)
        ));
        assert_eq!(
            decide(FloorManagerLevel::Ask, Action::Act, command),
            Decision::Confirm
        );
        assert_eq!(
            decide(FloorManagerLevel::Trusted, Action::Act, command),
            Decision::Allow
        );
        for level in [
            FloorManagerLevel::ReadOnly,
            FloorManagerLevel::Ask,
            FloorManagerLevel::Trusted,
        ] {
            assert!(matches!(
                decide(level, Action::Read, "ys project list"),
                Decision::Allow
            ));
            assert!(matches!(
                decide(level, Action::Deny, "ys workspace delete old"),
                Decision::Refuse(message) if message.contains("at any permission")
            ));
        }
    }

    #[test]
    fn the_instructions_name_the_active_level_and_the_lines_every_level_keeps() {
        let ask = instructions(FloorManagerLevel::Ask);
        let read_only = instructions(FloorManagerLevel::ReadOnly);
        let trusted = instructions(FloorManagerLevel::Trusted);
        for body in [&ask, &read_only, &trusted] {
            for promise in [
                "ys project list",
                "ys pr list",
                "ys pr checkout",
                "ys workflow run code-review",
                "Never",
                "cloud",
                "ys workspace delete",
                "ys task close",
                "ys attach",
                "not a sandbox",
            ] {
                assert!(body.contains(promise), "missing {promise:?} in {body}");
            }
        }
        assert!(ask.contains("## Ask first"));
        assert!(ask.contains("Allow"));
        assert!(!read_only.contains("## Ask first"));
        assert!(read_only.contains("## Read only"));
        assert!(read_only.contains("refuses"));
        assert!(trusted.contains("## Trusted"));
        assert!(trusted.contains("without waiting"));
        assert!(!trusted.contains("## Ask first"));
    }
}

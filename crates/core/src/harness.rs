//! Harnesses: terminal coding agents, described as pure configuration.
//!
//! A definition is a command plus argument templates for the few things Yardsort needs to do
//! with an agent. Adding one must never need code — M4 makes these user-editable; until then the
//! built-ins below are all there is. See `docs/design/04-harnesses.md`.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptTransport {
    /// The prompt is an argument. Simple and reliable.
    Argv,
    /// The prompt is pasted into the terminal once the harness has started and gone quiet. For
    /// harnesses with no prompt argument, and for prompts too long for a command line.
    Stdin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SessionIdMode {
    /// We choose the harness's session id up front, so resume is deterministic.
    Assigned,
    /// The harness picks; we resume "the latest session in this directory", which is safe
    /// because every workspace has a directory of its own.
    LatestInCwd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDef {
    /// Stable key, recorded with sessions.
    pub id: String,
    pub label: String,
    pub command: String,
    /// Always passed.
    pub base_args: Vec<String>,
    pub model_args: Vec<String>,
    pub effort_args: Vec<String>,
    pub session_args: Vec<String>,
    pub prompt_args: Vec<String>,
    /// How to run this agent **non-interactively**, with `{prompt}` for the question: the
    /// print / exec mode every agent has. Yardsort uses it to have the agent write a commit
    /// message or a pull request — see [`crate::draft`]. Empty means this harness cannot, which
    /// is the honest default for one we know nothing about.
    #[serde(default)]
    pub write_args: Vec<String>,
    pub resume_args: Vec<String>,
    pub fork_args: Vec<String>,
    /// Effort levels to offer. Empty hides the picker.
    pub efforts: Vec<String>,
    /// Model suggestions. Free text is always accepted: names change faster than we ship.
    pub models: Vec<String>,
    pub prompt_transport: PromptTransport,
    pub session_id_mode: SessionIdMode,
    /// With the `stdin` transport: how long the harness must have been quiet, after printing
    /// something, before the prompt is pasted.
    pub stdin_ready_ms: u32,
    /// Disabled harnesses stay configured but are not offered.
    pub enabled: bool,
    /// In the user's words, what this harness is good at. Assist uses it to suggest a harness
    /// for a task; harnesses without one are never suggested. Empty for every built-in: which
    /// agent suits which work is the user's call, not ours.
    #[serde(default)]
    pub strengths: String,
}

pub const DEFAULT_STDIN_READY_MS: u32 = 1500;

/// A sentence or two is plenty for Assist to tell harnesses apart.
pub const MAX_STRENGTHS_CHARS: usize = 400;

/// Values for the placeholders. `None` means "not given": the arg group using it is dropped.
#[derive(Debug, Clone, Default)]
pub struct LaunchValues {
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub session_id: Option<String>,
    /// When forking: the id the *new* conversation should get, so it can be resumed later too.
    pub new_session_id: Option<String>,
}

impl HarnessDef {
    /// The argv (without the command) for starting a new session. With the `stdin` transport
    /// the prompt is not an argument, so its group is left out whatever `values` holds.
    pub fn start_args(&self, values: &LaunchValues) -> Vec<String> {
        let mut groups = vec![
            &self.base_args,
            &self.model_args,
            &self.effort_args,
            &self.session_args,
        ];
        if self.prompt_transport == PromptTransport::Argv {
            groups.push(&self.prompt_args);
        }
        groups
            .into_iter()
            .flat_map(|group| expand(group, values))
            .collect()
    }

    /// Whether a fork started with [`Self::continue_args`] gets an id we choose — and so can be
    /// resumed by id afterwards.
    pub fn fork_assigns_id(&self) -> bool {
        self.session_id_mode == SessionIdMode::Assigned
            && self
                .fork_args
                .iter()
                .any(|arg| arg.contains("{new_session_id}"))
    }

    /// The argv for resuming (`fork = false`) or forking a previous session.
    pub fn continue_args(&self, values: &LaunchValues, fork: bool) -> Vec<String> {
        let last = if fork {
            &self.fork_args
        } else {
            &self.resume_args
        };
        [&self.base_args, &self.model_args, &self.effort_args, last]
            .into_iter()
            .flat_map(|group| expand(group, values))
            .collect()
    }

    /// A blank definition for a harness the user is adding.
    pub fn custom(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: id.to_owned(),
            command: id.to_owned(),
            base_args: vec![],
            model_args: vec![],
            effort_args: vec![],
            session_args: vec![],
            prompt_args: strings(&["--", "{prompt}"]),
            write_args: vec![],
            resume_args: vec![],
            fork_args: vec![],
            efforts: vec![],
            models: vec![],
            prompt_transport: PromptTransport::Argv,
            session_id_mode: SessionIdMode::LatestInCwd,
            stdin_ready_ms: DEFAULT_STDIN_READY_MS,
            enabled: true,
            strengths: String::new(),
        }
    }
}

/// Substitute placeholders in one arg group. Each element stays exactly one argument whatever
/// the value contains — spaces, quotes, newlines — because nothing is ever re-split and no shell
/// is involved. A group that needs a value nobody gave is dropped whole, so choosing no model
/// leaves no dangling `--model`.
fn expand(group: &[String], values: &LaunchValues) -> Vec<String> {
    let lookup = |name: &str| match name {
        "prompt" => values.prompt.as_deref(),
        "model" => values.model.as_deref(),
        "effort" => values.effort.as_deref(),
        "session_id" => values.session_id.as_deref(),
        "new_session_id" => values.new_session_id.as_deref(),
        _ => None,
    };
    let mut out = Vec::with_capacity(group.len());
    for template in group {
        let mut arg = String::new();
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            let name = &rest[open + 1..open + close];
            if !is_placeholder(name) {
                // Literal braces, e.g. JSON in an argument: copy through untouched.
                arg.push_str(&rest[..=open + close]);
            } else if let Some(value) = lookup(name).filter(|v| !v.is_empty()) {
                arg.push_str(&rest[..open]);
                arg.push_str(value);
            } else {
                return Vec::new();
            }
            rest = &rest[open + close + 1..];
        }
        arg.push_str(rest);
        out.push(arg);
    }
    out
}

fn is_placeholder(name: &str) -> bool {
    matches!(
        name,
        "prompt" | "model" | "effort" | "session_id" | "new_session_id"
    )
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// The harnesses Yardsort knows out of the box. Flags verified against each CLI's `--help`
/// (versions in `docs/design/04-harnesses.md`); they move, so M4 lets users correct them.
pub fn builtin() -> Vec<HarnessDef> {
    vec![
        HarnessDef {
            id: "claude".into(),
            label: "Claude Code".into(),
            command: "claude".into(),
            base_args: vec![],
            model_args: strings(&["--model", "{model}"]),
            effort_args: strings(&["--effort", "{effort}"]),
            session_args: strings(&["--session-id", "{session_id}"]),
            prompt_args: strings(&["--", "{prompt}"]),
            write_args: strings(&["--print", "{prompt}"]),
            resume_args: strings(&["--resume", "{session_id}"]),
            fork_args: strings(&[
                "--resume",
                "{session_id}",
                "--fork-session",
                "--session-id",
                "{new_session_id}",
            ]),
            efforts: strings(&["low", "medium", "high", "xhigh", "max"]),
            models: strings(&["fable", "opus", "sonnet", "haiku"]),
            prompt_transport: PromptTransport::Argv,
            session_id_mode: SessionIdMode::Assigned,
            stdin_ready_ms: DEFAULT_STDIN_READY_MS,
            enabled: true,
            strengths: String::new(),
        },
        HarnessDef {
            id: "codex".into(),
            label: "Codex".into(),
            command: "codex".into(),
            base_args: vec![],
            model_args: strings(&["-m", "{model}"]),
            effort_args: strings(&["-c", "model_reasoning_effort=\"{effort}\""]),
            session_args: vec![],
            prompt_args: strings(&["--", "{prompt}"]),
            write_args: strings(&["exec", "{prompt}"]),
            resume_args: strings(&["resume", "--last"]),
            fork_args: strings(&["fork", "--last"]),
            efforts: strings(&["low", "medium", "high"]),
            models: vec![],
            prompt_transport: PromptTransport::Argv,
            session_id_mode: SessionIdMode::LatestInCwd,
            stdin_ready_ms: DEFAULT_STDIN_READY_MS,
            enabled: true,
            strengths: String::new(),
        },
        HarnessDef {
            id: "grok".into(),
            label: "Grok".into(),
            command: "grok".into(),
            base_args: vec![],
            model_args: strings(&["-m", "{model}"]),
            effort_args: strings(&["--reasoning-effort", "{effort}"]),
            session_args: strings(&["--session-id", "{session_id}"]),
            prompt_args: strings(&["--", "{prompt}"]),
            write_args: strings(&["--single", "{prompt}"]),
            resume_args: strings(&["--resume", "{session_id}"]),
            fork_args: strings(&[
                "--resume",
                "{session_id}",
                "--fork-session",
                "--session-id",
                "{new_session_id}",
            ]),
            efforts: strings(&["low", "medium", "high"]),
            models: vec![],
            prompt_transport: PromptTransport::Argv,
            session_id_mode: SessionIdMode::Assigned,
            stdin_ready_ms: DEFAULT_STDIN_READY_MS,
            enabled: true,
            strengths: String::new(),
        },
        HarnessDef {
            id: "opencode".into(),
            label: "OpenCode".into(),
            command: "opencode".into(),
            base_args: vec![],
            model_args: strings(&["-m", "{model}"]),
            effort_args: vec![],
            session_args: vec![],
            prompt_args: strings(&["--prompt", "{prompt}"]),
            write_args: strings(&["run", "{prompt}"]),
            resume_args: strings(&["--continue"]),
            fork_args: strings(&["--continue", "--fork"]),
            efforts: vec![],
            models: vec![],
            prompt_transport: PromptTransport::Argv,
            session_id_mode: SessionIdMode::LatestInCwd,
            stdin_ready_ms: DEFAULT_STDIN_READY_MS,
            enabled: true,
            strengths: String::new(),
        },
    ]
}

/// What the settings file says about one harness. For a built-in this is a *partial* override:
/// only the fields the user changed are stored, so corrected defaults in a newer Yardsort
/// still reach everything they left alone — and "restore defaults" is deleting the entry. For a
/// custom harness the entry is the whole definition, missing fields being blank.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessOverride {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_transport: Option<PromptTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id_mode: Option<SessionIdMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin_ready_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strengths: Option<String>,
}

macro_rules! each_field {
    ($apply:ident) => {
        $apply!(
            label,
            command,
            base_args,
            model_args,
            effort_args,
            session_args,
            prompt_args,
            resume_args,
            fork_args,
            efforts,
            models,
            prompt_transport,
            session_id_mode,
            stdin_ready_ms,
            enabled,
            strengths
        )
    };
}

impl HarnessOverride {
    /// `base` with every field this override sets.
    pub fn apply(&self, mut base: HarnessDef) -> HarnessDef {
        macro_rules! set {
            ($($field:ident),*) => { $( if let Some(value) = &self.$field { base.$field = value.clone(); } )* };
        }
        each_field!(set);
        base
    }

    /// The smallest override that turns `base` into `wanted`.
    pub fn between(base: &HarnessDef, wanted: &HarnessDef) -> Self {
        let mut diff = Self {
            id: wanted.id.clone(),
            ..Self::default()
        };
        macro_rules! compare {
            ($($field:ident),*) => { $( if base.$field != wanted.$field { diff.$field = Some(wanted.$field.clone()); } )* };
        }
        each_field!(compare);
        diff
    }

    /// Whether this changes anything at all.
    pub fn is_empty(&self) -> bool {
        *self
            == Self {
                id: self.id.clone(),
                ..Self::default()
            }
    }
}

/// A harness as the app sees it: the definition in force and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub def: HarnessDef,
    pub builtin: bool,
    /// A built-in the user has changed.
    pub modified: bool,
}

/// Built-ins with the user's overrides applied, followed by the user's own harnesses.
pub fn resolve_all(overrides: &[HarnessOverride]) -> Vec<Resolved> {
    let builtins = builtin();
    let mut all: Vec<Resolved> = builtins
        .iter()
        .map(|base| {
            let custom = overrides.iter().find(|o| o.id == base.id);
            Resolved {
                def: custom.map_or_else(|| base.clone(), |o| o.apply(base.clone())),
                builtin: true,
                modified: custom.is_some_and(|o| !o.is_empty()),
            }
        })
        .collect();
    all.extend(
        overrides
            .iter()
            .filter(|o| !builtins.iter().any(|base| base.id == o.id))
            .map(|o| Resolved {
                def: o.apply(HarnessDef::custom(&o.id)),
                builtin: false,
                modified: false,
            }),
    );
    all
}

pub fn find(id: &str, overrides: &[HarnessOverride]) -> Option<HarnessDef> {
    resolve_all(overrides)
        .into_iter()
        .find(|harness| harness.def.id == id)
        .map(|harness| harness.def)
}

/// Check a definition the user wants to save. Returns the message to show them.
pub fn validate(def: &HarnessDef) -> Result<(), String> {
    let id_ok = !def.id.is_empty()
        && def.id.len() <= 40
        && def
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !id_ok {
        return Err("The id may only contain lowercase letters, digits, - and _.".into());
    }
    if def.label.trim().is_empty() {
        return Err("Give the harness a label.".into());
    }
    if def.command.trim().is_empty() {
        return Err("The command cannot be empty.".into());
    }
    if def.session_id_mode == SessionIdMode::Assigned
        && !def
            .session_args
            .iter()
            .any(|arg| arg.contains("{session_id}"))
    {
        return Err("With an assigned session id, the session args must use {session_id}.".into());
    }
    if def.prompt_transport == PromptTransport::Argv
        && !def.prompt_args.iter().any(|arg| arg.contains("{prompt}"))
    {
        return Err("With the argv transport, the prompt args must use {prompt}.".into());
    }
    if def.strengths.chars().count() > MAX_STRENGTHS_CHARS {
        return Err(format!(
            "Keep \"Good at\" under {MAX_STRENGTHS_CHARS} characters."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(prompt: &str, model: &str, effort: &str) -> LaunchValues {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        LaunchValues {
            prompt: some(prompt),
            model: some(model),
            effort: some(effort),
            session_id: Some("11111111-2222-3333-4444-555555555555".into()),
            new_session_id: Some("99999999-8888-7777-6666-555555555555".into()),
        }
    }

    #[test]
    fn claude_gets_model_effort_session_and_prompt() {
        let args = find("claude", &[])
            .unwrap()
            .start_args(&values("fix the bug", "opus", "high"));
        assert_eq!(
            args,
            [
                "--model",
                "opus",
                "--effort",
                "high",
                "--session-id",
                "11111111-2222-3333-4444-555555555555",
                "--",
                "fix the bug",
            ]
        );
    }

    #[test]
    fn groups_without_a_value_vanish_whole() {
        let args = find("claude", &[]).unwrap().start_args(&values("", "", ""));
        assert_eq!(
            args,
            ["--session-id", "11111111-2222-3333-4444-555555555555"]
        );
        assert!(find("opencode", &[])
            .unwrap()
            .start_args(&values("", "", "high"))
            .is_empty());
    }

    /// A first message beginning with `-` is ordinary English — a bullet pasted from a list — but
    /// to an argument parser it is a flag. Every harness that takes the prompt as a bare
    /// positional argument must therefore have `--` in front of it, which is how every parser we
    /// launch is told the options have ended. `claude`, `codex` and `grok` all refused such a
    /// prompt outright before this; clap's own message suggests exactly this fix.
    #[test]
    fn a_prompt_that_starts_with_a_hyphen_is_not_read_as_an_option() {
        let dashed = "- Commit / push / open PR from the UI";
        for harness in builtin() {
            let args = harness.start_args(&values(dashed, "", ""));
            let at = args
                .iter()
                .position(|arg| arg == dashed)
                .unwrap_or_else(|| panic!("{}: the prompt is not in {args:?}", harness.id));
            let before = at
                .checked_sub(1)
                .map(|i| args[i].as_str())
                .unwrap_or_else(|| panic!("{}: nothing shields {args:?}", harness.id));
            assert!(
                before == "--" || before.starts_with("--"),
                "{}: the prompt is a bare positional after {before:?}, so a leading hyphen \
                 reads as an option: {args:?}",
                harness.id
            );
        }
    }

    #[test]
    fn a_prompt_is_always_exactly_one_argument() {
        let nasty = "rename `foo` to \"bar\"; rm -rf $HOME\n--model evil 'quoted' {model}";
        let args = find("opencode", &[])
            .unwrap()
            .start_args(&values(nasty, "", ""));
        assert_eq!(args, ["--prompt", nasty]);
    }

    #[test]
    fn placeholders_substitute_inside_an_argument_and_other_braces_are_literal() {
        let args = find("codex", &[])
            .unwrap()
            .start_args(&values("go", "", "medium"));
        assert_eq!(
            args,
            ["-c", "model_reasoning_effort=\"medium\"", "--", "go"]
        );

        let group = strings(&["--config", "{\"a\":{\"b\":1}}", "--tag={effort}"]);
        assert_eq!(
            expand(&group, &values("", "", "low")),
            ["--config", "{\"a\":{\"b\":1}}", "--tag=low"]
        );
    }

    #[test]
    fn an_override_stores_only_what_changed_and_restoring_is_deleting_it() {
        let base = find("claude", &[]).unwrap();
        let mut wanted = base.clone();
        wanted.command = "/opt/claude-nightly".into();
        wanted.enabled = false;

        let diff = HarnessOverride::between(&base, &wanted);
        assert_eq!(diff.command.as_deref(), Some("/opt/claude-nightly"));
        assert_eq!(diff.enabled, Some(false));
        assert_eq!(
            diff.model_args, None,
            "untouched fields keep following the built-in"
        );
        assert_eq!(diff.apply(base.clone()), wanted);

        let toml = toml::to_string(&diff).unwrap();
        assert!(
            !toml.contains("model_args") && toml.contains("command"),
            "{toml}"
        );
        assert!(HarnessOverride::between(&base, &base).is_empty());
    }

    #[test]
    fn overrides_modify_builtins_and_unknown_ids_become_custom_harnesses() {
        let overrides = vec![
            HarnessOverride {
                id: "codex".into(),
                efforts: Some(strings(&["minimal", "high"])),
                ..Default::default()
            },
            HarnessOverride {
                id: "aider".into(),
                label: Some("Aider".into()),
                prompt_args: Some(strings(&["--message", "{prompt}"])),
                ..Default::default()
            },
            HarnessOverride {
                id: "grok".into(),
                ..Default::default()
            },
        ];
        let all = resolve_all(&overrides);
        let by_id = |id: &str| all.iter().find(|h| h.def.id == id).unwrap();

        assert_eq!(by_id("codex").def.efforts, ["minimal", "high"]);
        assert!(by_id("codex").builtin && by_id("codex").modified);
        assert!(!by_id("grok").modified, "an empty override changes nothing");
        assert!(!by_id("claude").modified);

        let aider = by_id("aider");
        assert!(!aider.builtin);
        assert_eq!(
            aider.def.command, "aider",
            "a custom harness defaults its command to its id"
        );
        assert_eq!(
            all.last().unwrap().def.id,
            "aider",
            "custom harnesses come after the built-ins"
        );
        assert_eq!(
            aider.def.start_args(&values("hi", "", "")),
            ["--message", "hi"]
        );
    }

    #[test]
    fn the_stdin_transport_keeps_the_prompt_off_the_command_line() {
        let mut def = find("claude", &[]).unwrap();
        def.prompt_transport = PromptTransport::Stdin;
        let args = def.start_args(&values("a very long prompt", "opus", ""));
        assert_eq!(
            args,
            [
                "--model",
                "opus",
                "--session-id",
                "11111111-2222-3333-4444-555555555555"
            ]
        );
    }

    #[test]
    fn resume_and_fork_replace_the_session_and_prompt_groups() {
        let claude = find("claude", &[]).unwrap();
        let v = values("ignored", "opus", "");
        assert_eq!(
            claude.continue_args(&v, false),
            [
                "--model",
                "opus",
                "--resume",
                "11111111-2222-3333-4444-555555555555"
            ]
        );
        // A fork names both conversations: the one it copies and the one it becomes.
        assert_eq!(
            claude.continue_args(&v, true)[2..],
            [
                "--resume",
                "11111111-2222-3333-4444-555555555555",
                "--fork-session",
                "--session-id",
                "99999999-8888-7777-6666-555555555555"
            ]
        );
        assert_eq!(
            find("codex", &[]).unwrap().continue_args(&v, false),
            ["-m", "opus", "resume", "--last"]
        );
    }

    #[test]
    fn definitions_are_checked_before_saving() {
        let good = find("claude", &[]).unwrap();
        assert!(validate(&good).is_ok());
        for (breakage, hint) in [
            (
                Box::new(|d: &mut HarnessDef| d.id = "Has Spaces".into())
                    as Box<dyn Fn(&mut HarnessDef)>,
                "id",
            ),
            (
                Box::new(|d: &mut HarnessDef| d.command = "  ".into()),
                "command",
            ),
            (
                Box::new(|d: &mut HarnessDef| d.label = String::new()),
                "label",
            ),
            (
                Box::new(|d: &mut HarnessDef| d.session_args = vec![]),
                "{session_id}",
            ),
            (
                Box::new(|d: &mut HarnessDef| d.prompt_args = strings(&["--msg"])),
                "{prompt}",
            ),
        ] {
            let mut bad = good.clone();
            breakage(&mut bad);
            assert!(validate(&bad).unwrap_err().contains(hint), "{hint}");
        }
        assert!(builtin().iter().all(|def| validate(def).is_ok()));
    }

    #[test]
    fn builtin_ids_are_unique_and_lookups_work() {
        let all = builtin();
        let mut ids: Vec<_> = all.iter().map(|h| h.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(find("nope", &[]).is_none());
    }
}

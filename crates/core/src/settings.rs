//! User settings, in a TOML file people can read, edit, back up and diff.
//!
//! The file holds only what differs from the defaults. Two rules protect it: it is written
//! atomically, and a file we could not parse is never overwritten — it is set aside first.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::assist::Thresholds;
use crate::harness::HarnessOverride;

pub const DEFAULT_BRANCH_PREFIX: &str = "ys";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip_serializing_if = "GeneralSettings::is_default")]
    pub general: GeneralSettings,
    pub workspaces: WorkspaceSettings,
    #[serde(skip_serializing_if = "AssistSettings::is_default")]
    pub assist: AssistSettings,
    #[serde(skip_serializing_if = "DraftSettings::is_default")]
    pub draft: DraftSettings,
    #[serde(skip_serializing_if = "ActivitySettings::is_default")]
    pub activity: ActivitySettings,
    /// Overrides of built-in harnesses, and whole custom ones. See [`HarnessOverride`].
    #[serde(rename = "harness", skip_serializing_if = "Vec::is_empty")]
    pub harnesses: Vec<HarnessOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    /// The command "Open in editor" runs, e.g. `code` or `zed --new`. It is given the workspace
    /// folder and then the file. `None` tries the common editors in turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_command: Option<String>,
    /// Send a desktop notification when an agent that was busy for a while goes quiet and
    /// nobody is looking at it.
    pub notify_when_quiet: bool,
    /// Look for a newer release shortly after starting and once a day. Looking is all it does:
    /// nothing is downloaded or installed until the user says so.
    pub check_for_updates: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            editor_command: None,
            notify_when_quiet: true,
            check_for_updates: true,
        }
    }
}

impl GeneralSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Assist: small judgments by TypeSafe's Jev model. Each feature sends something off the machine,
/// so each is off until the user turns it on. The API key is not here: it lives in the OS
/// credential store (see `assist::key`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistSettings {
    /// Check each changed file against the workspace's task and for risky edits. Sends diffs.
    pub review_changes: bool,
    /// Suggest a harness and an effort in the composer. Sends the message being typed, and the
    /// harnesses' "Good at" descriptions.
    pub suggest_in_composer: bool,
    /// How sure the model must be before Yardsort acts on an answer. See [`Thresholds`].
    #[serde(flatten)]
    pub thresholds: Thresholds,
}

impl AssistSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Having a model write a commit message or a pull request. See [`crate::draft`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DraftSettings {
    /// Offer the button at all. On, because it costs nothing until it is pressed; switching it
    /// off is for people who would rather not be offered.
    pub enabled: bool,
    /// The model the API key path asks for. Ignored when an agent does the writing — that one
    /// uses whatever the agent itself is configured to use.
    pub model: String,
}

impl Default for DraftSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: crate::draft::DEFAULT_MODEL.to_owned(),
        }
    }
}

impl DraftSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Agent activity: the local record of what was started and how it ended. See
/// [`crate::activity`]. Nothing here leaves the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActivitySettings {
    /// Record when a harness, shell or run command starts and exits in a workspace. On by
    /// default: it is what Yardsort already knew, kept.
    pub record_lifecycle: bool,
    /// Show the experimental activity timeline in a workspace's footer.
    pub show_timeline: bool,
    /// Give Yardsort's own launches of Claude Code a settings file whose hooks report what it
    /// does — prompts, tools, turns, the session — as metadata. Off by default: it is the
    /// agent's cooperation, asked for per launch, and needs `record_lifecycle`.
    pub capture_claude: bool,
    /// Give Yardsort's own launches of Codex a `notify` program that reports each turn, and
    /// read the turn's commands, file changes and token usage from Codex's own session file.
    /// Off by default, for the same reasons.
    pub capture_codex: bool,
    /// Give Yardsort's own launches of OpenCode a plugin, through its environment, that
    /// reports what it does. Off by default, for the same reasons.
    pub capture_opencode: bool,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        Self {
            record_lifecycle: true,
            show_timeline: false,
            capture_claude: false,
            capture_codex: false,
            capture_opencode: false,
        }
    }
}

impl ActivitySettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceSettings {
    /// Where worktrees are created: `<root>/<project>/<workspace>`. `None` means
    /// `~/yardsort`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<String>,
    /// New branches are named `<prefix>/<workspace>`; empty means no prefix.
    pub branch_prefix: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            worktree_root: None,
            branch_prefix: DEFAULT_BRANCH_PREFIX.to_owned(),
        }
    }
}

impl WorkspaceSettings {
    /// The message to show if these cannot be saved.
    pub fn validate(&self) -> Result<(), String> {
        let prefix = &self.branch_prefix;
        let bad_ref = prefix.starts_with(['/', '-', '.'])
            || prefix.ends_with(['/', '.'])
            || prefix.contains("..")
            || prefix.contains("//")
            || prefix.ends_with(".lock")
            || prefix
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\@{".contains(c));
        if bad_ref {
            return Err(format!("\"{prefix}\" cannot be part of a git branch name."));
        }
        if let Some(root) = &self.worktree_root {
            if !Path::new(root).is_absolute() {
                return Err("The worktree folder must be an absolute path.".into());
            }
        }
        Ok(())
    }

    /// The full branch name for a workspace called `name`.
    pub fn branch_for(&self, name: &str) -> String {
        if self.branch_prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", self.branch_prefix)
        }
    }

    /// The workspace name a branch suggests: the branch without our prefix.
    pub fn name_from_branch<'a>(&self, branch: &'a str) -> &'a str {
        if self.branch_prefix.is_empty() {
            return branch;
        }
        branch
            .strip_prefix(&self.branch_prefix)
            .and_then(|rest| rest.strip_prefix('/'))
            // Branches from when the app was Switchyard and the prefix `sy`.
            .or_else(|| crate::legacy::strip_old_branch_prefix(branch))
            .unwrap_or(branch)
    }
}

pub struct SettingsFile {
    path: PathBuf,
    state: Mutex<Loaded>,
}

struct Loaded {
    settings: Settings,
    /// Why the file on disk was not used, if it was not.
    problem: Option<String>,
}

impl SettingsFile {
    /// Read `path`. A missing file means defaults; an unreadable one means defaults *and* a
    /// reported problem, with the file left untouched until the user saves something.
    pub fn load(path: PathBuf) -> Self {
        let (settings, problem) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(settings) => (settings, None),
                Err(error) => (
                    Settings::default(),
                    Some(format!("{} could not be read: {error}", path.display())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), None)
            }
            Err(error) => (
                Settings::default(),
                Some(format!("{} could not be read: {error}", path.display())),
            ),
        };
        Self {
            path,
            state: Mutex::new(Loaded { settings, problem }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn problem(&self) -> Option<String> {
        self.lock().problem.clone()
    }

    /// Change the settings and write them out. Nothing changes in memory unless the write
    /// succeeded.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> std::io::Result<Settings> {
        let mut state = self.lock();
        let mut next = state.settings.clone();
        change(&mut next);

        if state.problem.is_some() && self.path.exists() {
            // Never destroy a file we failed to understand; the user may want their edits back.
            std::fs::rename(&self.path, self.path.with_extension("toml.unreadable"))?;
        }
        write_atomically(&self.path, &render(&next))?;
        state.settings = next.clone();
        state.problem = None;
        Ok(next)
    }

    fn lock(&self) -> MutexGuard<'_, Loaded> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn render(settings: &Settings) -> String {
    let body = toml::to_string_pretty(settings).expect("settings are plain data");
    format!(
        "# Yardsort settings. Edited by the app — comments here are not preserved.\n\
         # Only values that differ from the defaults are stored; delete an entry to restore it.\n\n{body}"
    )
}

/// Write to a sibling file, then rename over the target, so a crash never leaves half a file.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("settings.toml");
        (dir, path)
    }

    #[test]
    fn legacy_custom_harnesses_keep_their_identity_and_blank_fields_after_an_upgrade() {
        use crate::harness::{self, HarnessDef, LaunchValues};
        let (dir, _) = file();
        let path = dir.path().join("settings.toml");
        // This is what the old save path wrote: only differences from HarnessDef::custom.
        std::fs::write(&path, "[[harness]]\nid = \"omp\"\n[[harness]]\nid = \"cursor\"\n[[harness]]\nid = \"pi\"\nresume_args = [\"--continue\"]\n").unwrap();
        let file = SettingsFile::load(path.clone());
        assert!(file.problem().is_none());
        for id in ["omp", "cursor", "pi"] {
            let resolved = harness::resolve_all(&file.get().harnesses)
                .into_iter()
                .find(|h| h.def.id == id)
                .unwrap();
            assert!(!resolved.builtin && !resolved.modified);
            let mut expected = HarnessDef::custom(id);
            if id == "pi" {
                expected.resume_args = vec!["--continue".into()];
            }
            assert_eq!(resolved.def, expected);
            assert_eq!(
                resolved.def.continue_args(&LaunchValues::default(), false),
                expected.resume_args
            );
            expected.label = format!("My {id}");
            file.update(|s| harness::save_override(&mut s.harnesses, &expected))
                .unwrap();
            let reloaded = SettingsFile::load(path.clone());
            assert_eq!(harness::find(id, &reloaded.get().harnesses), Some(expected));
            assert_eq!(
                reloaded
                    .get()
                    .harnesses
                    .iter()
                    .find(|h| h.id == id)
                    .unwrap()
                    .builtin,
                Some(false)
            );
        }
    }

    #[test]
    fn new_builtin_overrides_follow_builtin_defaults_after_saving_and_reloading() {
        use crate::harness;
        let (_dir, path) = file();
        let file = SettingsFile::load(path.clone());
        for id in ["omp", "cursor", "pi"] {
            let base = harness::find(id, &[]).unwrap();
            let mut wanted = base.clone();
            wanted.label = format!("My {id}");
            file.update(|s| harness::save_override(&mut s.harnesses, &wanted))
                .unwrap();
            let loaded = SettingsFile::load(path.clone());
            let settings = loaded.get();
            let resolved = harness::resolve_all(&settings.harnesses)
                .into_iter()
                .find(|h| h.def.id == id)
                .unwrap();
            assert!(resolved.builtin && resolved.modified);
            assert_eq!(resolved.def, wanted);
            let entry = settings.harnesses.iter().find(|h| h.id == id).unwrap();
            assert_eq!(entry.builtin, Some(true));
            assert_eq!(entry.session_args, None);
            file.update(|s| harness::save_override(&mut s.harnesses, &base))
                .unwrap();
            assert!(!file.get().harnesses.iter().any(|h| h.id == id));
        }
    }

    #[test]
    fn a_missing_file_means_defaults_and_is_not_created_by_reading() {
        let (_dir, path) = file();
        let settings = SettingsFile::load(path.clone());
        assert_eq!(settings.get(), Settings::default());
        assert_eq!(settings.get().workspaces.branch_prefix, "ys");
        assert_eq!(settings.problem(), None);
        assert!(!path.exists());
    }

    #[test]
    fn changes_survive_a_reload_and_only_differences_are_written() {
        let (_dir, path) = file();
        SettingsFile::load(path.clone())
            .update(|s| {
                s.workspaces.branch_prefix = "wip".into();
                s.harnesses.push(HarnessOverride {
                    id: "claude".into(),
                    command: Some("/opt/claude".into()),
                    ..Default::default()
                });
            })
            .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("branch_prefix = \"wip\"") && text.contains("[[harness]]"),
            "{text}"
        );
        assert!(
            !text.contains("worktree_root") && !text.contains("model_args"),
            "{text}"
        );

        let reloaded = SettingsFile::load(path).get();
        assert_eq!(reloaded.workspaces.branch_prefix, "wip");
        assert_eq!(
            reloaded.harnesses[0].command.as_deref(),
            Some("/opt/claude")
        );
    }

    #[test]
    fn assist_switches_and_thresholds_round_trip_and_stay_out_of_the_file_until_changed() {
        let (_dir, path) = file();
        let settings = SettingsFile::load(path.clone());
        assert_eq!(settings.get().assist.thresholds, Thresholds::default());

        settings
            .update(|s| {
                s.assist.review_changes = true;
                s.assist.thresholds.flag_at_percent = 80;
            })
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("review_changes = true"), "{text}");
        assert!(text.contains("flag_at_percent = 80"), "{text}");

        let reloaded = SettingsFile::load(path).get().assist;
        assert!(reloaded.review_changes && !reloaded.suggest_in_composer);
        assert_eq!(reloaded.thresholds.flag_at_percent, 80);
        assert_eq!(
            reloaded.thresholds.off_task_at_percent,
            Thresholds::default().off_task_at_percent,
            "a threshold nobody touched keeps the default"
        );
    }

    #[test]
    fn a_hand_written_file_with_unknown_keys_and_gaps_still_loads() {
        let (_dir, path) = file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "future_feature = true\n\n[[harness]]\nid = \"aider\"\ncommand = \"aider\"\nprompt_args = [\"--message\", \"{prompt}\"]\n",
        )
        .unwrap();
        let settings = SettingsFile::load(path);
        assert_eq!(settings.problem(), None);
        assert_eq!(settings.get().harnesses[0].id, "aider");
        assert_eq!(settings.get().workspaces, WorkspaceSettings::default());
    }

    #[test]
    fn a_broken_file_is_reported_and_set_aside_not_overwritten() {
        let (_dir, path) = file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[workspaces\nbranch_prefix = ").unwrap();

        let settings = SettingsFile::load(path.clone());
        assert!(settings.problem().unwrap().contains("could not be read"));
        assert_eq!(settings.get(), Settings::default());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[workspaces\nbranch_prefix = ",
            "untouched"
        );

        settings
            .update(|s| s.workspaces.branch_prefix = "x".into())
            .unwrap();
        assert_eq!(settings.problem(), None);
        let kept = std::fs::read_to_string(path.with_extension("toml.unreadable")).unwrap();
        assert_eq!(kept, "[workspaces\nbranch_prefix = ");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("branch_prefix = \"x\""));
    }

    #[test]
    fn branch_prefixes_are_checked_and_applied() {
        let with = |prefix: &str| WorkspaceSettings {
            branch_prefix: prefix.into(),
            ..Default::default()
        };
        for ok in ["ys", "", "joao/wip", "feature"] {
            assert!(with(ok).validate().is_ok(), "{ok:?}");
        }
        for bad in ["has space", "/lead", "trail/", "a..b", "x~y", "-dash", "q?"] {
            assert!(with(bad).validate().is_err(), "{bad:?}");
        }
        assert_eq!(with("ys").branch_for("fix"), "ys/fix");
        assert_eq!(with("").branch_for("fix"), "fix");
        assert_eq!(with("ys").name_from_branch("ys/fix"), "fix");
        assert_eq!(
            with("ys").name_from_branch("system"),
            "system",
            "a prefix is a whole path part"
        );
        assert_eq!(with("").name_from_branch("ys/fix"), "ys/fix");

        let relative = WorkspaceSettings {
            worktree_root: Some("relative/dir".into()),
            ..Default::default()
        };
        assert!(relative.validate().is_err());
    }
}

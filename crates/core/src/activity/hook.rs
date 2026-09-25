//! The executable's hook mode: `yardsort --yardsort-hook <harness> <inbox directory>`.
//!
//! An agent runs this with a JSON payload on stdin. It reduces the payload to metadata, writes
//! one file to the inbox, and exits 0 — always 0. A non-zero exit is something Claude Code
//! shows the user, and 2 would *block* the action the hook was told about; Yardsort observes,
//! it never gets in the way. Whatever goes wrong is a line on stderr, which the agent keeps to
//! itself. Nothing is written to stdout: some hooks' stdout is read back as instructions.
//!
//! Both the app's binary and `ys` have this mode, before anything else they do: whichever
//! launched the agent named itself in the settings file, and both are the same core.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;

use super::inbox::{Inbox, InboxEntry, INBOX_VERSION};
use super::{claude, PRIVACY_METADATA, RECORD_ENV, RUN_ENV, WORKSPACE_ENV};

/// Makes this executable *be* a hook. See the module documentation.
pub const HOOK_FLAG: &str = "--yardsort-hook";
/// Read no more of stdin than this: a tool's output can be large, and only its size matters.
pub const MAX_STDIN_BYTES: u64 = 8 * 1024 * 1024;

/// Run as a hook and exit, when asked to. Called before Tauri or clap see the arguments.
pub fn run_hook_and_exit_if_asked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(HOOK_FLAG)) {
        return;
    }
    let (Some(harness), Some(inbox)) = (args.next(), args.next()) else {
        eprintln!("{HOOK_FLAG} takes a harness id and an inbox directory");
        std::process::exit(0);
    };
    let env = |name: &str| std::env::var(name).ok();
    let mut stdin = std::io::stdin().lock();
    if let Err(why) = deliver(
        &harness.to_string_lossy(),
        Path::new(&inbox),
        &mut stdin,
        &env,
    ) {
        eprintln!("yardsort hook: {why}");
    }
    std::process::exit(0);
}

/// Read one payload, keep its metadata, and put it in the inbox. `env` is the hook process's
/// environment, which is the agent's, which is what the launcher set: the run, workspace and
/// record ids ride in on it.
pub fn deliver(
    harness: &str,
    inbox_dir: &Path,
    input: &mut dyn Read,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<InboxEntry, String> {
    let mut bytes = Vec::new();
    input
        .take(MAX_STDIN_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading stdin: {error}"))?;
    let payload: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("stdin is not JSON: {error}"))?;
    let (producer, reported) = match harness {
        claude::HARNESS_ID => (claude::PRODUCER, claude::normalize(&payload)?),
        other => return Err(format!("no adapter for harness `{other}`")),
    };
    let entry = InboxEntry {
        version: INBOX_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: crate::store::now_ms(),
        producer: producer.to_owned(),
        method: claude::METHOD.to_owned(),
        fidelity: claude::FIDELITY.to_owned(),
        run_id: env(RUN_ENV).filter(|v| !v.is_empty()),
        workspace_id: env(WORKSPACE_ENV).filter(|v| !v.is_empty()),
        session_record_id: env(RECORD_ENV).filter(|v| !v.is_empty()),
        native_session_id: reported.native_session_id,
        kind: reported.kind.to_owned(),
        privacy_class: PRIVACY_METADATA.to_owned(),
        payload: reported.payload,
    };
    Inbox::new(inbox_dir.to_path_buf())
        .record(&entry)
        .map_err(|error| format!("could not write to the inbox: {error}"))?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/claude-hooks")
                .join(claude::FIXTURE_VERSION)
                .join(format!("{name}.json")),
        )
        .unwrap()
    }

    #[test]
    fn a_payload_becomes_one_inbox_entry_carrying_the_launchers_ids() {
        let dir = tempfile::tempdir().unwrap();
        let env = |name: &str| match name {
            RUN_ENV => Some("run-7".to_owned()),
            WORKSPACE_ENV => Some("ws-3".to_owned()),
            RECORD_ENV => Some(String::new()),
            _ => None,
        };
        let entry = deliver(
            "claude",
            dir.path(),
            &mut fixture("04-PostToolUse").as_slice(),
            &env,
        )
        .unwrap();
        assert_eq!(entry.kind, "tool.completed");
        assert_eq!(entry.run_id.as_deref(), Some("run-7"));
        assert_eq!(entry.workspace_id.as_deref(), Some("ws-3"));
        assert_eq!(entry.session_record_id, None, "empty is absent");
        assert_eq!(
            entry.native_session_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(entry.payload["tool"], "Write");

        let inbox = Inbox::new(dir.path().to_path_buf());
        let files = inbox.entries().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(Inbox::read(&files[0]).unwrap(), entry);
    }

    #[test]
    fn what_cannot_be_understood_is_refused_without_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let env = |_: &str| None;
        let garbage = deliver("claude", dir.path(), &mut b"not json".as_slice(), &env);
        assert!(garbage.unwrap_err().contains("not JSON"));
        let unknown = deliver(
            "claude",
            dir.path(),
            &mut br#"{"hook_event_name":"MessageDisplay"}"#.as_slice(),
            &env,
        );
        assert!(unknown.unwrap_err().contains("MessageDisplay"));
        let no_adapter = deliver(
            "codex",
            dir.path(),
            &mut fixture("11-Stop").as_slice(),
            &env,
        );
        assert!(no_adapter.unwrap_err().contains("codex"));
        assert!(Inbox::new(dir.path().to_path_buf())
            .entries()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn stdin_is_read_up_to_the_bound_and_no_further() {
        let dir = tempfile::tempdir().unwrap();
        let env = |_: &str| None;
        // Valid JSON up to the bound, then more: the parse fails on the truncation, and the
        // process has not tried to hold the whole thing.
        let mut huge =
            br#"{"hook_event_name":"Stop","session_id":"s","last_assistant_message":""#.to_vec();
        huge.resize(usize::try_from(MAX_STDIN_BYTES).unwrap() + 100, b'x');
        let result = deliver("claude", dir.path(), &mut huge.as_slice(), &env);
        assert!(result.is_err());
    }
}

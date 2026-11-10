//! What a named artifact may contain, and what it must not.
//!
//! A workflow step declares a name and a source. This module turns that source into text when
//! the step succeeds. The default sources are a file in the workspace, text the file itself
//! wrote, and a field the step already leaves. A conversation is copied only when the step says
//! `from: transcript`. Even then, tool output and the agent's reasoning are not in the turns
//! Yardsort reads, and `raw` is what asks for those turns in full instead of the budgeted digest.
//!
//! Nothing here reads the terminal scrollback, the environment, or a file the step did not name.

use std::io::Read;
use std::path::{Component, Path};

use crate::activity::digest::{self, Conversation};
use crate::activity::transcript::Turn;

/// The most text an artifact keeps, in characters. The same order of size as a conversation
/// digest. Longer text is cut, and the stored body says so.
pub const MAX_CHARS: usize = 32_000;

/// A file larger than this is refused rather than read. An artifact is a document, not a dump.
const MAX_READ_BYTES: u64 = 1024 * 1024;

const CUT_NOTE: &str = "\n\n[Yardsort cut this artifact at 32000 characters.]";

/// Cut `text` to [`MAX_CHARS`], noting the cut in the stored body.
///
/// C0 and C1 controls are dropped first, except newline and tab. A body is typed into a
/// later session, and a control there can close a paste or hide the rest of the text.
pub fn cap(text: &str) -> (String, bool) {
    let text = strip_controls(text);
    if text.chars().count() <= MAX_CHARS {
        return (text, false);
    }
    let mut out: String = text.chars().take(MAX_CHARS).collect();
    out.push_str(CUT_NOTE);
    (out, true)
}

/// Refuse an empty document. The next phase would otherwise start with a blank handoff.
pub fn require_text(name: &str, text: String) -> Result<(String, bool), String> {
    let (body, truncated) = cap(&text);
    if body.trim().is_empty() {
        return Err(format!(
            "The artifact `{name}` is empty, so there is nothing to hand on."
        ));
    }
    Ok((body, truncated))
}

/// Drop C0 and C1 controls. Newline and tab stay, so a document keeps its lines.
fn strip_controls(text: &str) -> String {
    text.chars()
        .filter(|ch| {
            let code = u32::from(*ch);
            *ch == '\n' || *ch == '\t' || !((code <= 0x1F) || (0x80..=0x9F).contains(&code))
        })
        .collect()
}

/// Why `path` cannot be an artifact file, before it is read. A path that still has
/// `{{ variables }}` is checked again once those are filled in.
pub fn check_path(path: &str) -> Result<(), String> {
    // Before trimming: a trailing space is part of the name, and Windows ignores it when
    // opening the file. A colon is an alternate data stream.
    if let Some(part) = unsafe_component(Path::new(path)) {
        return Err(unsafe_message(path.trim(), &part));
    }
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("The artifact path is empty.".into());
    }
    if trimmed.contains("{{") {
        return Ok(());
    }
    let rel = Path::new(trimmed);
    // `is_absolute` is not enough. On Windows `/etc/passwd` and `\Windows\win.ini` have a root
    // without being absolute, and `C:secret` has a drive prefix. Joining either onto the
    // workspace replaces part of that path and can leave the folder.
    if rel.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(format!(
            "`{trimmed}` is outside the workspace. An artifact file is a path inside it."
        ));
    }
    if is_secret(rel) {
        return Err(secret_message(trimmed));
    }
    if let Some(part) = unsafe_component(rel) {
        return Err(unsafe_message(trimmed, &part));
    }
    Ok(())
}

/// Read a workspace file for an artifact. The path must stay inside `root` after links are
/// resolved, and the resolved name is checked again: `notes.md` pointing at `.env` is still
/// a secret. The text is not capped here.
pub fn read_file(root: &Path, path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    // A rendered path no longer has braces, so the same checks apply.
    if trimmed.contains("{{") {
        return Err(format!("`{trimmed}` still has a variable in it."));
    }
    check_path(trimmed)?;
    let root_real = root
        .canonicalize()
        .map_err(|error| format!("Could not read the workspace: {error}"))?;
    let full = root.join(trimmed);
    let full_real = match full.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "`{trimmed}` is not in the workspace yet, so there is nothing to hand on."
            ));
        }
        Err(error) => return Err(format!("Could not read `{trimmed}`: {error}")),
    };
    contained(&root_real, &full_real, trimmed)?;
    // Open the resolved path, not the name that was written, so a swapped link is not what
    // is read. The size is the handle's, and the read stops after the cap.
    let file = std::fs::File::open(&full_real)
        .map_err(|error| format!("Could not read `{trimmed}`: {error}"))?;
    let again = full
        .canonicalize()
        .map_err(|error| format!("Could not read `{trimmed}`: {error}"))?;
    if again != full_real {
        return Err(format!(
            "`{trimmed}` is outside the workspace. An artifact file is a path inside it."
        ));
    }
    contained(&root_real, &again, trimmed)?;
    let meta = file
        .metadata()
        .map_err(|error| format!("Could not read `{trimmed}`: {error}"))?;
    if !meta.is_file() {
        return Err(format!("`{trimmed}` is not a file."));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!("`{trimmed}` is too large to hand on."));
    }
    let mut bytes = Vec::new();
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read `{trimmed}`: {error}"))?;
    if bytes.len() as u64 > MAX_READ_BYTES {
        return Err(format!("`{trimmed}` is too large to hand on."));
    }
    if bytes.contains(&0) {
        return Err(format!("`{trimmed}` is not text, so it is not copied."));
    }
    String::from_utf8(bytes).map_err(|_| format!("`{trimmed}` is not text, so it is not copied."))
}

/// The resolved path stays inside the workspace, and its real name is not a secret.
fn contained(root: &Path, full: &Path, written: &str) -> Result<(), String> {
    if !full.starts_with(root) {
        return Err(format!(
            "`{written}` is outside the workspace. An artifact file is a path inside it."
        ));
    }
    let rel = full.strip_prefix(root).unwrap_or(full);
    if is_secret(rel) {
        return Err(secret_message(written));
    }
    if let Some(part) = unsafe_component(rel) {
        return Err(unsafe_message(written, &part));
    }
    Ok(())
}

/// The text of one session's conversation, for a step that asked for a transcript.
///
/// Without `raw`, this is the budgeted digest: turns shortened to fit, tool output left out
/// because it is not part of a turn. With `raw`, each turn's own text is kept in full, still
/// without tool output, for the caller to cap.
pub fn conversation_text(
    conversations: &[Conversation],
    record_id: &str,
    raw: bool,
) -> Result<Option<String>, String> {
    let Some(conversation) = conversations.iter().find(|c| c.id == record_id) else {
        return Ok(None);
    };
    if let Some(why) = &conversation.unreadable {
        return Err(format!("Could not read that session's conversation: {why}"));
    }
    if conversation.turns.is_empty() {
        return Ok(None);
    }
    if raw {
        let text = raw_turns(&conversation.turns);
        return Ok((!text.trim().is_empty()).then_some(text));
    }
    let digest = digest::select(
        conversations,
        &digest::Options {
            budget: digest::PACKET_BUDGET,
            conversation: Some(record_id.to_owned()),
            since: None,
        },
    );
    let text = digest::render(&digest);
    Ok((!text.trim().is_empty()).then_some(text))
}

/// A file name, or a directory on the way to it, that holds secrets rather than a document.
fn is_secret(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    const DIRS: &[&str] = &[".ssh", ".aws", ".gnupg", ".git", ".kube", ".docker"];
    if parts.iter().any(|part| DIRS.contains(&part.as_str())) {
        return true;
    }
    let Some(name) = parts.last() else {
        return false;
    };
    if name == ".env" || name.starts_with(".env.") || name == ".envrc" {
        return !matches!(
            name.as_str(),
            ".env.example" | ".env.sample" | ".env.template"
        );
    }
    if name == "secrets" || name.starts_with("secrets.") {
        return true;
    }
    const NAMES: &[&str] = &[
        "credentials",
        "credentials.json",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".pgpass",
        ".git-credentials",
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
    ];
    if NAMES.contains(&name.as_str()) {
        return true;
    }
    matches!(
        name.rsplit_once('.').map(|(_, ext)| ext),
        Some("pem" | "key" | "p12" | "pfx" | "tfvars")
    )
}

fn secret_message(path: &str) -> String {
    format!("`{path}` looks like a secret, so it is not copied into the run.")
}

/// A component Windows treats specially: an alternate data stream (`file:stream`), or a
/// trailing dot or space, which the file system ignores when it opens the file.
fn unsafe_component(path: &Path) -> Option<String> {
    path.components().find_map(|component| match component {
        Component::Normal(part) => {
            let part = part.to_string_lossy();
            (part.contains(':') || part.ends_with('.') || part.ends_with(' '))
                .then(|| part.into_owned())
        }
        _ => None,
    })
}

fn unsafe_message(path: &str, part: &str) -> String {
    format!("`{path}` is not a safe file name (`{part}`), so it is not copied into the run.")
}

fn raw_turns(turns: &[Turn]) -> String {
    let mut parts = Vec::new();
    for turn in turns {
        if !turn.user.trim().is_empty() {
            parts.push(turn.user.trim().to_owned());
        }
        for said in &turn.said {
            if !said.trim().is_empty() {
                parts.push(said.trim().to_owned());
            }
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(id: &str, turns: Vec<Turn>) -> Conversation {
        Conversation {
            id: id.to_owned(),
            harness: "claude".into(),
            model: None,
            started_at: 1,
            running: false,
            forked_from: None,
            turns,
            unreadable: None,
        }
    }

    #[test]
    fn a_long_artifact_is_cut_and_says_so() {
        let (text, truncated) = cap(&"a".repeat(MAX_CHARS + 10));
        assert!(truncated);
        assert!(text.contains("cut this artifact at 32000"));
        assert!(text.chars().count() > MAX_CHARS);
        let (same, truncated) = cap("short");
        assert!(!truncated);
        assert_eq!(same, "short");
    }

    #[test]
    fn an_empty_artifact_is_refused() {
        let error = require_text("spec", "  \n".into()).unwrap_err();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn secret_and_escaping_paths_are_refused() {
        for path in [
            ".env",
            ".env.local",
            ".envrc",
            ".git-credentials",
            ".pgpass",
            "config/credentials.json",
            ".ssh/id_rsa",
            ".kube/config",
            ".docker/config.json",
            "keys/server.pem",
            "prod.tfvars",
            "secrets.yaml",
            "file.txt:Zone.Identifier",
            "notes.",
            "notes ",
            "../secret.md",
            "/etc/passwd",
        ] {
            assert!(check_path(path).is_err(), "{path}");
        }
        assert!(check_path("SPEC.md").is_ok());
        assert!(check_path(".env.example").is_ok());
        assert!(check_path("docs/{{ inputs.name }}.md").is_ok());
    }

    #[test]
    fn a_file_outside_the_workspace_is_not_read() {
        let root = tempfile::tempdir().unwrap();
        // A link can point outside the folder the workflow named. The resolved path is what
        // matters, and it must stay inside the workspace.
        #[cfg(unix)]
        {
            let elsewhere = tempfile::tempdir().unwrap();
            std::fs::write(elsewhere.path().join("secret.md"), "nope").unwrap();
            let link = root.path().join("linked.md");
            std::os::unix::fs::symlink(elsewhere.path().join("secret.md"), &link).unwrap();
            let error = read_file(root.path(), "linked.md").unwrap_err();
            assert!(error.contains("outside"), "{error}");
        }

        let missing = read_file(root.path(), "SPEC.md").unwrap_err();
        assert!(missing.contains("not in the workspace"), "{missing}");

        std::fs::write(root.path().join("SPEC.md"), "Be kind.").unwrap();
        assert_eq!(read_file(root.path(), "SPEC.md").unwrap(), "Be kind.");

        std::fs::write(root.path().join("blob.bin"), b"hi\0there").unwrap();
        let binary = read_file(root.path(), "blob.bin").unwrap_err();
        assert!(binary.contains("not text"), "{binary}");

        std::fs::write(root.path().join(".env"), "TOKEN=1").unwrap();
        let secret = read_file(root.path(), ".env").unwrap_err();
        assert!(secret.contains("secret"), "{secret}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_secret_file_is_not_read() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".env"), "TOKEN=1").unwrap();
        std::os::unix::fs::symlink(".env", root.path().join("notes.md")).unwrap();
        let error = read_file(root.path(), "notes.md").unwrap_err();
        assert!(error.contains("secret"), "{error}");
    }

    #[test]
    fn control_characters_are_dropped_from_an_artifact() {
        let (text, truncated) =
            require_text("spec", "be\u{0007} kind\r\n\tline\u{009b}".into()).unwrap();
        assert!(!truncated);
        assert_eq!(text, "be kind\n\tline");
        let error = require_text("spec", "\u{0001}\r".into()).unwrap_err();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn a_transcript_is_the_digest_unless_raw_is_asked() {
        let long = "x".repeat(2_000);
        let mut turns = Vec::new();
        for n in 0..6 {
            turns.push(Turn {
                at: n,
                user: format!("question {n}"),
                said: vec![if n == 0 {
                    long.clone()
                } else {
                    format!("answer {n}")
                }],
            });
        }
        let conversations = vec![conversation("rec-1", turns)];
        let digest = conversation_text(&conversations, "rec-1", false)
            .unwrap()
            .unwrap();
        let raw = conversation_text(&conversations, "rec-1", true)
            .unwrap()
            .unwrap();
        assert!(digest.contains("question 5"), "{digest}");
        assert!(!digest.contains(&long), "the digest shortens an older turn");
        assert!(raw.contains(&long), "raw keeps the turn's own text");
        assert!(!raw.contains("tool output"));
        assert!(conversation_text(&conversations, "missing", false)
            .unwrap()
            .is_none());
    }
}

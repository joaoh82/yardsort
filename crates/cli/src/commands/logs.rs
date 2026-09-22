//! `ys logs` — what a session has on its screen, without taking over the terminal.
//!
//! The daemon keeps a session after its process ends so the last screen stays readable. That is
//! the screen this prints, which makes it the way to see how a finished agent ended up — the one
//! thing [`attach`](super::attach) refuses to do, since there is nothing left to type at.

use std::io::Write;
use std::sync::{Arc, Mutex};

use pty_host::{SessionId, TerminalHost};

use crate::target::{self, NoMatch};
use crate::{Failure, Output, Yardsort};

pub fn run(
    ys: &Yardsort,
    wanted: Option<String>,
    raw: bool,
    lines: Option<usize>,
    out: &Output,
) -> Result<(), Failure> {
    let client = ys.daemon().ok_or_else(|| {
        Failure::new(
            "No terminal daemon is running for this profile, so no screens are being kept.\n\
             Sessions live in it, not in the database — once it stops, their screens are gone.",
        )
    })?;

    let candidates = target::candidates(
        &client.list(),
        &ys.store.workspaces()?,
        &ys.store.all_sessions()?,
    );
    let target = target::resolve(&candidates, wanted.as_deref()).map_err(describe)?;
    let session = SessionId(target.pty.clone());

    // The snapshot is handed to the sink before `attach` returns — everything shares one ordered
    // stream, so by the time the reply has been read the screen has been delivered in full.
    // Nothing in the sink may ask the daemon anything: it runs on the thread that answers.
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let collected = Arc::clone(&collected);
        Box::new(move |bytes: &[u8]| {
            collected
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            true
        })
    };
    let attachment = client
        .attach(&session, sink)
        .map_err(|e| Failure::new(format!("cannot read that session: {e}")))?;
    let _ = client.detach(&session, attachment);
    let snapshot = std::mem::take(
        &mut *collected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );

    if raw {
        if out.json {
            return Err(Failure::new(
                "--raw and --json are different answers; pick one.",
            ));
        }
        // Escape sequences and all: repaints a terminal exactly as attaching would.
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(&snapshot)
            .and_then(|()| stdout.flush())
            .map_err(|e| Failure::new(format!("cannot write the screen: {e}")))?;
        return Ok(());
    }

    let text = pty_host::snapshot::to_text(&snapshot, target.size.rows, target.size.cols);
    let text = match lines {
        Some(n) => {
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(n)..].join("\n")
        }
        None => text,
    };

    out.emit(
        &Screen {
            session: target.pty.clone(),
            workspace: target.workspace.clone(),
            running: target.running,
            text: text.clone(),
        },
        || {
            if text.is_empty() {
                println!("{} has printed nothing.", target.label);
            } else {
                println!("{text}");
            }
        },
    )
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Screen {
    session: String,
    workspace: Option<String>,
    running: bool,
    text: String,
}

fn describe(no_match: NoMatch) -> Failure {
    Failure::new(match no_match {
        NoMatch::Nothing => "The daemon is holding no sessions at all.".to_owned(),
        NoMatch::Unknown { choices } => format!(
            "No session by that name. The daemon is holding: {}.",
            choices.join(", ")
        ),
        NoMatch::Several { choices } => format!(
            "Which one? {} — name a workspace, or the start of an id from `ys session list`.",
            choices.join(", ")
        ),
    })
}

//! A terminal on another computer, in this window: watched through that host's link, and typed
//! into once the person here takes control. See [`remote_host::computer::terminal`] for what is
//! sent when, and [`crate::computers`] for the rest of this computer as a client.
//!
//! **One stream, in order.** What the terminal shows comes on a single channel: each message is a
//! tag byte, then either the terminal's own bytes (`0`) or a [`RemoteTerminalSeen`] as JSON (`1`),
//! so a resize or a fresh start is never drawn out of order with the bytes around it. The bytes are
//! never read here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use pty_host::TermSize;
use remote_host::computer::terminal::{self, Asker, Seen};
use remote_proto::Controller;
use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::computers::home;
use crate::error::{IpcError, IpcResult};
use crate::state::AppState;
use crate::terminal::RawBytes;

const OUTPUT: u8 = 0;
const SEEN: u8 = 1;

/// The remote terminals open in this window, by the number each was given.
#[derive(Default)]
pub struct RemoteTerminals {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Open>>,
}

struct Open {
    asker: Asker,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        // With every asker gone too, control is handed back and the terminal detached.
        self.task.abort();
    }
}

/// What a remote terminal shows besides its bytes, in their order.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RemoteTerminalSeen {
    /// Clear the view, at this size: a snapshot follows.
    Reset {
        size: TermSize,
    },
    Resized {
        size: TermSize,
    },
    /// Who controls it, by that computer's word.
    Controller {
        controller: RemoteController,
    },
    Exited {
        code: u32,
        success: bool,
    },
    /// Control is this computer's.
    Held,
    /// Control is not this computer's: why, when the person should know.
    NotHeld {
        why: Option<String>,
    },
    /// Not reached just now; it comes back by itself.
    Away {
        why: String,
    },
    /// It cannot be watched any more.
    Gone {
        why: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RemoteController {
    Nobody,
    /// The person at that computer.
    Local,
    Device {
        device: String,
        name: String,
    },
}

fn size(size: remote_proto::TermSize) -> TermSize {
    TermSize {
        cols: size.cols,
        rows: size.rows,
    }
}

fn seen(seen: Seen) -> Result<Vec<u8>, RemoteTerminalSeen> {
    Err(match seen {
        Seen::Output(bytes) => return Ok(bytes),
        Seen::Reset { size: s } => RemoteTerminalSeen::Reset { size: size(s) },
        Seen::Resized(s) => RemoteTerminalSeen::Resized { size: size(s) },
        Seen::Controller(controller) => RemoteTerminalSeen::Controller {
            controller: match controller {
                Controller::Nobody => RemoteController::Nobody,
                Controller::Local => RemoteController::Local,
                Controller::Device { device, name } => RemoteController::Device {
                    device: device.as_str().to_owned(),
                    name,
                },
            },
        },
        Seen::Exited(exit) => RemoteTerminalSeen::Exited {
            code: exit.code,
            success: exit.success,
        },
        Seen::Held => RemoteTerminalSeen::Held,
        Seen::NotHeld(why) => RemoteTerminalSeen::NotHeld { why },
        Seen::Away(why) => RemoteTerminalSeen::Away { why },
        Seen::Gone(why) => RemoteTerminalSeen::Gone { why },
    })
}

/// One message on the channel: its tag, then what it carries.
fn framed(item: Seen) -> Vec<u8> {
    match seen(item) {
        Ok(bytes) => {
            let mut framed = Vec::with_capacity(bytes.len() + 1);
            framed.push(OUTPUT);
            framed.extend_from_slice(&bytes);
            framed
        }
        Err(event) => {
            let mut framed = vec![SEEN];
            framed.extend(serde_json::to_vec(&event).unwrap_or_default());
            framed
        }
    }
}

fn asker(app: &AppHandle, id: u32) -> IpcResult<Asker> {
    let state = app.state::<AppState>();
    let open = state
        .remote_terminals
        .open
        .lock()
        .map_err(|_| IpcError::internal("poisoned"))?;
    open.get(&id)
        .map(|open| open.asker.clone())
        .ok_or_else(|| IpcError::new("remote_terminal", "That terminal is not open here."))
}

/// Watch a terminal on another computer. What it shows comes on `output`; the number it returns
/// asks things of it, until [`remote_terminal_close`].
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_open(
    app: AppHandle,
    host_id: String,
    terminal_id: String,
    output: Channel<RawBytes>,
) -> IpcResult<u32> {
    let mut watching = terminal::watch(&home(&app), &host_id, &terminal_id)
        .map_err(|f| IpcError::new(f.code, f.message))?;
    let asker = watching.asker();
    let task = tauri::async_runtime::spawn(async move {
        while let Some(item) = watching.next().await {
            // The view is gone: stop watching.
            if output.send(RawBytes::new(framed(item))).is_err() {
                return;
            }
        }
    });
    let state = app.state::<AppState>();
    let id = state.remote_terminals.next.fetch_add(1, Ordering::Relaxed);
    state
        .remote_terminals
        .open
        .lock()
        .map_err(|_| IpcError::internal("poisoned"))?
        .insert(id, Open { asker, task });
    Ok(id)
}

/// Stop watching: control, if held, goes back to that computer.
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_close(app: AppHandle, id: u32) -> IpcResult<()> {
    let state = app.state::<AppState>();
    let open = state
        .remote_terminals
        .open
        .lock()
        .map_err(|_| IpcError::internal("poisoned"))?
        .remove(&id);
    drop(open);
    Ok(())
}

/// Ask for control. The answer comes on the terminal's channel.
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_take_control(app: AppHandle, id: u32) -> IpcResult<()> {
    asker(&app, id)?.take_control();
    Ok(())
}

/// Hand control back to that computer.
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_release(app: AppHandle, id: u32) -> IpcResult<()> {
    asker(&app, id)?.release();
    Ok(())
}

/// Type, while control is held. Nothing is sent otherwise.
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_write(app: AppHandle, id: u32, data: String) -> IpcResult<()> {
    asker(&app, id)?.input(data);
    Ok(())
}

/// Fit the terminal to the view, while control is held.
#[tauri::command]
#[specta::specta]
pub async fn remote_terminal_resize(app: AppHandle, id: u32, size: TermSize) -> IpcResult<()> {
    asker(&app, id)?.resize(remote_proto::TermSize {
        cols: size.cols,
        rows: size.rows,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_what_else_it_shows_are_told_apart_by_their_first_byte() {
        assert_eq!(framed(Seen::Output(b"hi".to_vec())), b"\x00hi");
        let event = framed(Seen::Resized(remote_proto::TermSize { cols: 80, rows: 24 }));
        assert_eq!(event[0], SEEN);
        assert_eq!(
            std::str::from_utf8(&event[1..]).unwrap(),
            r#"{"kind":"resized","size":{"cols":80,"rows":24}}"#
        );
    }
}

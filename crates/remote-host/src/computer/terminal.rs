//! One terminal on another computer, watched through that host's link, and typed into once its
//! person here takes control. See docs/design/35-remote-r4-r5.md §3, "The desktop as a client".
//!
//! **Watching takes nothing.** Attached, the terminal shows at the host's size, whoever controls
//! it. Typing goes nowhere until **Take control** acquires a lease, as on the phone: the person at
//! the host sees who has it, so nothing here takes it by itself, and nothing takes it again once
//! lost.
//!
//! **Nothing is sent twice.** Input is numbered within the lease. When the link drops while input
//! is on its way, whether it arrived is not known, and the person is told so; what was waiting is
//! dropped. Control ends with the link, and is taken again only by pressing again.
//!
//! **After a drop it resumes** from the last item drawn, on the next channel to the host; a host
//! that no longer has what came after, a gap, or a new epoch starts over from a snapshot, which
//! [`Seen::Reset`] announces so the view clears first.

use std::sync::Arc;
use std::time::Duration;

use remote_client::channel::{AttachmentEnd, ChannelError, Control, ControlLoss, Watched};
use remote_client::Channel;
use remote_proto::{
    AttachStart, Controller, DeviceId, ErrorCode, ProgramExit, StreamPosition, TermSize,
    TerminalId, MAX_INPUT_BYTES,
};
use tokio::sync::{mpsc, watch};

use super::{lock, ComputerHome};
use crate::account::Failure;

/// How long to wait before attaching again after the host refused for a reason that may pass.
const RETRY: Duration = Duration::from_secs(2);

/// What watching a terminal shows, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// Start over: clear the view, at this size; a snapshot follows.
    Reset { size: TermSize },
    /// The terminal's own bytes, for the renderer unchanged.
    Output(Vec<u8>),
    /// The terminal is this size from here on.
    Resized(TermSize),
    /// Who controls it now, by the host's word.
    Controller(Controller),
    /// The program ended; its last screen stays.
    Exited(ProgramExit),
    /// Control is this computer's now.
    Held,
    /// Control is not this computer's, or no longer: why, when the person should know.
    NotHeld(Option<String>),
    /// Not reached just now: why, in a few words. It is attached again when the host is back.
    Away(String),
    /// It cannot be watched any more: why. Nothing more comes.
    Gone(String),
}

enum Ask {
    Take,
    Release,
    Input(String),
    Resize(TermSize),
}

/// One terminal watched. Once this and every [`Asker`] of it are dropped, control is handed back
/// and the terminal detached.
pub struct Watching {
    seen: mpsc::UnboundedReceiver<Seen>,
    asks: Asker,
}

impl Watching {
    /// What it shows next; `None` once it is gone.
    pub async fn next(&mut self) -> Option<Seen> {
        self.seen.recv().await
    }

    /// What asks things of it, to keep apart from what it shows.
    pub fn asker(&self) -> Asker {
        self.asks.clone()
    }
}

/// What the person here asks of a watched terminal.
#[derive(Clone)]
pub struct Asker {
    asks: mpsc::UnboundedSender<Ask>,
}

impl Asker {
    /// Ask for control: answered by [`Seen::Held`], or [`Seen::NotHeld`] with why not.
    pub fn take_control(&self) {
        let _ = self.asks.send(Ask::Take);
    }

    /// Hand control back to the host.
    pub fn release(&self) {
        let _ = self.asks.send(Ask::Release);
    }

    /// Type, while control is held; dropped otherwise. In the order given, each once.
    pub fn input(&self, data: String) {
        let _ = self.asks.send(Ask::Input(data));
    }

    /// Fit the terminal to this view, while control is held; dropped otherwise.
    pub fn resize(&self, size: TermSize) {
        let _ = self.asks.send(Ask::Resize(size));
    }
}

/// Watch `terminal` on `host`, through the link to it. Needs a Tokio runtime.
pub fn watch<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    terminal: &str,
) -> std::result::Result<Watching, Failure> {
    let terminal = TerminalId::new(terminal.to_owned())
        .map_err(|_| Failure::new("remote_terminal", "No such terminal."))?;
    let me = home
        .store()
        .remote_computer()
        .ok()
        .flatten()
        .and_then(|row| DeviceId::new(row.device_id).ok());
    let open = {
        let known = lock(home.computer());
        known
            .links
            .iter()
            .find(|(id, _)| id.as_str() == host)
            .map(|(_, link)| link.channel.subscribe())
    };
    let Some(open) = open else {
        return Err(Failure::new(
            "remote_not_paired",
            "This computer is not let in to that one.",
        ));
    };
    let (shown, seen) = mpsc::unbounded_channel();
    let (asks, asked) = mpsc::unbounded_channel();
    tokio::spawn(follow(terminal, me, open, shown, asked));
    Ok(Watching {
        seen,
        asks: Asker { asks },
    })
}

struct Follower {
    terminal: TerminalId,
    me: Option<DeviceId>,
    shown: mpsc::UnboundedSender<Seen>,
    controller: Controller,
    control: Option<Control>,
}

impl Follower {
    fn show(&self, seen: Seen) -> bool {
        self.shown.send(seen).is_ok()
    }

    /// Control is no longer this computer's: nothing waiting goes anywhere.
    fn lose(&mut self, why: Option<String>) {
        if self.control.take().is_some() {
            self.show(Seen::NotHeld(why));
        }
    }

    fn held_by_another(&self) -> bool {
        matches!(&self.controller, Controller::Device { device, .. } if Some(device) != self.me.as_ref())
    }

    /// Why control went, by who has it now.
    fn took_it(&self) -> String {
        match &self.controller {
            Controller::Local => "The person at that computer took control.".to_owned(),
            Controller::Device { device, name } if Some(device) != self.me.as_ref() => {
                format!("{} took control.", device_name(name))
            }
            _ => "This computer no longer has control.".to_owned(),
        }
    }

    fn refusal(&self, error: &ChannelError) -> String {
        match error.code() {
            Some(ErrorCode::ControlConflict) => match self.controller {
                Controller::Local => {
                    "The person at that computer is typing here. Try again once they stop."
                        .to_owned()
                }
                _ => "Another device has control of this terminal.".to_owned(),
            },
            Some(ErrorCode::Forbidden) => "That computer lets this one watch, but not type. To \
                change that, forget it here, pair again, and allow typing there."
                .to_owned(),
            _ if matches!(error, ChannelError::Closed(_)) => {
                "The connection dropped before that computer answered.".to_owned()
            }
            _ => error.to_string(),
        }
    }
}

fn device_name(name: &str) -> &str {
    if name.is_empty() {
        "Another device"
    } else {
        name
    }
}

/// How one attachment ended, for what comes next.
enum Next {
    /// Attach again, resuming after this when the host still can.
    Again(Option<StreamPosition>),
    /// The person closed it here.
    Stop,
}

async fn follow(
    terminal: TerminalId,
    me: Option<DeviceId>,
    mut open: watch::Receiver<Option<Arc<Channel>>>,
    shown: mpsc::UnboundedSender<Seen>,
    mut asked: mpsc::UnboundedReceiver<Ask>,
) {
    let mut f = Follower {
        terminal,
        me,
        shown,
        controller: Controller::Nobody,
        control: None,
    };
    let mut drawn: Option<StreamPosition> = None;
    loop {
        // The next channel to the host; none while it is away, and none ever once unpaired.
        let channel = loop {
            let current = open.borrow_and_update().clone();
            match current {
                Some(channel) if channel.is_open() => break channel,
                _ => {}
            }
            // Asked for meanwhile: nothing can be done with it yet.
            while let Ok(ask) = asked.try_recv() {
                if matches!(ask, Ask::Take) {
                    f.show(Seen::NotHeld(Some(
                        "That computer is not reached just now.".to_owned(),
                    )));
                }
            }
            tokio::select! {
                changed = open.changed() => if changed.is_err() {
                    f.show(Seen::Gone("This computer is no longer let in to that one.".to_owned()));
                    return;
                },
                ask = asked.recv() => match ask {
                    None => return,
                    Some(Ask::Take) => {
                        f.show(Seen::NotHeld(Some("That computer is not reached just now.".to_owned())));
                    }
                    Some(_) => {}
                },
            }
        };
        let mut attachment = match channel.attach(f.terminal.clone(), drawn).await {
            Ok(attachment) => attachment,
            Err(error) => match error.code() {
                Some(ErrorCode::NotFound) => {
                    f.show(Seen::Gone(
                        "That terminal is no longer on that computer.".to_owned(),
                    ));
                    return;
                }
                Some(ErrorCode::Forbidden) => {
                    f.show(Seen::Gone(
                        "That computer does not let this one watch its terminals.".to_owned(),
                    ));
                    return;
                }
                _ => {
                    f.show(Seen::Away(error.to_string()));
                    if !matches!(error, ChannelError::Closed(_)) {
                        tokio::time::sleep(RETRY).await;
                    }
                    continue;
                }
            },
        };
        let attached = attachment.attached().clone();
        if attached.start == AttachStart::Snapshot {
            f.show(Seen::Reset {
                size: attached.size,
            });
        } else {
            f.show(Seen::Resized(attached.size));
        }
        f.controller = attached.controller.clone();
        f.show(Seen::Controller(attached.controller));

        let next = 'attached: loop {
            let lost = async {
                match f.control.as_ref() {
                    Some(control) => control.lost().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                item = attachment.next() => match item {
                    Some(Watched::Output { bytes, .. }) => {
                        if !f.show(Seen::Output(bytes)) {
                            break 'attached Next::Stop;
                        }
                    }
                    Some(Watched::Resized { size, .. }) => {
                        f.show(Seen::Resized(size));
                    }
                    Some(Watched::Control(controller)) => {
                        let told = controller != f.controller;
                        f.controller = controller.clone();
                        f.show(Seen::Controller(controller));
                        // Nobody is not a loss by itself: a lapse is found at the next renewal.
                        if told && f.control.is_some()
                            && (f.controller == Controller::Local || f.held_by_another())
                        {
                            let why = f.took_it();
                            f.lose(Some(why));
                        }
                    }
                    Some(Watched::Exited(exit)) => {
                        f.lose(None);
                        f.show(Seen::Exited(exit));
                    }
                    Some(Watched::Ended(end)) => {
                        break 'attached match end {
                            // Fell behind, or out of order: from a snapshot.
                            AttachmentEnd::Detached(_) | AttachmentEnd::Gap { .. } => Next::Again(None),
                            AttachmentEnd::DetachedHere => Next::Stop,
                            AttachmentEnd::Closed(_) => {
                                f.lose(Some("Control ended when the connection dropped. Take it again when you are ready.".to_owned()));
                                f.show(Seen::Away("The connection dropped.".to_owned()));
                                Next::Again(attachment.drawn())
                            }
                        };
                    }
                    None => break 'attached Next::Again(attachment.drawn()),
                },
                why = lost => {
                    let why = match why {
                        ControlLoss::Released => None,
                        ControlLoss::Lost(_) => Some(f.took_it()),
                        ControlLoss::Closed(_) => Some("Control ended when the connection dropped. Take it again when you are ready.".to_owned()),
                    };
                    f.lose(why);
                }
                ask = asked.recv() => match ask {
                    None => break 'attached Next::Stop,
                    Some(Ask::Take) => {
                        if f.control.is_some() {
                            f.show(Seen::Held);
                            continue;
                        }
                        let take_over = f.held_by_another();
                        match channel.acquire(f.terminal.clone(), take_over).await {
                            Ok(control) => {
                                f.control = Some(control);
                                f.show(Seen::Held);
                            }
                            Err(error) => {
                                let why = f.refusal(&error);
                                f.show(Seen::NotHeld(Some(why)));
                            }
                        }
                    }
                    Some(Ask::Release) => {
                        if let Some(control) = f.control.take() {
                            let _ = control.release().await;
                            f.show(Seen::NotHeld(None));
                        }
                    }
                    Some(Ask::Input(data)) => {
                        let Some(control) = f.control.as_ref() else { continue };
                        let mut failed = None;
                        for piece in pieces(&data) {
                            if let Err(error) = control.input(piece, false).await {
                                failed = Some(error);
                                break;
                            }
                        }
                        if let Some(error) = failed {
                            let why = match &error {
                                ChannelError::Closed(_) => "The connection dropped while typing: what was typed last may not have arrived.".to_owned(),
                                _ if error.code() == Some(ErrorCode::LeaseLost) => f.took_it(),
                                _ => error.to_string(),
                            };
                            f.lose(Some(why));
                        }
                    }
                    Some(Ask::Resize(size)) => {
                        if let Some(control) = f.control.as_ref() {
                            let _ = control.resize(size).await;
                        }
                    }
                },
            }
        };
        match next {
            Next::Stop => {
                if let Some(control) = f.control.take() {
                    let _ = control.release().await;
                }
                let _ = attachment.detach().await;
                return;
            }
            // Control is the channel's, not the attachment's: it outlives a new snapshot.
            Next::Again(position) => drawn = position,
        }
    }
}

/// `data` in pieces the host takes, each whole characters.
fn pieces(data: &str) -> impl Iterator<Item = &str> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = rest.len().min(MAX_INPUT_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (piece, after) = rest.split_at(end);
        rest = after;
        Some(piece)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_paste_goes_in_pieces_of_whole_characters() {
        let text = "é".repeat(MAX_INPUT_BYTES);
        let sent: Vec<&str> = pieces(&text).collect();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|p| p.len() <= MAX_INPUT_BYTES));
        assert_eq!(sent.concat(), text);
        assert_eq!(pieces("").count(), 0);
    }
}

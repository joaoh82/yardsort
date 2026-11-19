//! The dashboard's Remote: signing this host in to a Yardsort server, and pairing a device, from
//! the page (`host_dashboard::Remote`). The same steps as `yardsort-host sign-in` and `pair`, run
//! by the host itself, which holds the sign-in and the profile while it serves.
//!
//! A browser signed in to the dashboard may do these, as a person at a terminal here may: it
//! already runs commands as this user. A command it runs may not: the guard refuses `pair` to
//! the dashboard's jobs, and those jobs do not hold the page's session.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use host_dashboard::{PairAnswer, PairProject, Pairing, Remote, SigningIn};
use remote_client::SIGN_IN_TIMEOUT;
use remote_host::account::{self, Answer};
use remote_proto::account::{DeviceKind, NewDevice};
use remote_proto::Platform;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::home::HostHome;

pub struct HostRemote {
    home: Arc<HostHome>,
    runtime: Handle,
    pairing: Arc<Mutex<PairState>>,
    signing_in: Arc<Mutex<SignState>>,
}

#[derive(Default)]
struct PairState {
    now: Option<Pairing>,
    /// When the offer lapses, while one is out.
    lapses: Option<Instant>,
    /// Waiting for a device to ask.
    waiting: Option<tokio::task::JoinHandle<()>>,
}

/// An address pasted, and where to say whether it was the sign-in's.
type Paste = (String, oneshot::Sender<Result<(), String>>);

#[derive(Default)]
struct SignState {
    now: Option<SigningIn>,
    /// Addresses pasted, each told whether it was the one.
    pastes: Option<mpsc::UnboundedSender<Paste>>,
    waiting: Option<tokio::task::JoinHandle<()>>,
    /// Which sign-in this is, counted up with each one begun: what a sign-in's own task knows
    /// itself by, once another may have taken its place.
    turn: u64,
    /// The browser has come back, or its address was pasted, and the sign-in is being finished:
    /// redeemed at the server, the host registered there and kept here. From here it is not
    /// given up, since stopping it part way would leave a host at the server that nothing here
    /// knows of. It takes a moment, and is undone by signing out.
    finishing: bool,
}

/// Said when a sign-in is asked to stop, or another to start, while one is being finished.
const FINISHING: &str = "This sign-in is being finished and cannot be given up now. Once it is \
                         done, signing out of the server undoes it.";

impl SignState {
    /// The sign-in of `turn` has its code, and goes on to finish: unless it was given up, or
    /// another took its place, while it waited.
    fn finish(&mut self, turn: u64) -> bool {
        if self.turn != turn {
            return false;
        }
        self.finishing = true;
        true
    }

    /// Give a sign-in up while it still waits for its browser. One being finished is left to,
    /// and says so where the page shows it.
    fn give_up(&mut self) -> bool {
        if self.finishing {
            if let Some(SigningIn::Waiting { problem, .. }) = self.now.as_mut() {
                *problem = Some(FINISHING.into());
            }
            return false;
        }
        if let Some(waiting) = self.waiting.take() {
            waiting.abort();
        }
        self.now = None;
        self.pastes = None;
        // Whatever of it is still running is no longer this turn's, and finishes nothing.
        self.turn += 1;
        true
    }

    /// The sign-in of `turn` is over, as `ended`.
    fn end(&mut self, turn: u64, ended: SigningIn) {
        if self.turn != turn {
            return;
        }
        self.now = Some(ended);
        self.pastes = None;
        self.waiting = None;
        self.finishing = false;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl HostRemote {
    /// Needs a Tokio runtime to be running: the one the work is done on.
    pub fn new(home: Arc<HostHome>) -> Self {
        Self {
            home,
            runtime: Handle::current(),
            pairing: Arc::default(),
            signing_in: Arc::default(),
        }
    }

    fn projects(&self) -> Vec<PairProject> {
        let projects: Vec<PairProject> = self
            .home
            .store
            .projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| PairProject {
                id: p.id,
                name: p.name,
            })
            .collect();
        if projects.len() > 1 {
            projects
        } else {
            Vec::new()
        }
    }
}

impl Remote for HostRemote {
    fn pairing(&self) -> Pairing {
        let mut state = lock(&self.pairing);
        let lapses = state.lapses;
        if let (Some(Pairing::Offered { lapses_in_ms, .. }), Some(at)) = (&mut state.now, lapses) {
            match at.checked_duration_since(Instant::now()) {
                Some(left) => *lapses_in_ms = left.as_millis() as u64,
                None => {
                    state.now = Some(Pairing::Lapsed);
                    state.lapses = None;
                }
            }
        }
        state.now.clone().unwrap_or(Pairing::Idle)
    }

    fn pair(&self) -> Result<Pairing, String> {
        if matches!(
            self.pairing(),
            Pairing::Offered { .. } | Pairing::Asked { .. }
        ) {
            return Ok(self.pairing());
        }
        self.stop_pairing();
        let home = self.home.clone();
        // Listening before the offer is made, so a device quick to ask is not missed.
        let mut asking = home.asking();
        let offer = self
            .runtime
            .block_on(account::pair(home.as_ref()))
            .map_err(|f| f.message)?;
        let good_for = Duration::from_millis(u64::from(offer.expires_in_ms));
        let lapses = Instant::now() + good_for;
        let projects = self.projects();
        let shared = self.pairing.clone();
        let waiting = self.runtime.spawn(async move {
            loop {
                let request = tokio::select! {
                    asked = asking.recv() => match asked {
                        Ok(request) => request,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => return,
                    },
                    () = tokio::time::sleep(good_for) => return,
                };
                let mut state = lock(&shared);
                state.now = Some(Pairing::Asked {
                    grant: request.grant_id,
                    device: request.device_name,
                    wants_control: request.wants_control,
                    wants_workflows: request.wants_workflows,
                    projects,
                });
                state.lapses = None;
                return;
            }
        });
        let mut state = lock(&self.pairing);
        state.now = Some(Pairing::Offered {
            code: offer.code,
            qr: qr(&offer.link),
            lapses_in_ms: good_for.as_millis() as u64,
        });
        state.lapses = Some(lapses);
        state.waiting = Some(waiting);
        drop(state);
        Ok(self.pairing())
    }

    fn answer(&self, answer: PairAnswer) -> Result<Pairing, String> {
        let device = match &lock(&self.pairing).now {
            Some(Pairing::Asked { grant, device, .. }) if *grant == answer.grant => device.clone(),
            _ => return Err("No device is asking now. Show a code again.".into()),
        };
        let allowed = answer.allow;
        let given = if allowed {
            Answer::Allow {
                control: answer.control,
                workflows: answer.workflows,
                projects: answer.projects,
            }
        } else {
            Answer::Deny
        };
        self.runtime
            .block_on(account::answer(self.home.as_ref(), answer.grant, given))
            .map_err(|f| f.message)?;
        lock(&self.pairing).now = Some(Pairing::Answered { device, allowed });
        Ok(self.pairing())
    }

    fn stop_pairing(&self) {
        let mut state = lock(&self.pairing);
        if let Some(waiting) = state.waiting.take() {
            waiting.abort();
        }
        *state = PairState::default();
    }

    fn signing_in(&self) -> SigningIn {
        lock(&self.signing_in)
            .now
            .clone()
            .unwrap_or(SigningIn::Idle)
    }

    fn sign_in(&self, server: &str, name: &str) -> Result<SigningIn, String> {
        if let Ok(Some(row)) = self.home.store.remote_host() {
            return Err(format!(
                "This computer is signed in already, as {} at {}. Sign it out first.",
                row.name, row.server_url
            ));
        }
        if server.is_empty() {
            return Err("Name the server, such as https://remote.yardsort.sh.".into());
        }
        let name = match name {
            "" => crate::sign_in::host_name().ok_or("Name this computer.")?,
            name => name.to_owned(),
        };
        if !lock(&self.signing_in).give_up() {
            return Err(FINISHING.into());
        }
        let (client, described) = self
            .runtime
            .block_on(account::prepare(server, &name))
            .map_err(|f| f.message)?;
        let sign_in = self
            .runtime
            .block_on(client.begin_sign_in())
            .map_err(|e| account::failed(e).message)?;
        let (pastes, mut pasted) = mpsc::unbounded_channel::<Paste>();
        let waiting = SigningIn::Waiting {
            url: sign_in.url().to_string(),
            port: sign_in.redirect_port(),
            problem: None,
        };
        let shared = self.signing_in.clone();
        let home = self.home.clone();
        // Held until the sign-in is in place, so that its task, which may be running by then,
        // finds it there. Another begun while the server was asked gives way, as one before did.
        let mut state = lock(&self.signing_in);
        if !state.give_up() {
            return Err(FINISHING.into());
        }
        let turn = state.turn;
        let task = self.runtime.spawn(async move {
            let code = {
                let returned = sign_in.returned(SIGN_IN_TIMEOUT);
                tokio::pin!(returned);
                loop {
                    tokio::select! {
                        code = &mut returned => break code.map_err(|e| account::failed(e).message),
                        paste = pasted.recv() => {
                            let Some((address, told)) = paste else { return };
                            match sign_in.pasted(&address) {
                                Ok(code) => {
                                    let _ = told.send(Ok(()));
                                    break Ok(code);
                                }
                                Err(error) => {
                                    let _ = told.send(Err(error.to_string()));
                                }
                            }
                        }
                    }
                }
            };
            // From here it is not given up: see `SignState::finishing`. Nothing is awaited
            // between the code and this, so a sign-in given up is either stopped before it or
            // told that it is too late.
            if code.is_ok() && !lock(&shared).finish(turn) {
                return;
            }
            let done = async {
                let code = code?;
                let device = NewDevice {
                    name: name.clone(),
                    kind: DeviceKind::Host,
                    platform: Platform::current(),
                };
                let signed_in = sign_in
                    .redeem(&client, device, code)
                    .await
                    .map_err(|e| account::failed(e).message)?;
                let row = account::enable(&home, client, &described, &name, signed_in)
                    .await
                    .map_err(|f| f.message)?;
                account::connect(&home);
                Ok::<_, String>(SigningIn::SignedIn {
                    name: row.name,
                    server_url: row.server_url,
                })
            }
            .await;
            lock(&shared).end(
                turn,
                done.unwrap_or_else(|message| SigningIn::Failed { message }),
            );
        });
        state.now = Some(waiting.clone());
        state.pastes = Some(pastes);
        state.waiting = Some(task);
        Ok(waiting)
    }

    fn pasted(&self, address: &str) -> Result<SigningIn, String> {
        let Some(pastes) = lock(&self.signing_in).pastes.clone() else {
            return Err("No sign-in is waiting. Start one again.".into());
        };
        let (told, answer) = oneshot::channel();
        pastes
            .send((address.trim().to_owned(), told))
            .map_err(|_| "No sign-in is waiting. Start one again.".to_owned())?;
        let taken = self
            .runtime
            .block_on(answer)
            .unwrap_or_else(|_| Err("The sign-in ended. Start one again.".into()));
        let mut state = lock(&self.signing_in);
        if let (Err(problem), Some(SigningIn::Waiting { problem: was, .. })) =
            (&taken, state.now.as_mut())
        {
            *was = Some(problem.clone());
        }
        drop(state);
        taken?;
        // Taken: redeeming it and registering the host take a moment, which the page waits out.
        for _ in 0..50 {
            if !matches!(self.signing_in(), SigningIn::Waiting { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(self.signing_in())
    }

    fn stop_signing_in(&self) {
        lock(&self.signing_in).give_up();
    }

    fn sign_out(&self) -> Result<(), String> {
        self.stop_pairing();
        self.stop_signing_in();
        let home = self.home.clone();
        self.runtime.block_on(async move {
            account::disconnect(home.as_ref()).await;
            account::disable(home.as_ref()).await.map_err(|f| f.message)
        })
    }
}

/// The QR code of `link`, a row of modules each, for the page to draw.
fn qr(link: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(link.as_bytes()) else {
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting() -> SigningIn {
        SigningIn::Waiting {
            url: "https://remote.example.test/v1/login".into(),
            port: 45999,
            problem: None,
        }
    }

    /// A sign-in in place, as `sign_in` leaves it: its turn.
    fn begun(state: &mut SignState) -> u64 {
        assert!(state.give_up());
        state.now = Some(waiting());
        state.turn
    }

    #[test]
    fn a_sign_in_given_up_while_it_waits_finishes_nothing_even_with_its_code_in_hand() {
        let mut state = SignState::default();
        let turn = begun(&mut state);
        assert!(state.give_up());
        assert!(state.now.is_none());
        // Its task had the code by then, and had not yet looked: it is refused, so nothing is
        // redeemed, registered or kept.
        assert!(!state.finish(turn));
        // Nor does what it would have ended as show, under the sign-in begun after it.
        let next = begun(&mut state);
        state.end(
            turn,
            SigningIn::Failed {
                message: "late".into(),
            },
        );
        assert!(matches!(state.now, Some(SigningIn::Waiting { .. })));
        assert!(state.finish(next));
    }

    #[test]
    fn a_sign_in_being_finished_is_not_given_up_and_says_so() {
        let mut state = SignState::default();
        let turn = begun(&mut state);
        assert!(state.finish(turn));
        // Neither by Cancel nor by another sign-in begun: it is left to finish.
        assert!(!state.give_up());
        assert_eq!(state.turn, turn);
        match &state.now {
            Some(SigningIn::Waiting { problem, .. }) => {
                assert_eq!(problem.as_deref(), Some(FINISHING));
            }
            other => panic!("{other:?}"),
        }
        // Done, it shows as it ended, and the next one can begin.
        state.end(
            turn,
            SigningIn::SignedIn {
                name: "Build box".into(),
                server_url: "https://remote.example.test".into(),
            },
        );
        assert!(matches!(state.now, Some(SigningIn::SignedIn { .. })));
        assert!(state.give_up());
    }

    #[test]
    fn a_qr_code_is_square_rows_of_modules() {
        let rows = qr("yardsort://pair?server=https%3A%2F%2Fremote.example.test&secret=abc");
        assert!(rows.len() >= 21, "{}", rows.len());
        assert!(rows.iter().all(|row| row.len() == rows.len()));
        assert!(rows
            .iter()
            .all(|row| row.chars().all(|c| c == '0' || c == '1')));
        // A finder pattern in the top left corner: seven dark modules across.
        assert_eq!(&rows[0][..7], "1111111");
    }
}

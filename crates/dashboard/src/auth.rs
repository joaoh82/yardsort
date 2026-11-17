//! Who may use the dashboard: a browser that was handed a sign-in code by the program serving it.
//!
//! A code is made only where a person is: `yardsort-host dashboard` at a terminal of their own, or
//! the app's own window. It is good once, for ten minutes, and buys a session, which the browser
//! keeps as a cookie for thirty days. Sessions outlast a restart, since an upgrade restarts the
//! host: they are kept by their hash, in a file only this user can read, so the file is no
//! session to whoever reads it.
//!
//! There is no password, so there is nothing to guess at leisure: a code has 80 bits of chance in
//! it and ten minutes to live.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a sign-in code is good for.
pub const CODE_GOOD_FOR: Duration = Duration::from_secs(10 * 60);
/// How long a browser stays signed in.
pub const SESSION_GOOD_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Codes waiting at once. More than a person asks for, and few enough that asking in a loop
/// fills nothing.
const CODES_KEPT: usize = 8;
/// Browsers signed in at once. The oldest goes when one more signs in.
const SESSIONS_KEPT: usize = 32;

/// Crockford's alphabet: no I, L, O or U, so a code read aloud or typed from a terminal is not
/// mistaken.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Sixteen characters of five bits each.
const CODE_LENGTH: usize = 16;

pub struct Auth {
    file: PathBuf,
    /// A few characters that are this dashboard's alone, for its cookie's name: a browser
    /// keeps one jar of cookies for `127.0.0.1` whatever the port, and two dashboards reached
    /// through two forwarded ports must not sign each other out.
    instance: String,
    state: Mutex<State>,
}

/// The file as it is kept.
#[derive(Default, Serialize, Deserialize)]
struct Kept {
    instance: String,
    sessions: Vec<Session>,
}

#[derive(Default)]
struct State {
    codes: Vec<Code>,
    sessions: Vec<Session>,
}

struct Code {
    hash: [u8; 32],
    lapses: Instant,
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    /// SHA-256 of the token, in hex.
    hash: String,
    /// When it was signed in, in milliseconds since 1970.
    since: i64,
}

impl Auth {
    /// The sessions kept in `dir`, which is made for this user alone. A file that cannot be read
    /// means nobody is signed in, which is always safe.
    pub fn open(dir: &Path) -> Self {
        let file = dir.join("sessions.json");
        let kept = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Kept>(&bytes).ok())
            .filter(|kept| {
                kept.instance.len() == 8 && kept.instance.bytes().all(|b| b.is_ascii_hexdigit())
            });
        let new = kept.is_none();
        let kept = kept.unwrap_or_else(|| {
            let mut bytes = [0u8; 4];
            fill(&mut bytes);
            Kept {
                instance: hex(&bytes),
                sessions: Vec::new(),
            }
        });
        let auth = Self {
            file,
            instance: kept.instance,
            state: Mutex::new(State {
                codes: Vec::new(),
                sessions: kept.sessions,
            }),
        };
        auth.lock().drop_lapsed(now_ms());
        if new {
            // So that the name is the same after a restart, whether or not anyone signs in.
            auth.save();
        }
        auth
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// A new sign-in code, as it is shown: `XXXX-XXXX-XXXX-XXXX`.
    pub fn offer(&self) -> String {
        let mut bytes = [0u8; CODE_LENGTH];
        fill(&mut bytes);
        let code: String = bytes
            .iter()
            .map(|byte| ALPHABET[usize::from(byte & 31)] as char)
            .collect();
        let mut state = self.lock();
        state.codes.retain(|code| code.lapses > Instant::now());
        if state.codes.len() >= CODES_KEPT {
            state.codes.remove(0);
        }
        state.codes.push(Code {
            hash: hash(&code),
            lapses: Instant::now() + CODE_GOOD_FOR,
        });
        code.as_bytes()
            .chunks(4)
            .map(|group| std::str::from_utf8(group).expect("the alphabet is ASCII"))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Spend a code, however it was typed, for a session's token. `None` for a code that was
    /// never offered, was used already or has lapsed.
    pub fn redeem(&self, typed: &str) -> Option<String> {
        let typed = hash(&normalized(typed)?);
        let token = {
            let mut state = self.lock();
            state.codes.retain(|code| code.lapses > Instant::now());
            let at = state
                .codes
                .iter()
                .position(|code| same(&code.hash, &typed))?;
            state.codes.remove(at);
            let mut bytes = [0u8; 32];
            fill(&mut bytes);
            let token = hex(&bytes);
            if state.sessions.len() >= SESSIONS_KEPT {
                state.sessions.remove(0);
            }
            state.sessions.push(Session {
                hash: hex(&hash(&token)),
                since: now_ms(),
            });
            token
        };
        self.save();
        Some(token)
    }

    /// Whether `token` is a session still in force.
    pub fn allows(&self, token: &str) -> bool {
        let wanted = hex(&hash(token));
        let now = now_ms();
        self.lock()
            .sessions
            .iter()
            .any(|session| !session.lapsed(now) && same(session.hash.as_bytes(), wanted.as_bytes()))
    }

    /// Sign one browser out.
    pub fn end(&self, token: &str) {
        let wanted = hex(&hash(token));
        self.lock()
            .sessions
            .retain(|session| session.hash != wanted);
        self.save();
    }

    /// Sign every browser out, and void every code. Says how many browsers that was.
    pub fn end_all(&self) -> usize {
        let ended = {
            let mut state = self.lock();
            state.codes.clear();
            let now = now_ms();
            let ended = state.sessions.iter().filter(|s| !s.lapsed(now)).count();
            state.sessions.clear();
            ended
        };
        self.save();
        ended
    }

    /// How many browsers are signed in.
    pub fn signed_in(&self) -> usize {
        let now = now_ms();
        self.lock()
            .sessions
            .iter()
            .filter(|s| !s.lapsed(now))
            .count()
    }

    fn save(&self) {
        let sessions = {
            let mut state = self.lock();
            state.drop_lapsed(now_ms());
            state.sessions.clone()
        };
        let text = serde_json::to_vec(&Kept {
            instance: self.instance.clone(),
            sessions,
        })
        .expect("sessions are plain data");
        if let Err(error) = write_private(&self.file, &text) {
            // Still signed in until this program stops; only the restart forgets it.
            eprintln!(
                "yardsort: the dashboard's sessions could not be kept in {}: {error}",
                self.file.display()
            );
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn drop_lapsed(&mut self, now: i64) {
        self.sessions.retain(|session| !session.lapsed(now));
    }
}

impl Session {
    fn lapsed(&self, now: i64) -> bool {
        // A clock set back does not make a session immortal: one from the future has lapsed too.
        let age = now - self.since;
        age < -60_000 || age > SESSION_GOOD_FOR.as_millis() as i64
    }
}

/// A typed code as it was made: upper case, without the dashes or spaces, and with the letters
/// the alphabet leaves out read as the digits they look like. `None` when it is not a code.
fn normalized(typed: &str) -> Option<String> {
    let code: String = typed
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\t' | '\n' | '\r'))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();
    (code.len() == CODE_LENGTH && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

fn hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Compared without stopping at the first difference. What is compared is a hash, so the time
/// taken says nothing of the code either way; this costs nothing and assumes less.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fill(bytes: &mut [u8]) {
    getrandom::fill(bytes).expect("the operating system's random numbers");
}

fn now_ms() -> i64 {
    yardsort_core::store::now_ms()
}

/// Write `contents` to `path`, in a folder and a file that only this user can read, made that
/// way from the start and renamed into place.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    make_private_dir(dir)?;
    let temporary = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
}

pub(crate) fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_buys_one_session_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::open(dir.path());
        let code = auth.offer();
        assert_eq!(code.len(), 19, "{code}");
        let token = auth.redeem(&code).expect("a code just offered");
        assert!(auth.allows(&token));
        assert!(auth.redeem(&code).is_none(), "a code is good once");
        assert!(!auth.allows("not a token"));
        assert_eq!(auth.signed_in(), 1);
    }

    #[test]
    fn a_code_is_taken_however_it_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::open(dir.path());
        let code = auth.offer();
        let typed = format!(" {} ", code.to_lowercase().replace('-', " "));
        assert!(auth.redeem(&typed).is_some());
        // The letters the alphabet leaves out are the digits they look like.
        assert_eq!(
            normalized("oooo-iiii-llll-0000").as_deref(),
            Some("0000111111110000")
        );
        assert_eq!(normalized("too short"), None);
        assert_eq!(normalized("UUUU-UUUU-UUUU-UUUU"), None);
    }

    #[test]
    fn a_code_nobody_was_given_buys_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::open(dir.path());
        auth.offer();
        assert!(auth.redeem("0000-0000-0000-0000").is_none());
        assert!(auth.redeem("").is_none());
        assert_eq!(auth.signed_in(), 0);
    }

    #[test]
    fn a_session_outlasts_a_restart_and_the_file_holds_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let token = {
            let auth = Auth::open(dir.path());
            let code = auth.offer();
            auth.redeem(&code).unwrap()
        };
        let kept = std::fs::read_to_string(dir.path().join("sessions.json")).unwrap();
        assert!(!kept.contains(&token), "kept by its hash alone");
        let auth = Auth::open(dir.path());
        assert!(auth.allows(&token));
        assert!(
            kept.contains(auth.instance()),
            "the same dashboard as before"
        );
        auth.end(&token);
        assert!(!auth.allows(&token));
        assert!(!Auth::open(dir.path()).allows(&token));
    }

    #[test]
    fn signing_everyone_out_voids_the_codes_too() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::open(dir.path());
        let first = auth.redeem(&auth.offer()).unwrap();
        let waiting = auth.offer();
        assert_eq!(auth.end_all(), 1);
        assert!(!auth.allows(&first));
        assert!(auth.redeem(&waiting).is_none());
    }

    #[test]
    fn a_session_lapses_after_thirty_days() {
        let now = now_ms();
        let session = |since| Session {
            hash: String::new(),
            since,
        };
        assert!(!session(now).lapsed(now));
        assert!(!session(now - 29 * 86_400_000).lapsed(now));
        assert!(session(now - 31 * 86_400_000).lapsed(now));
        assert!(session(now + 3_600_000).lapsed(now), "one from the future");
    }

    #[cfg(unix)]
    #[test]
    fn the_sessions_file_is_this_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("dashboard");
        let auth = Auth::open(&kept);
        auth.redeem(&auth.offer()).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&kept.join("sessions.json")), 0o600);
        assert_eq!(mode(&kept), 0o700);
    }
}

//! The dashboard's address beyond this computer: a rustunnel tunnel, kept up while it is wanted.
//!
//! The dashboard listens on 127.0.0.1 alone. To reach it from elsewhere without an SSH forward,
//! this runs the `rustunnel` client, which reaches out to a tunnel server and is given a public
//! `https://<subdomain>.…` address that the server forwards here. Yardsort keeps no token: the
//! client finds its own, in `~/.rustunnel/config.yml` (`rustunnel setup`) or `RUSTUNNEL_TOKEN`.
//!
//! The client is asked for its machine-readable events (`--json`), and those alone say how the
//! tunnel stands: the address once it is up, that it is reconnecting, or why it failed. It
//! reconnects by itself; when it ends instead, it is started again, at longer and longer waits.
//! It is ended when the tunnel is turned off, when this program stops, and, on Linux, when this
//! program dies without stopping.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use yardsort_core::env::ShellEnv;
use yardsort_core::program::Program;
use yardsort_core::settings::TunnelSettings;

/// The client's name on `PATH`.
pub const CLIENT: &str = "rustunnel";
/// The longest wait before starting the client again.
const LONGEST_WAIT: Duration = Duration::from_secs(300);
/// How long a client that has closed its output gets to end by itself.
const LINGERS: Duration = Duration::from_secs(5);

/// How the tunnel stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum State {
    /// Not wanted.
    Off,
    /// The client is starting, or connecting.
    Starting,
    /// Up, at this address.
    Ready { url: String },
    /// The client lost its server and is trying again by itself.
    Reconnecting { reason: String },
    /// The client ended. It is started again in a while.
    Failed {
        message: String,
        hint: Option<String>,
    },
    /// There is no `rustunnel` on this user's `PATH`.
    Missing,
}

pub struct Tunnel {
    shared: Arc<Shared>,
    first_wait: Duration,
}

struct Shared {
    state: Mutex<State>,
    /// The public host name, from the moment the tunnel was first up until it is turned off:
    /// kept through a reconnect, so a page open on it is not refused meanwhile.
    host: Mutex<Option<String>>,
    /// The client running now, with the run of the supervisor that started it. A supervisor
    /// only ever takes the one that is its own: after a restart, an earlier run that is still
    /// reading its dead client's last output would otherwise find the next run's client here.
    child: Mutex<Option<(u64, Child)>>,
    /// Which run of the supervisor is the current one. A stop, or a new start, moves it on.
    turn: AtomicU64,
}

impl Shared {
    fn set(&self, mine: u64, state: State) {
        // An old supervisor on its way out says nothing more.
        if self.turn.load(Ordering::SeqCst) == mine {
            *lock(&self.state) = state;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for Tunnel {
    fn default() -> Self {
        Self::with_first_wait(Duration::from_secs(5))
    }
}

impl Tunnel {
    pub fn with_first_wait(first_wait: Duration) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::Off),
                host: Mutex::new(None),
                child: Mutex::new(None),
                turn: AtomicU64::new(0),
            }),
            first_wait,
        }
    }

    pub fn state(&self) -> State {
        lock(&self.shared.state).clone()
    }

    /// The public host name requests through the tunnel are addressed to, while there is one.
    pub fn host(&self) -> Option<String> {
        lock(&self.shared.host).clone()
    }

    /// Make the tunnel as `wanted` says, to the dashboard on `port`: end the client running
    /// now, and start one if a tunnel is wanted.
    pub fn apply(&self, wanted: &TunnelSettings, port: u16, env: Arc<ShellEnv>) {
        self.stop();
        if !wanted.enabled {
            return;
        }
        let mine = self.shared.turn.load(Ordering::SeqCst);
        *lock(&self.shared.state) = State::Starting;
        let shared = self.shared.clone();
        let wanted = wanted.clone();
        let first_wait = self.first_wait;
        let spawned = std::thread::Builder::new()
            .name("dashboard-tunnel".into())
            .spawn(move || supervise(&shared, mine, &wanted, port, &env, first_wait));
        if let Err(error) = spawned {
            *lock(&self.shared.state) = State::Failed {
                message: format!("Could not start: {error}"),
                hint: None,
            };
        }
    }

    /// End the client, and say the tunnel is off.
    pub fn stop(&self) {
        self.shared.turn.fetch_add(1, Ordering::SeqCst);
        // Taken in a statement of its own: the client is ended with the lock let go.
        let running = lock(&self.shared.child).take();
        if let Some((_, mut child)) = running {
            let _ = child.kill();
            let _ = child.wait();
        }
        *lock(&self.shared.host) = None;
        *lock(&self.shared.state) = State::Off;
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check what a person typed for the tunnel, before any of it becomes an argument.
pub fn validate(wanted: &TunnelSettings) -> Result<(), String> {
    if let Some(subdomain) = &wanted.subdomain {
        let ok = (1..=63).contains(&subdomain.len())
            && subdomain
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !subdomain.starts_with('-')
            && !subdomain.ends_with('-');
        if !ok {
            return Err(
                "A subdomain is lower-case letters, digits and hyphens, such as build-box.".into(),
            );
        }
    }
    if let Some(region) = &wanted.region {
        if !matches!(region.as_str(), "auto" | "eu" | "us" | "ap") {
            return Err("A region is one of auto, eu, us or ap.".into());
        }
    }
    if let Some(server) = &wanted.server {
        let ok = server.len() <= 255
            && !server.starts_with('-')
            && server.rsplit_once(':').is_some_and(|(host, port)| {
                !host.is_empty()
                    && host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
                    && port.parse::<u16>().is_ok_and(|port| port > 0)
            });
        if !ok {
            return Err(
                "A tunnel server is a host and a port, such as tunnel.example.com:4040.".into(),
            );
        }
    }
    Ok(())
}

/// The client's arguments for a tunnel to the dashboard on `port`.
fn arguments(wanted: &TunnelSettings, port: u16) -> Vec<String> {
    let mut args: Vec<String> = [
        "http",
        &port.to_string(),
        // Not `localhost`, which may be ::1 first: the dashboard listens on 127.0.0.1 alone.
        "--local-host",
        "127.0.0.1",
        "--json",
        // Its request inspector is a second listener nobody asked for.
        "--no-inspect",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for (flag, value) in [
        ("--subdomain", &wanted.subdomain),
        ("--region", &wanted.region),
        ("--server", &wanted.server),
    ] {
        if let Some(value) = value {
            args.push(flag.into());
            args.push(value.clone());
        }
    }
    args
}

fn supervise(
    shared: &Arc<Shared>,
    mine: u64,
    wanted: &TunnelSettings,
    port: u16,
    env: &ShellEnv,
    first_wait: Duration,
) {
    let current = || shared.turn.load(Ordering::SeqCst) == mine;
    let mut wait = first_wait;
    while current() {
        let Some(program) = Program::find(env, CLIENT) else {
            shared.set(mine, State::Missing);
            return;
        };
        let home = env.home_dir().unwrap_or_else(std::env::temp_dir);
        let mut command = program.command(&home);
        command
            .args(arguments(wanted, port))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        die_with_parent(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                shared.set(
                    mine,
                    State::Failed {
                        message: format!("Could not run {}: {error}", program.path().display()),
                        hint: None,
                    },
                );
                return;
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        {
            let mut kept = lock(&shared.child);
            if !current() {
                // Turned off while it was starting: `stop` found no client to end.
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            *kept = Some((mine, child));
        }
        // What it says outside its events: kept for when it ends without one.
        let said = Arc::new(Mutex::new(String::new()));
        let saying = stderr.map(|mut stderr| {
            let said = said.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 2048];
                while let Ok(read @ 1..) = stderr.read(&mut buffer) {
                    let mut said = lock(&said);
                    said.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    let over = said.len().saturating_sub(2048);
                    if over > 0 {
                        let cut = (over..=said.len())
                            .find(|at| said.is_char_boundary(*at))
                            .unwrap_or(said.len());
                        said.drain(..cut);
                    }
                }
            })
        });

        let mut was_up = false;
        let mut failure: Option<State> = None;
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match event(&line) {
                    Some(Event::Ready { url }) => {
                        was_up = true;
                        if current() {
                            *lock(&shared.host) = host_of(&url);
                        }
                        shared.set(mine, State::Ready { url });
                    }
                    Some(Event::Reconnecting { reason }) => {
                        shared.set(mine, State::Reconnecting { reason });
                    }
                    Some(Event::Error { message, hint }) => {
                        failure = Some(State::Failed { message, hint });
                    }
                    None => {}
                }
            }
        }
        // Its output closed: it has ended, or is about to. Only this run's own client is
        // waited for, and never with the lock held, so that `stop` can always end what runs.
        let closed = std::time::Instant::now();
        let status = loop {
            {
                let mut kept = lock(&shared.child);
                let Some((_, child)) = kept.as_mut().filter(|(turn, _)| *turn == mine) else {
                    // Stopped, or replaced: whatever is here now is not this run's.
                    break None;
                };
                match child.try_wait() {
                    Ok(None) if closed.elapsed() < LINGERS => {}
                    // One that closed its output and stays is no tunnel: ended, and tried again.
                    Ok(None) => {
                        let _ = child.kill();
                        break kept.take().map(|(_, mut child)| child.wait());
                    }
                    Ok(Some(status)) => {
                        kept.take();
                        break Some(Ok(status));
                    }
                    Err(error) => {
                        kept.take();
                        break Some(Err(error));
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // What it said is read as it comes; something it started may hold that open for good.
        if let Some(saying) = saying {
            let since = std::time::Instant::now();
            while !saying.is_finished() && since.elapsed() < Duration::from_millis(500) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if !current() {
            return;
        }
        let failure = failure.unwrap_or_else(|| {
            let said = lock(&said).trim().to_owned();
            State::Failed {
                message: match (said.is_empty(), status) {
                    (false, _) => said.lines().last().unwrap_or_default().trim().to_owned(),
                    (true, Some(Ok(status))) => format!("rustunnel ended ({status})."),
                    (true, _) => "rustunnel ended.".to_owned(),
                },
                hint: None,
            }
        });
        shared.set(mine, failure);
        // One that was up starts over from the short wait; one that never was waits longer
        // each time, so a wrong token is not tried all day at full speed.
        if was_up {
            wait = first_wait;
        }
        let waited = std::time::Instant::now();
        while current() && waited.elapsed() < wait {
            std::thread::sleep(Duration::from_millis(50).min(wait));
        }
        wait = (wait * 3).min(LONGEST_WAIT);
        if current() {
            shared.set(mine, State::Starting);
        }
    }
}

enum Event {
    Ready {
        url: String,
    },
    Reconnecting {
        reason: String,
    },
    Error {
        message: String,
        hint: Option<String>,
    },
}

/// One line of the client's `--json` output, when it is an event that changes how the tunnel
/// stands. Its other events (`reconnected`, which a `tunnel_ready` follows; notices) change
/// nothing here.
fn event(line: &str) -> Option<Event> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
    match value.get("event")?.as_str()? {
        "tunnel_ready" => Some(Event::Ready {
            url: text("public_url")?,
        }),
        "reconnecting" => Some(Event::Reconnecting {
            reason: text("reason").unwrap_or_default(),
        }),
        "error" => Some(Event::Error {
            message: text("message").unwrap_or_else(|| "rustunnel failed.".into()),
            hint: text("hint"),
        }),
        _ => None,
    }
}

/// The host name in `https://name.example.com[:port][/…]`, in lower case.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Have the client end when this program dies, however it dies. Linux alone can ask that; the
/// thread that starts the client lives as long as the client does, which is what makes it mean
/// "this program" rather than "this thread".
fn die_with_parent(command: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `prctl` is async-signal-safe, and this runs in the child between the fork and
        // the exec, which is what `pre_exec` is for.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clients_events_say_how_the_tunnel_stands() {
        let ready = event(
            r#"{"event":"tunnel_ready","protocol":"http","public_url":"https://build-box.eu.edge.rustunnel.com","local_port":28680,"local_host":"127.0.0.1"}"#,
        );
        assert!(
            matches!(ready, Some(Event::Ready { ref url }) if url == "https://build-box.eu.edge.rustunnel.com")
        );
        assert!(matches!(
            event(r#"{"event":"reconnecting","attempt":1,"reason":"connection reset","delay_secs":1.0}"#),
            Some(Event::Reconnecting { ref reason }) if reason == "connection reset"
        ));
        assert!(matches!(
            event(r#"{"event":"error","code":"auth","message":"Invalid token","hint":"Run rustunnel setup"}"#),
            Some(Event::Error { ref message, hint: Some(ref hint) })
                if message == "Invalid token" && hint == "Run rustunnel setup"
        ));
        assert!(event(r#"{"event":"reconnected"}"#).is_none());
        assert!(event(r#"{"event":"notice","message":"80% of the plan used"}"#).is_none());
        assert!(event("not json").is_none());
    }

    #[test]
    fn the_public_host_is_read_out_of_the_address() {
        assert_eq!(
            host_of("https://Build-Box.eu.edge.rustunnel.com").as_deref(),
            Some("build-box.eu.edge.rustunnel.com")
        );
        assert_eq!(
            host_of("https://box.tunnel.example.com:8443/path?x=1").as_deref(),
            Some("box.tunnel.example.com")
        );
        assert_eq!(host_of("no scheme"), None);
    }

    #[test]
    fn the_client_is_pointed_at_the_loopback_port_and_asked_for_events() {
        let wanted = TunnelSettings {
            enabled: true,
            subdomain: Some("build-box".into()),
            region: Some("eu".into()),
            server: None,
        };
        assert_eq!(
            arguments(&wanted, 28680).join(" "),
            "http 28680 --local-host 127.0.0.1 --json --no-inspect --subdomain build-box --region eu"
        );
    }

    #[test]
    fn nothing_typed_for_the_tunnel_can_become_an_option() {
        let with = |subdomain: Option<&str>, region: Option<&str>, server: Option<&str>| {
            validate(&TunnelSettings {
                enabled: true,
                subdomain: subdomain.map(str::to_owned),
                region: region.map(str::to_owned),
                server: server.map(str::to_owned),
            })
        };
        assert!(with(
            Some("build-box"),
            Some("eu"),
            Some("tunnel.example.com:4040")
        )
        .is_ok());
        assert!(with(None, None, None).is_ok());
        for bad in ["--token", "-x", "x-", "Build", "a b", "a.b", ""] {
            assert!(with(Some(bad), None, None).is_err(), "{bad:?}");
        }
        assert!(with(None, Some("mars"), None).is_err());
        for bad in [
            "--insecure",
            "example.com",
            "example.com:0",
            ":4040",
            "a b:1",
            "x/y:1",
        ] {
            assert!(with(None, None, Some(bad)).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    mod with_a_stand_in {
        use super::super::*;
        use std::collections::BTreeMap;
        use std::path::Path;
        use std::time::Instant;
        use yardsort_core::env::EnvSource;

        /// An environment whose `PATH` is `bin` and the system's own, and whose home is `home`.
        fn env(bin: &Path, home: &Path) -> Arc<ShellEnv> {
            Arc::new(ShellEnv {
                vars: BTreeMap::from([
                    (
                        "PATH".to_owned(),
                        format!("{}:/usr/bin:/bin", bin.display()),
                    ),
                    ("HOME".to_owned(), home.display().to_string()),
                ]),
                source: EnvSource::Process,
                warning: None,
            })
        }

        fn until(tunnel: &Tunnel, what: &str, is: impl Fn(&State) -> bool) -> State {
            let since = Instant::now();
            loop {
                let state = tunnel.state();
                if is(&state) {
                    return state;
                }
                assert!(
                    since.elapsed() < Duration::from_secs(20),
                    "waiting for {what}, the tunnel is {state:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn on() -> TunnelSettings {
            TunnelSettings {
                enabled: true,
                subdomain: Some("build-box".into()),
                ..TunnelSettings::default()
            }
        }

        #[test]
        fn the_tunnel_is_up_at_the_address_the_client_says_and_ends_with_it() {
            let dir = tempfile::tempdir().unwrap();
            let pid = dir.path().join("pid");
            // Says what it was run with, then that it is up, and stays.
            yardsort_core::program::testing::executable(
                dir.path().join("rustunnel"),
                format!(
                    "#!/bin/sh\necho $$ > {pid}\necho \"$*\" > {args}\n\
                     echo '{{\"event\":\"tunnel_ready\",\"public_url\":\"https://build-box.edge.example.test\"}}'\n\
                     exec sleep 600\n",
                    pid = pid.display(),
                    args = dir.path().join("args").display(),
                ),
            );
            let tunnel = Tunnel::default();
            assert_eq!(tunnel.state(), State::Off);
            tunnel.apply(&on(), 28680, env(dir.path(), dir.path()));
            let state = until(&tunnel, "ready", |s| matches!(s, State::Ready { .. }));
            assert_eq!(
                state,
                State::Ready {
                    url: "https://build-box.edge.example.test".into()
                }
            );
            assert_eq!(
                tunnel.host().as_deref(),
                Some("build-box.edge.example.test")
            );
            let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
            assert_eq!(
                args.trim(),
                "http 28680 --local-host 127.0.0.1 --json --no-inspect --subdomain build-box"
            );

            let client: i32 = std::fs::read_to_string(&pid)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            tunnel.stop();
            assert_eq!(tunnel.state(), State::Off);
            assert_eq!(tunnel.host(), None);
            // SAFETY: signal 0 only asks whether the process is there.
            assert_ne!(
                unsafe { libc::kill(client, 0) },
                0,
                "the client outlived the tunnel"
            );
        }

        /// Turned off and on again while the first client's output is still open, held by
        /// something it started: the first supervisor is still reading when the second client is
        /// in place. When that output closes, the first supervisor must leave the second client
        /// alone. It used to take it and wait for it, with the lock `stop` needs.
        #[test]
        fn a_supervisor_from_before_a_restart_leaves_the_new_client_alone() {
            let dir = tempfile::tempdir().unwrap();
            let pids = dir.path().join("pids");
            yardsort_core::program::testing::executable(
                dir.path().join("rustunnel"),
                format!(
                    "#!/bin/sh\necho $$ >> {pids}\n\
                     echo '{{\"event\":\"tunnel_ready\",\"public_url\":\"https://b.example.test\"}}'\n\
                     sleep 1.5 &\nexec sleep 600\n",
                    pids = pids.display(),
                ),
            );
            let clients = || -> Vec<i32> {
                std::fs::read_to_string(&pids)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| line.trim().parse().ok())
                    .collect()
            };
            // SAFETY: signal 0 only asks whether the process is there.
            let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
            let ready = |s: &State| matches!(s, State::Ready { .. });
            let tunnel = Arc::new(Tunnel::default());
            let env = env(dir.path(), dir.path());

            tunnel.apply(&on(), 28680, env.clone());
            until(&tunnel, "the first client", ready);
            tunnel.apply(&on(), 28680, env);
            until(&tunnel, "the second client", ready);
            let since = Instant::now();
            while clients().len() < 2 {
                assert!(since.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(20));
            }
            let [first, second] = clients()[..] else {
                panic!("two clients were started: {:?}", clients());
            };
            assert!(!alive(first), "the first client was ended by the restart");

            // Past the moment the first client's output closes, and its supervisor wakes.
            std::thread::sleep(Duration::from_millis(2200));
            assert!(ready(&tunnel.state()), "{:?}", tunnel.state());
            assert!(
                alive(second),
                "the second client was ended by the first supervisor"
            );

            let (done, stopped) = std::sync::mpsc::channel();
            let stopping = tunnel.clone();
            std::thread::spawn(move || {
                stopping.stop();
                let _ = done.send(());
            });
            assert!(
                stopped.recv_timeout(Duration::from_secs(3)).is_ok(),
                "stopping the tunnel waited on a client it could not reach"
            );
            assert_eq!(tunnel.state(), State::Off);
            assert!(!alive(second), "the second client outlived the tunnel");
        }

        #[test]
        fn a_client_that_fails_says_why_and_is_started_again() {
            let dir = tempfile::tempdir().unwrap();
            let runs = dir.path().join("runs");
            // Fails the first time, as with a wrong token; is up the second.
            yardsort_core::program::testing::executable(
                dir.path().join("rustunnel"),
                format!(
                    "#!/bin/sh\nif [ -e {runs} ]; then\n\
                     echo '{{\"event\":\"tunnel_ready\",\"public_url\":\"https://b.example.test\"}}'\n\
                     exec sleep 600\nfi\ntouch {runs}\n\
                     echo '{{\"event\":\"error\",\"code\":\"auth\",\"message\":\"Invalid token\",\"hint\":\"Run rustunnel setup\"}}'\n\
                     exit 1\n",
                    runs = runs.display(),
                ),
            );
            let tunnel = Tunnel::with_first_wait(Duration::from_millis(300));
            tunnel.apply(&on(), 28680, env(dir.path(), dir.path()));
            let failed = until(&tunnel, "the failure", |s| {
                matches!(s, State::Failed { .. })
            });
            assert_eq!(
                failed,
                State::Failed {
                    message: "Invalid token".into(),
                    hint: Some("Run rustunnel setup".into())
                }
            );
            until(&tunnel, "the second start", |s| {
                matches!(s, State::Ready { .. })
            });
        }

        #[test]
        fn a_client_that_ends_without_a_word_is_reported_by_what_it_printed() {
            let dir = tempfile::tempdir().unwrap();
            yardsort_core::program::testing::executable(
                dir.path().join("rustunnel"),
                "#!/bin/sh\necho 'first line' >&2\necho 'no config file' >&2\nexit 2\n",
            );
            let tunnel = Tunnel::with_first_wait(Duration::from_secs(60));
            tunnel.apply(&on(), 28680, env(dir.path(), dir.path()));
            let failed = until(&tunnel, "the failure", |s| {
                matches!(s, State::Failed { .. })
            });
            assert_eq!(
                failed,
                State::Failed {
                    message: "no config file".into(),
                    hint: None
                }
            );
        }

        #[test]
        fn without_the_client_the_tunnel_says_it_is_missing() {
            let dir = tempfile::tempdir().unwrap();
            let tunnel = Tunnel::default();
            let nowhere = Arc::new(ShellEnv {
                vars: BTreeMap::from([("PATH".to_owned(), dir.path().display().to_string())]),
                source: EnvSource::Process,
                warning: None,
            });
            tunnel.apply(&on(), 28680, nowhere);
            until(&tunnel, "missing", |s| *s == State::Missing);
        }

        #[test]
        fn a_tunnel_not_wanted_starts_nothing() {
            let dir = tempfile::tempdir().unwrap();
            yardsort_core::program::testing::executable(
                dir.path().join("rustunnel"),
                format!("#!/bin/sh\ntouch {}\n", dir.path().join("ran").display()),
            );
            let tunnel = Tunnel::default();
            tunnel.apply(
                &TunnelSettings::default(),
                28680,
                env(dir.path(), dir.path()),
            );
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(tunnel.state(), State::Off);
            assert!(!dir.path().join("ran").exists());
        }
    }
}

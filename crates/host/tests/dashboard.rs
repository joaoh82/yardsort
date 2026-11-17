//! The dashboard as `yardsort-host` serves it: the real program, on a profile of its own, asked
//! by its own commands and over HTTP. `serve` serves it unless the settings say not to, says
//! where, and never hands a way in to anything that is not a person at a terminal: which a test
//! is not, so every command here that would open it up is refused, as it would be for an agent.
#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// A profile, a home and a runtime folder of their own, and the host running on them.
struct Host {
    dir: tempfile::TempDir,
    serve: Option<Child>,
    port: u16,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Host {
    /// A profile whose settings are `settings`, with `{port}` a port that is free.
    fn new(settings: &str) -> Self {
        // Short, for the sockets in it: a socket's path has little room.
        let dir = tempfile::Builder::new()
            .prefix("ysd")
            .tempdir_in("/tmp")
            .or_else(|_| tempfile::tempdir())
            .unwrap();
        let port = free_port();
        for folder in ["data", "home", "run"] {
            std::fs::create_dir(dir.path().join(folder)).unwrap();
        }
        std::fs::write(
            dir.path().join("data/settings.toml"),
            settings.replace("{port}", &port.to_string()),
        )
        .unwrap();
        Self {
            dir,
            serve: None,
            port,
        }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// `yardsort-host`, told nothing of the machine it is tested on: no display, no agent's
    /// markers, a plain shell for a login shell.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yardsort-host"));
        command
            .env_clear()
            .env("HOME", self.dir.path().join("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("YARDSORT_DATA_DIR", self.data())
            .env("XDG_RUNTIME_DIR", self.dir.path().join("run"))
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .output()
            .expect("yardsort-host runs")
    }

    /// Start `serve`, and wait until it has said everything it says at its start.
    fn serve(&mut self) -> String {
        let log = self.dir.path().join("serve.log");
        let out = std::fs::File::create(&log).unwrap();
        let child = self
            .command()
            .arg("serve")
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .expect("serve starts");
        self.serve = Some(child);
        let since = Instant::now();
        loop {
            // `status` answers once the control socket is up, which is before the dashboard is.
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let settled = said.contains("Dashboard at")
                || said.contains("The dashboard is not served")
                || (said.contains("Serving") && self.status().contains("Dashboard: off"));
            if settled {
                return said;
            }
            assert!(
                since.elapsed() < Duration::from_secs(60),
                "serve said: {said}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn status(&self) -> String {
        String::from_utf8_lossy(&self.run(&["status"]).stdout).into_owned()
    }

    /// The status line of an answer on `port`, or `None` when nothing listens there.
    fn get(&self, port: u16, path: &str) -> Option<String> {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream
            .write_all(
                format!(
                    "GET {path} HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\nconnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .ok()?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).ok()?;
        answer.lines().next().map(str::to_owned)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(mut serve) = self.serve.take() {
            // SAFETY: a signal to a process this test started.
            unsafe {
                libc::kill(serve.id() as libc::pid_t, libc::SIGTERM);
            }
            let _ = serve.wait();
        }
        // `serve` leaves its terminal daemon running, as it should: the test ends it.
        end_daemon(&self.data());
    }
}

/// End the terminal daemon that was started for `data_dir`, found by the profile it was told:
/// this process's own idea of where its socket is would be another profile's.
fn end_daemon(data_dir: &Path) {
    let profile = data_dir.to_string_lossy().into_owned();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return;
    };
    for pid in processes.filter_map(|entry| {
        entry
            .ok()?
            .file_name()
            .to_str()?
            .parse::<libc::pid_t>()
            .ok()
    }) {
        let line = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let line = String::from_utf8_lossy(&line);
        if line.contains("--yardsort-daemon") && line.contains(&profile) {
            // SAFETY: a signal to a daemon this test's own host started.
            unsafe {
                libc::kill(pid, libc::SIGHUP);
            }
        }
    }
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn serve_serves_the_dashboard_on_loopback_and_says_where() {
    let mut host = Host::new("[dashboard]\nport = {port}\n");
    let started = host.serve();
    let address = format!("http://127.0.0.1:{}", host.port);
    assert!(
        started.contains(&format!(
            "Dashboard at {address}. Sign in with `yardsort-host dashboard`."
        )),
        "{started}"
    );
    assert_eq!(host.get(host.port, "/").as_deref(), Some("HTTP/1.1 200 OK"));
    // Nothing of the host is told to a browser that has not signed in.
    assert_eq!(
        host.get(host.port, "/api/status").as_deref(),
        Some("HTTP/1.1 401 Unauthorized")
    );
    let status = host.status();
    assert!(
        status.contains(&format!("Dashboard: {address}, 0 browsers signed in.")),
        "{status}"
    );
    assert!(status.contains("Tunnel: off"), "{status}");
    // Its sessions are kept in the profile, for this user alone.
    use std::os::unix::fs::PermissionsExt;
    let kept = host.data().join("dashboard");
    let mode = std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "{}", kept.display());
}

#[test]
fn nothing_but_a_person_at_a_terminal_opens_the_dashboard_up() {
    let mut host = Host::new("[dashboard]\nport = {port}\n");
    host.serve();
    // Asked by a program with no terminal, as an agent's tool or a script would ask.
    for opens in [
        &["dashboard"][..],
        &["dashboard", "on"],
        &["dashboard", "tunnel", "on", "--subdomain", "build-box"],
    ] {
        let refused = host.run(opens);
        assert!(!refused.status.success(), "{opens:?}");
        let why = said(&refused);
        assert!(
            why.contains("opened up only by a person"),
            "{opens:?}: {why}"
        );
        // No code, and no address with one in it.
        assert!(!why.contains("#code="), "{why}");
    }
    let settings = std::fs::read_to_string(host.data().join("settings.toml")).unwrap();
    assert!(!settings.contains("tunnel"), "{settings}");
}

#[test]
fn closing_the_dashboard_needs_no_person_and_is_kept_for_the_next_start() {
    let mut host = Host::new("[dashboard]\nport = {port}\n");
    host.serve();
    let out = host.run(&["dashboard", "sign-out"]);
    assert!(out.status.success(), "{}", said(&out));
    assert!(
        said(&out).contains("No browser was signed in"),
        "{}",
        said(&out)
    );
    let out = host.run(&["dashboard", "tunnel", "off"]);
    assert!(said(&out).contains("The tunnel is off"), "{}", said(&out));

    let out = host.run(&["dashboard", "off"]);
    assert!(out.status.success(), "{}", said(&out));
    assert!(
        said(&out).contains("The dashboard is off"),
        "{}",
        said(&out)
    );
    assert_eq!(host.get(host.port, "/"), None, "still listening");
    assert!(host.status().contains("Dashboard: off."));
    let settings = std::fs::read_to_string(host.data().join("settings.toml")).unwrap();
    assert!(
        settings.contains("[dashboard]\nenabled = false"),
        "{settings}"
    );
    // Off, its other commands say so rather than fail oddly.
    let out = host.run(&["dashboard", "sign-out"]);
    assert!(
        said(&out).contains("The dashboard is off"),
        "{}",
        said(&out)
    );
}

#[test]
fn a_host_told_not_to_serve_the_dashboard_listens_nowhere() {
    let mut host = Host::new("[dashboard]\nenabled = false\nport = {port}\n");
    let started = host.serve();
    assert!(!started.contains("Dashboard"), "{started}");
    assert_eq!(host.get(host.port, "/"), None);
    assert!(host.status().contains("Dashboard: off."));
    assert!(!host.data().join("dashboard").exists());
}

#[test]
fn a_host_whose_ports_are_all_taken_serves_on_without_the_dashboard() {
    let mut host = Host::new("[dashboard]\nport = {port}\n");
    let taken: Vec<_> = (host.port..host.port.saturating_add(10))
        .filter_map(|port| std::net::TcpListener::bind(("127.0.0.1", port)).ok())
        .collect();
    if taken.len() < 10 {
        // Something else holds one of them: the dashboard would be right to take the gap.
        return;
    }
    let started = host.serve();
    assert!(started.contains("The dashboard is not served"), "{started}");
    assert!(started.contains("[dashboard]"), "{started}");
    // The host itself is up all the same.
    assert!(host.status().contains("Host: running"), "{}", host.status());
}

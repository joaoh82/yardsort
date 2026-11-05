//! Start a daemon from another build, with an agent and a shell left running in it, the way an
//! update finds them.
//!
//! ```sh
//! cargo run -p pty-ipc --example leave_running -- <data dir> <that build's yardsort>
//! ```
//!
//! For the smoke test of replacing a daemon that cannot share its terminals, in
//! `.yardsort/tests/remote-earlier-daemon.yaml`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pty_host::{LaunchPlan, TermSize, TerminalHost};
use pty_ipc::{DaemonClient, Endpoint};

fn sleeping(labels: &[(&str, &str)]) -> LaunchPlan {
    LaunchPlan {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 3600".into()],
        cwd: None,
        env: vec![],
        clear_env: false,
        size: TermSize { cols: 80, rows: 24 },
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        prompt: None,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(data_dir), Some(build)) = (args.next(), args.next()) else {
        eprintln!("usage: leave_running <data dir> <that build's yardsort>");
        std::process::exit(2);
    };
    let data_dir = Path::new(&data_dir);
    let endpoint = Endpoint::for_data_dir(data_dir);
    let spool = data_dir.join("activity/spool");
    std::fs::create_dir_all(&spool).unwrap();
    // Never waited on: it is to outlive this program, as a daemon does the app that started it.
    #[allow(clippy::zombie_processes)]
    Command::new(build)
        .arg("--yardsort-daemon")
        .arg(endpoint.as_os_str())
        .arg(&spool)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let client = loop {
        match DaemonClient::connect(&endpoint, "leave_running", Arc::new(|_| {})) {
            Ok(client) => break client,
            Err(error) if Instant::now() > deadline => panic!("no daemon: {error}"),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let info = client.daemon_info();
    println!("daemon {} protocol {}", info.version, info.protocol);
    client.spawn(sleeping(&[("harness", "claude")])).unwrap();
    client.spawn(sleeping(&[])).unwrap();
    println!("left running: an agent and a shell");
}

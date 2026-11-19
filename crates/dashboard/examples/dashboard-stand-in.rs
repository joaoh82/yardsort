//! The dashboard, served with a stand-in host, to look at the page without a host of your own:
//!
//! ```sh
//! cargo run -p host-dashboard --example dashboard-stand-in -- /tmp/dash
//! ```
//!
//! It prints a sign-in link, and one more each time Enter is pressed. With `--remote`, the page's
//! Remote tab works as a host with no screen's does, and a line `ask` makes a pretend phone ask
//! to be let in; `--remote-off` is the same with this host not signed in yet. The folder given is its
//! home: settings, sessions and whatever a command run from the page leaves there. Programs are
//! looked for in `<folder>/bin` and then in /usr/bin and /bin, so a stand-in `gh`, `claude` or
//! `rustunnel` put in `bin` is the one the page finds. Used for the screenshots in the guide,
//! which must show nobody's real names or paths (`scripts/screenshots.sh`).

#[path = "../tests/common/mod.rs"]
mod common;

use std::io::BufRead;
use std::sync::Arc;

use host_dashboard::{Dashboard, Options, Project};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let home = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("a folder for the stand-in's home");
    let home = std::path::absolute(&home).expect("a path");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("the folder can be made");
    let flags: Vec<String> = std::env::args().skip(2).collect();
    let mut source = common::StandIn::new(&home, &bin);
    if flags.iter().any(|f| f.starts_with("--remote")) {
        source.remote = Some(common::StandInRemote::default());
    }
    if flags.iter().any(|f| f == "--remote-off") {
        source.about.remote = None;
    }
    let source = Arc::new(source);
    *source.projects.lock().unwrap() = ["my-service", "website"]
        .into_iter()
        .map(|name| Project {
            id: format!("project-{name}"),
            name: name.to_owned(),
            path: home.join("Projects").join(name).display().to_string(),
        })
        .collect();
    let dashboard = Dashboard::start(
        source.clone(),
        Options {
            state_dir: home.join("dashboard"),
        },
    )
    .await
    .expect("the dashboard starts");
    println!("{}", dashboard.sign_in().local);
    let (again, mut asked) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if again.send(line).is_err() {
                break;
            }
        }
    });
    // Until its input closes: each line is a request for one more link, or a phone asking.
    while let Some(line) = asked.recv().await {
        if line.trim() == "ask" {
            if let Some(remote) = &source.remote {
                remote.asks("Ana's phone", true);
            }
            continue;
        }
        let sign_in = dashboard.sign_in();
        println!("{}", sign_in.public.unwrap_or(sign_in.local));
    }
    dashboard.stop().await;
}

//! What the host ships with: the systemd user unit, the image and its compose example. Each line
//! checked here is one the guide relies on, and a change to it would cost someone their agents or
//! open the host up.

use std::path::Path;

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The lines of a file that are not comments.
fn said(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

#[test]
fn stopping_the_service_stops_serve_and_leaves_the_agents_running() {
    let unit = read("packaging/yardsort-host.service");
    let lines = said(&unit);
    // Without it, systemd ends every process in the unit, the terminal daemon and the agents too.
    assert!(lines.contains(&"KillMode=process"), "{unit}");
    // Where the guide and the tarball's README install it.
    assert!(
        lines.contains(&"ExecStart=%h/.local/bin/yardsort-host serve"),
        "{unit}"
    );
    assert!(read("packaging/README.md").contains("-t ~/.local/bin/"));
    // A user unit: started with the user's manager, which linger keeps up.
    assert!(lines.contains(&"WantedBy=default.target"), "{unit}");
    assert!(
        !lines.iter().any(|line| line.starts_with("User=")),
        "{unit}"
    );
}

#[test]
fn the_image_runs_as_an_ordinary_user_under_tini_and_listens_nowhere() {
    let dockerfile = read("Dockerfile");
    let lines = said(&dockerfile);
    let runtime = lines
        .iter()
        .rposition(|line| line.starts_with("FROM "))
        .expect("a FROM");
    let runtime = &lines[runtime..];
    assert!(runtime.contains(&"USER yardsort"), "{dockerfile}");
    assert!(
        runtime
            .iter()
            .any(|line| line.starts_with("ENTRYPOINT [\"/usr/bin/tini\", \"--\"")),
        "{dockerfile}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("EXPOSE")),
        "{dockerfile}"
    );
    // Pinned bases: a build months from now is the same build.
    for from in lines.iter().filter(|line| line.starts_with("FROM ")) {
        assert!(from.contains("@sha256:"), "not pinned: {from}");
    }

    let compose = read("compose.yaml");
    let lines = said(&compose);
    assert!(
        !lines.iter().any(|line| line.starts_with("ports:")),
        "{compose}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("privileged:")),
        "{compose}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("docker.sock")),
        "{compose}"
    );
}

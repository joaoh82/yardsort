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

#[test]
fn the_full_image_is_the_slim_one_with_checked_downloads_and_runs_as_its_user() {
    let dockerfile = read("Dockerfile.full");
    let lines = said(&dockerfile);
    let froms: Vec<&&str> = lines
        .iter()
        .filter(|line| line.starts_with("FROM "))
        .collect();
    // On the slim image, so it keeps its user, tini and its entry point.
    assert_eq!(froms, [&"FROM ${SLIM}"], "{dockerfile}");
    assert!(!lines.iter().any(|line| line.starts_with("ENTRYPOINT")
        || line.starts_with("CMD")
        || line.starts_with("EXPOSE")));
    let last_user = lines
        .iter()
        .rev()
        .find(|line| line.starts_with("USER "))
        .expect("a USER");
    assert_eq!(*last_user, "USER yardsort", "{dockerfile}");
    // Nothing piped from the network into a shell; every download through `fetch`, which checks
    // it, and each `fetch` given a checksum.
    let piped_into_a_shell = dockerfile
        .split('|')
        .skip(1)
        .any(|after| matches!(after.split_whitespace().next(), Some("sh" | "bash")));
    assert!(!piped_into_a_shell, "{dockerfile}");
    let curls = dockerfile.matches("curl -fsSL").count();
    assert_eq!(curls, 1, "only fetch itself downloads:\n{dockerfile}");
    for line in dockerfile.lines().filter(|line| line.contains("&& fetch ")) {
        assert!(line.contains("\"$sum\""), "unchecked: {line}");
    }
    for line in dockerfile.lines().filter(|line| line.contains(" sum=")) {
        let sum = line.split(" sum=").nth(1).unwrap();
        let sum: String = sum.chars().take_while(char::is_ascii_hexdigit).collect();
        assert_eq!(sum.len(), 64, "not a sha256: {line}");
    }
    // The harnesses from the lockfile, as it pins them, not whatever is newest.
    assert!(
        lines.iter().any(|line| line.contains("npm ci ")),
        "{dockerfile}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("npm install")),
        "{dockerfile}"
    );
}

/// The image ships only what anyone may redistribute: each package in the harnesses' tree has a
/// licence that allows it, and the four harnesses are pinned to one version.
#[test]
fn every_package_the_full_image_ships_may_be_redistributed() {
    let manifest: serde_json::Value = serde_json::from_str(&read("agents/package.json")).unwrap();
    let lock: serde_json::Value = serde_json::from_str(&read("agents/package-lock.json")).unwrap();
    let packages = lock["packages"].as_object().expect("packages");
    for (name, version) in manifest["dependencies"].as_object().unwrap() {
        let version = version.as_str().unwrap();
        assert!(
            version.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "{name} is not pinned: {version}"
        );
        let locked = &packages[&format!("node_modules/{name}")];
        assert_eq!(locked["version"].as_str(), Some(version), "{name}");
    }
    const ALLOWED: [&str; 13] = [
        "MIT",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "ISC",
        "0BSD",
        "BlueOak-1.0.0",
        "Unlicense",
        "CC0-1.0",
        "MPL-2.0",
        "LGPL-3.0-or-later",
        "Python-2.0",
        "Zlib",
    ];
    for (path, package) in packages.iter().filter(|(path, _)| !path.is_empty()) {
        let licence = package["license"]
            .as_str()
            .unwrap_or_else(|| panic!("{path} names no licence"));
        let parts = licence
            .trim_matches(|c| c == '(' || c == ')')
            .split([' ', '(', ')'])
            .filter(|word| !matches!(*word, "" | "AND" | "OR"));
        for part in parts {
            assert!(ALLOWED.contains(&part), "{path}: {licence}");
        }
        assert!(
            package["integrity"]
                .as_str()
                .is_some_and(|i| i.starts_with("sha512-")),
            "{path} has no integrity hash"
        );
    }
}

#[test]
fn agents_find_the_tools_and_local_bin_first_from_a_login_shell() {
    let profile = read("agents/profile.sh");
    let line = said(&profile)
        .into_iter()
        .find(|line| line.starts_with("PATH="))
        .expect("a PATH");
    assert!(line.starts_with("PATH=\"$HOME/.local/bin:"), "{line}");
    for dir in ["/usr/local/cargo/bin", "/usr/local/go/bin", "$PATH"] {
        assert!(line.contains(dir), "{line}");
    }
    assert!(read("Dockerfile.full").contains("/etc/profile.d/"));

    let compose = read("compose.yaml");
    assert!(
        compose.contains("image: ghcr.io/joaoh82/yardsort-host-full:"),
        "{compose}"
    );
    assert!(compose.contains("- home:/home/yardsort"), "{compose}");
}

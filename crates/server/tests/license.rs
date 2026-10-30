//! The server is AGPL and the rest of the workspace GPL, and the AGPL must not reach the app
//! (docs/design/30-open-source-and-managed.md §1).

use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_server_says_agpl_and_ships_its_text() {
    let manifest = std::fs::read_to_string(workspace().join("crates/server/Cargo.toml")).unwrap();
    assert!(manifest.contains("\nlicense = \"AGPL-3.0-only\"\n"));
    let text = std::fs::read_to_string(workspace().join("crates/server/LICENSE")).unwrap();
    assert!(text
        .trim_start()
        .starts_with("GNU AFFERO GENERAL PUBLIC LICENSE"));
    assert!(text.contains("13. Remote Network Interaction"));
}

/// Anything that depended on the server would carry the AGPL into the app or `ys`.
#[test]
fn nothing_else_in_the_workspace_depends_on_the_server() {
    let root = workspace();
    let mut manifests = vec![root.join("src-tauri/Cargo.toml")];
    for crate_dir in std::fs::read_dir(root.join("crates")).unwrap() {
        let manifest = crate_dir.unwrap().path().join("Cargo.toml");
        if manifest.exists() && !manifest.ends_with("server/Cargo.toml") {
            manifests.push(manifest);
        }
    }
    assert!(manifests.len() > 4, "found only {manifests:?}");
    for manifest in manifests {
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            !text.contains("yardsort-server") && !text.contains("crates/server"),
            "{} depends on the AGPL server",
            manifest.display()
        );
        assert!(
            text.contains("license.workspace = true"),
            "{} should stay under the workspace's GPL",
            manifest.display()
        );
    }
    let workspace_manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(workspace_manifest.contains("license = \"GPL-3.0-only\""));
}

//! `yardsort-host install-agent`, run as the operator would, with a stand-in for the vendor's
//! installer: the program lands in ~/.local/bin and is reported; nothing needs a profile.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

fn install(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_yardsort-host"))
        .arg("install-agent")
        .args(args)
        .env("HOME", home)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", home.join(".local/bin").display()),
        )
        // Nothing the profile would be found by: install-agent does not need one.
        .env_remove("YARDSORT_DATA_DIR")
        .output()
        .expect("yardsort-host runs")
}

#[test]
fn an_agent_from_its_installer_lands_in_local_bin() {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: refused as root, on purpose");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stand-in-installer.sh");
    let out = install(
        home.path(),
        &["claude", "--installer", installer.to_str().unwrap()],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.path().join(".local/bin/claude").exists());
    assert!(stdout.contains("claude 0.0.0 (stand-in)"), "{stdout}");
    // On PATH here, so no advice about it.
    assert!(!stdout.contains("is not on PATH"), "{stdout}");
}

#[test]
fn an_unknown_harness_is_refused_with_the_known_ones() {
    let home = tempfile::tempdir().unwrap();
    let out = install(home.path(), &["aider"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No harness called aider") && stderr.contains("claude"),
        "{stderr}"
    );
    assert!(
        !home.path().join(".local").exists(),
        "nothing made for a refusal"
    );
}

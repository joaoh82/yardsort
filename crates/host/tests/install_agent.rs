//! `yardsort-host install-agent`, run as the operator would, with a stand-in for the vendor's
//! installer: the program lands in ~/.local/bin and is reported; nothing needs a profile.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
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

/// `--installer` names a file the operator read. For an agent that comes from npm there is none to
/// run, and going to npm instead would be the network in its place: refused, before anything.
#[test]
fn an_installer_for_an_npm_agent_is_refused_before_npm_runs() {
    let home = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let ran = tools.path().join("npm-ran");
    let npm = tools.path().join("npm");
    std::fs::write(
        &npm,
        format!("#!/bin/sh\necho \"$@\" > {}\n", ran.display()),
    )
    .unwrap();
    std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let reviewed = home.path().join("reviewed.sh");
    std::fs::write(&reviewed, "#!/bin/sh\n").unwrap();

    for installer in [reviewed.to_str().unwrap(), "/nowhere/reviewed.sh"] {
        let out = Command::new(env!("CARGO_BIN_EXE_yardsort-host"))
            .args(["install-agent", "pi", "--installer", installer])
            .env("HOME", home.path())
            .env("PATH", format!("{}:/usr/bin:/bin", tools.path().display()))
            .output()
            .expect("yardsort-host runs");
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("comes from npm"), "{stderr}");
        assert!(
            !ran.exists(),
            "npm ran: {}",
            std::fs::read_to_string(&ran).unwrap()
        );
        assert!(!home.path().join(".local").exists());
    }
}

#[test]
fn a_missing_installer_is_refused_before_anything() {
    let home = tempfile::tempdir().unwrap();
    let out = install(
        home.path(),
        &["claude", "--installer", "/nowhere/install.sh"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No installer at /nowhere/install.sh"),
        "{stderr}"
    );
    assert!(!home.path().join(".local").exists());
}

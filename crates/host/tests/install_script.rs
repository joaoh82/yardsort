//! `packaging/install.sh`, the one line the guide gives for a new server: run for real, as an
//! ordinary user with a home of its own, against a release laid out in a folder. The tarball here
//! holds stand-in programs; CI's host-package job runs the script against the real one.
#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const UNIT: &str = include_str!("../packaging/yardsort-host.service");

fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => panic!("no host tarball for {other}"),
    }
}

fn executable(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(fixture.home()).unwrap();
        // The script asks systemd to keep the user's services up. Here that is written down, not
        // done to whoever runs the tests: every such call fails, as it does without the rights.
        for tool in ["loginctl", "systemctl", "sudo"] {
            executable(
                &fixture.dir.path().join("stubs").join(tool),
                &format!(
                    "#!/bin/sh\necho \"{tool} $*\" >> '{}'\nexit 1\n",
                    fixture.calls().display()
                ),
            );
        }
        fixture
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn calls(&self) -> PathBuf {
        self.dir.path().join("calls")
    }

    /// A release in the folder, as GitHub lays one out, whose programs say their version.
    fn release(&self, version: &str) {
        let name = format!("yardsort-host-{version}-linux-{}", arch());
        let stage = self.dir.path().join("stage");
        for program in ["yardsort-host", "ys"] {
            executable(
                &stage.join(&name).join("bin").join(program),
                &format!("#!/bin/sh\necho \"{program} {version}\"\n"),
            );
        }
        let unit = stage.join(&name).join("systemd/yardsort-host.service");
        fs::create_dir_all(unit.parent().unwrap()).unwrap();
        fs::write(unit, UNIT).unwrap();
        let to = self
            .dir
            .path()
            .join("releases/download")
            .join(format!("v{version}"));
        fs::create_dir_all(&to).unwrap();
        let made = Command::new("tar")
            .arg("-czf")
            .arg(to.join(format!("{name}.tar.gz")))
            .arg("-C")
            .arg(&stage)
            .arg(&name)
            .status()
            .unwrap();
        assert!(made.success());
    }

    fn install(&self, args: &[&str]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/install.sh");
        Command::new("sh")
            .arg(script)
            .args(args)
            .env_clear()
            .env("HOME", self.home())
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.dir.path().join("stubs").display()),
            )
            .env("SHELL", "/bin/sh")
            .env(
                "YARDSORT_HOST_RELEASES",
                format!("file://{}", self.dir.path().join("releases").display()),
            )
            .output()
            .unwrap()
    }

    fn version_of(&self, program: &str) -> String {
        let out = Command::new(self.home().join(".local/bin").join(program))
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn it_installs_the_programs_and_the_service_for_the_user_and_says_what_is_left() {
    let fixture = Fixture::new();
    fixture.release("9.9.9");
    let out = fixture.install(&["--version", "9.9.9"]);
    let said = text(&out);
    assert!(out.status.success(), "{said}");

    assert_eq!(fixture.version_of("yardsort-host"), "yardsort-host 9.9.9");
    assert_eq!(fixture.version_of("ys"), "ys 9.9.9");
    let unit = fixture
        .home()
        .join(".config/systemd/user/yardsort-host.service");
    assert_eq!(fs::read_to_string(unit).unwrap(), UNIT);

    // Each step that is a sign-in of the operator's is named, and none is run.
    for step in [
        "yardsort-host sign-in",
        "yardsort-host install-agent claude",
        "yardsort-host pair",
        "yardsort-host dashboard",
        "#git-and-github",
    ] {
        assert!(said.contains(step), "{step}: {said}");
    }
    if Path::new("/run/systemd/system").is_dir() {
        let calls = fs::read_to_string(fixture.calls()).unwrap();
        assert!(calls.contains("loginctl enable-linger"), "{calls}");
        // It could not turn linger on here, so it says how.
        assert!(said.contains("sudo loginctl enable-linger"), "{said}");
        // Signing in comes first, with the service stopped: it is never started here.
        assert!(!calls.contains("enable --now"), "{calls}");
        assert!(!calls.contains("systemctl --user start"), "{calls}");
    }
}

#[test]
fn run_again_it_upgrades_the_programs_and_keeps_a_unit_that_was_changed() {
    let fixture = Fixture::new();
    fixture.release("9.9.9");
    fixture.release("9.9.10");
    assert!(fixture.install(&["--version", "9.9.9"]).status.success());
    let unit = fixture
        .home()
        .join(".config/systemd/user/yardsort-host.service");
    let changed = format!("{UNIT}\n# mine\n");
    fs::write(&unit, &changed).unwrap();

    // A `v` before the version is taken too, as a tag is written.
    let out = fixture.install(&["--version", "v9.9.10"]);
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(fixture.version_of("yardsort-host"), "yardsort-host 9.9.10");
    assert_eq!(fixture.version_of("ys"), "ys 9.9.10");
    assert_eq!(fs::read_to_string(&unit).unwrap(), changed);
    assert!(said.contains("Kept "), "{said}");
    assert!(said.contains("The agents keep running."), "{said}");
    // Not the first-time steps again.
    assert!(!said.contains("yardsort-host sign-in"), "{said}");
}

#[test]
fn a_release_that_is_not_there_installs_nothing() {
    let fixture = Fixture::new();
    fixture.release("9.9.9");
    let out = fixture.install(&["--version", "1.2.3"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("Could not download"), "{}", text(&out));
    assert!(!fixture.home().join(".local").exists());
}

#[test]
fn naming_another_user_is_refused_unless_root() {
    let fixture = Fixture::new();
    fixture.release("9.9.9");
    let out = fixture.install(&["--version", "9.9.9", "--user", "somebody-else"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("--user is for root"), "{}", text(&out));
    assert!(!fixture.home().join(".local").exists());
}

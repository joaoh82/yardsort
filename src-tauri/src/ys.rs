//! The `ys` command on the user's `PATH`: is it there, is it this version, and putting it there.
//!
//! Every packaged copy of the app carries `ys` next to its own executable — a Tauri sidecar, see
//! `src-tauri/tauri.bundle.conf.json` — built from the same commit, so the two always agree about
//! the database. What differs between installs is how it reaches `PATH`:
//!
//! - **`.deb` / `.rpm`**, and the AUR package, which unpacks the `.deb`: the package puts it in
//!   `/usr/bin` beside the app. Nothing to do; the package manager updates both together.
//! - **macOS**: a symlink, `/usr/local/bin/ys`, into the app bundle. The updater replaces the
//!   bundle, so the link always reaches the current `ys`. `/usr/local/bin` usually needs an
//!   administrator, so the system asks for a password when it does.
//! - **A Linux AppImage**, or any other copy no package owns: a *copy* in `~/.local/bin`. An
//!   AppImage is mounted somewhere new every time it starts, so a link would dangle as soon as
//!   the app quit. Instead the copy is refreshed at startup whenever the app is newer.
//! - **Windows**: a copy in `%LOCALAPPDATA%\dev.yardsort.app\bin`, which is added to the user's
//!   `PATH`, refreshed the same way. Not the install folder itself on `PATH`: that would put the
//!   uninstaller there too, and a `ys.exe` left running there would stop the updater replacing it.
//!
//! Nothing is installed until the user asks. A file already at the target that is not a `ys` is
//! replaced only after a second, explicit yes.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::program::Program;
use crate::state::blocking;

/// The app's version, which is also the version of the `ys` it carries.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const EXE: &str = if cfg!(windows) { "ys.exe" } else { "ys" };

/// How this copy of the app puts `ys` on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum YsMethod {
    /// The package installed it along with the app. Nothing to do.
    Packaged,
    /// A symlink into the app bundle (macOS).
    Link,
    /// A copy, refreshed when the app is updated (AppImage, Windows).
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct YsStatus {
    /// The app's version; the `ys` that came with it is the same one.
    pub version: String,
    pub method: YsMethod,
    /// The `ys` that came with this copy of the app. `None` for a build without one: a
    /// development build before `cargo build -p yardsort-cli`, or one bundled by hand.
    pub bundled: Option<String>,
    /// Where Install puts it. `None` when the package already did.
    pub target: Option<String>,
    /// Something is at `target` already.
    pub installed: bool,
    /// The folder `target` is in is on the login shell's `PATH`, so a terminal will find it.
    pub target_on_path: bool,
    /// The `ys` a terminal finds, if any.
    pub found: Option<String>,
    /// What `found --version` says. `None` if it is not a `ys` at all.
    pub found_version: Option<String>,
    /// `found` is the one this app ships or installed.
    pub found_is_ours: bool,
}

/// Where everything is, worked out once, so tests can point it at a temporary directory.
#[derive(Debug, Clone)]
pub struct Layout {
    pub method: YsMethod,
    pub bundled: Option<PathBuf>,
    pub target: Option<PathBuf>,
}

impl Layout {
    pub fn detect(env: &ShellEnv) -> Self {
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join(EXE)))
            .filter(|path| path.is_file());
        // `YARDSORT_YS_DIR` sends Install somewhere harmless, for trying it out.
        let dir = crate::legacy::env_var_os("YS_DIR").map(PathBuf::from);
        let method = if dir.is_none() && packaged() {
            YsMethod::Packaged
        } else if cfg!(target_os = "macos") {
            YsMethod::Link
        } else {
            YsMethod::Copy
        };
        let target = match method {
            YsMethod::Packaged => None,
            _ => dir.or_else(|| default_dir(env)).map(|dir| dir.join(EXE)),
        };
        Self {
            method,
            bundled,
            target,
        }
    }
}

/// dpkg and rpm own the files, `ys` included.
fn packaged() -> bool {
    use tauri::utils::config::BundleType;
    cfg!(target_os = "linux")
        && matches!(
            tauri::utils::platform::bundle_type(),
            Some(BundleType::Deb | BundleType::Rpm)
        )
}

fn default_dir(env: &ShellEnv) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from("/usr/local/bin"))
    } else if cfg!(windows) {
        env.get("LOCALAPPDATA")
            .map(|dir| Path::new(dir).join("dev.yardsort.app").join("bin"))
    } else {
        env.home_dir().map(|home| home.join(".local").join("bin"))
    }
}

pub fn status(layout: &Layout, env: &ShellEnv) -> YsStatus {
    let cwd = std::env::current_dir().unwrap_or_default();
    let found = env.find_program("ys", &cwd);
    let is = |path: &Path, other: &Option<PathBuf>| {
        other.as_deref().is_some_and(|other| same_file(path, other))
    };
    let target_on_path = match &layout.target {
        None => true,
        Some(target) => target.parent().is_some_and(|dir| on_path(env, dir)),
    };
    let text = |path: &Option<PathBuf>| {
        path.as_deref()
            .map(|path| path.to_string_lossy().into_owned())
    };
    YsStatus {
        version: VERSION.to_owned(),
        method: layout.method,
        bundled: text(&layout.bundled),
        target: text(&layout.target),
        installed: layout
            .target
            .as_deref()
            .is_some_and(|target| std::fs::symlink_metadata(target).is_ok()),
        target_on_path,
        found_version: found.as_deref().and_then(|path| version_of(path, env)),
        found_is_ours: found
            .as_deref()
            .is_some_and(|path| is(path, &layout.bundled) || is(path, &layout.target)),
        found: text(&found),
    }
}

/// Put `ys` at the layout's target. With `replace`, whatever is there goes even if it is not a
/// `ys` — the user has been asked.
pub fn install(layout: &Layout, env: &ShellEnv, replace: bool) -> IpcResult<()> {
    let bundled = layout.bundled.as_deref().ok_or_else(|| {
        IpcError::new(
            "ys_not_bundled",
            "This copy of Yardsort does not include ys. Download it from the release page instead.",
        )
    })?;
    let Some(target) = layout.target.as_deref() else {
        return Err(IpcError::new(
            "ys_packaged",
            format!(
                "ys came with this package, at {}. Your package manager keeps it up to date.",
                bundled.display()
            ),
        ));
    };
    if !replace && occupied_by_something_else(target, bundled, env) {
        return Err(IpcError::new(
            "ys_exists",
            format!(
                "There is already a file at {} and it is not Yardsort's ys.",
                target.display()
            ),
        ));
    }
    match layout.method {
        YsMethod::Link => {
            // A link into a bundle the system moved somewhere temporary, or into a disk image,
            // would stop working as soon as the app quit.
            let from = bundled.to_string_lossy();
            if from.contains("/AppTranslocation/") || from.starts_with("/Volumes/") {
                return Err(IpcError::new(
                    "ys_translocated",
                    "Move Yardsort to your Applications folder and open it from there first.",
                ));
            }
            link(bundled, target)
        }
        YsMethod::Copy => {
            copy(bundled, target).map_err(|e| failed(target, e))?;
            #[cfg(windows)]
            if let Some(dir) = target.parent() {
                user_path::add(dir).map_err(|e| {
                    IpcError::new(
                        "ys_install_failed",
                        format!("Installed ys, but could not add its folder to your PATH: {e}"),
                    )
                })?;
            }
            Ok(())
        }
        YsMethod::Packaged => unreachable!("a packaged install has no target"),
    }
}

/// Bring a copy installed by an older version up to this one. Returns what it did, for the log.
///
/// Only a copy — a link follows the bundle by itself — and only a `ys` that is older: a copy put
/// there by a newer Yardsort, or something that is not a `ys`, is left alone.
pub fn refresh(layout: &Layout, env: &ShellEnv) -> Option<String> {
    if layout.method != YsMethod::Copy {
        return None;
    }
    let (bundled, target) = (layout.bundled.as_deref()?, layout.target.as_deref()?);
    #[cfg(windows)]
    let _ = std::fs::remove_file(target.with_file_name(format!("{EXE}.old")));
    if !target.is_file() {
        return None;
    }
    let installed = version_of(target, env)?;
    if !is_newer(VERSION, &installed) {
        return None;
    }
    Some(match copy(bundled, target) {
        Ok(()) => format!("updated {} from {installed} to {VERSION}", target.display()),
        Err(e) => format!("could not update {}: {e}", target.display()),
    })
}

fn failed(target: &Path, error: impl std::fmt::Display) -> IpcError {
    IpcError::new(
        "ys_install_failed",
        format!("Could not install ys at {}: {error}", target.display()),
    )
}

/// Is there something at `target` we should not replace without asking?
fn occupied_by_something_else(target: &Path, bundled: &Path, env: &ShellEnv) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(target) else {
        return false;
    };
    if metadata.file_type().is_symlink() && !target.exists() {
        // A link to nowhere cannot be asked its version. It is ours only if it points where we
        // put links: this app's `ys`, or the `ys` of a Yardsort.app that has since been moved or
        // deleted. A dangling link to anything else may be a tool on a disk that is not mounted.
        return !std::fs::read_link(target).is_ok_and(|to| {
            let to = target.parent().map_or(to.clone(), |dir| dir.join(&to));
            to == bundled || to.ends_with(Path::new("Yardsort.app/Contents/MacOS").join(EXE))
        });
    }
    version_of(target, env).is_none()
}

/// Write a new file beside the old one and rename it into place, so that a terminal never finds
/// half a `ys`.
fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    let dir = to.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let fresh = dir.join(format!(".{EXE}.new"));
    // `fs::copy` carries the permission bits over, the executable bit included.
    std::fs::copy(from, &fresh)?;
    // A running `ys.exe` cannot be replaced, but it can be renamed out of the way. The old one
    // goes at the next refresh.
    #[cfg(windows)]
    if to.exists() {
        let old = dir.join(format!("{EXE}.old"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(to, &old)?;
    }
    std::fs::rename(&fresh, to)
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) -> IpcResult<()> {
    let attempt = || -> std::io::Result<()> {
        let dir = to.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(dir)?;
        let fresh = dir.join(format!(".{EXE}.new"));
        let _ = std::fs::remove_file(&fresh);
        std::os::unix::fs::symlink(from, &fresh)?;
        std::fs::rename(&fresh, to)
    };
    match attempt() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(target_os = "macos") => {
            link_as_administrator(from, to)
        }
        Err(e) => Err(failed(to, e)),
    }
}

#[cfg(not(unix))]
fn link(_: &Path, to: &Path) -> IpcResult<()> {
    Err(failed(to, "links are only made on macOS"))
}

/// macOS's own password prompt, then `ln`. The paths travel as arguments and are quoted by
/// AppleScript's `quoted form of`, never spliced into the script's text.
#[cfg(unix)]
fn link_as_administrator(from: &Path, to: &Path) -> IpcResult<()> {
    let dir = to.parent().unwrap_or(Path::new("/"));
    let output = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "do shell script \"mkdir -p \" & quoted form of item 1 of argv & \
             \" && ln -sfn \" & quoted form of item 2 of argv & \" \" & quoted form of item 3 \
             of argv with prompt \"Yardsort wants to install the ys command.\" with \
             administrator privileges",
            "-e",
            "end run",
        ])
        .arg(dir)
        .arg(from)
        .arg(to)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| failed(to, e))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // -128 is "User canceled".
    if stderr.contains("-128") {
        return Err(IpcError::new(
            "ys_cancelled",
            "Installing ys was cancelled.",
        ));
    }
    Err(failed(to, stderr.trim()))
}

/// What `<path> --version` says, if it is a `ys`. Anything else — another program called `ys`,
/// one that fails or hangs — is `None`.
fn version_of(path: &Path, env: &ShellEnv) -> Option<String> {
    version_within(path, env, Duration::from_secs(5))
}

/// The same, giving up after `timeout` — on the whole probe, output included. A program can exit
/// and leave a descendant holding its stdout, so the end of the output may never come: only the
/// first line is read, and that too against the deadline.
fn version_within(path: &Path, env: &ShellEnv, timeout: Duration) -> Option<String> {
    let mut child = Program::at(path.to_owned(), env)
        .command(&std::env::temp_dir())
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, first_line) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let line = first_line
        .recv_timeout(timeout.saturating_sub(started.elapsed()))
        .ok()?;
    parse_version(&line)
}

/// `ys 0.10.0` → `0.10.0`, as clap prints it.
fn parse_version(output: &str) -> Option<String> {
    let version = output.trim().strip_prefix("ys ")?.trim();
    (!version.is_empty() && !version.contains(char::is_whitespace)).then(|| version.to_owned())
}

/// `a` is a later version than `b`. `1.2.3` beats `1.2.3-beta.1`, which beats `1.2.2`.
fn is_newer(a: &str, b: &str) -> bool {
    fn key(version: &str) -> (Vec<u64>, bool, &str) {
        let (release, pre) = version.split_once('-').unwrap_or((version, ""));
        let numbers = release
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        (numbers, pre.is_empty(), pre)
    }
    key(a) > key(b)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn on_path(env: &ShellEnv, dir: &Path) -> bool {
    env.get("PATH").is_some_and(|path| {
        std::env::split_paths(path).any(|entry| {
            same_file(&entry, dir)
                || entry
                    .as_os_str()
                    .to_string_lossy()
                    .trim_end_matches(['/', '\\'])
                    == dir
                        .as_os_str()
                        .to_string_lossy()
                        .trim_end_matches(['/', '\\'])
        })
    })
}

/// The user's own `PATH`, as Windows keeps it in the registry.
#[cfg(windows)]
mod user_path {
    use std::path::Path;

    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::{RegKey, RegValue};

    /// Append `dir` to the user's `PATH` unless it is there, keeping the value's type:
    /// `REG_EXPAND_SZ` entries such as `%USERPROFILE%\…` must stay unexpanded.
    pub fn add(dir: &Path) -> std::io::Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
        let (current, vtype) = match key.get_raw_value("Path") {
            Ok(value) => (decode(&value.bytes), value.vtype),
            Err(_) => (String::new(), RegType::REG_EXPAND_SZ),
        };
        let Some(updated) = super::with_entry(&current, &dir.to_string_lossy()) else {
            return Ok(());
        };
        let vtype = match vtype {
            RegType::REG_SZ => RegType::REG_SZ,
            _ => RegType::REG_EXPAND_SZ,
        };
        key.set_raw_value(
            "Path",
            &RegValue {
                bytes: encode(&updated),
                vtype,
            },
        )?;
        broadcast();
        Ok(())
    }

    fn decode(bytes: &[u8]) -> String {
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&wide)
    }

    fn encode(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    /// Tell Explorer, so terminals started from now on get the new `PATH` without signing out.
    fn broadcast() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        let name: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result = 0usize;
        // SAFETY: `name` outlives the call, which copies nothing it keeps.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                name.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
    }
}

/// `path` with `dir` appended, or `None` if it has it already. Windows compares paths without
/// regard to case or a trailing backslash.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_entry(path: &str, dir: &str) -> Option<String> {
    let normal = |entry: &str| entry.trim().trim_end_matches('\\').to_lowercase();
    if path.split(';').any(|entry| normal(entry) == normal(dir)) {
        return None;
    }
    let path = path.trim_end_matches(';');
    Some(if path.is_empty() {
        dir.to_owned()
    } else {
        format!("{path};{dir}")
    })
}

/// Where `ys` is, whether it is this version, and where Install would put it.
#[tauri::command]
#[specta::specta]
pub async fn ys_status(app: AppHandle) -> IpcResult<YsStatus> {
    blocking(app, |state| {
        let env = state.env();
        Ok(status(&Layout::detect(&env), &env))
    })
    .await
}

/// Put `ys` on `PATH`. With `replace`, over a file that is not a `ys` — ask first.
#[tauri::command]
#[specta::specta]
pub async fn ys_install(app: AppHandle, replace: bool) -> IpcResult<YsStatus> {
    blocking(app, move |state| {
        let layout = Layout::detect(&state.env());
        install(&layout, &state.env(), replace)?;
        // The login shell again: `~/.local/bin` may only be on `PATH` once it exists.
        let env = state.reload_env();
        Ok(status(&layout, &env))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvSource;

    fn env_with_path(path: &Path) -> ShellEnv {
        let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        vars.retain(|name, _| !name.eq_ignore_ascii_case("PATH"));
        vars.insert("PATH".into(), path.to_string_lossy().into_owned());
        ShellEnv {
            vars,
            source: EnvSource::Process,
            warning: None,
        }
    }

    /// A stand-in for the bundled `ys`: a script that answers `--version` the way clap does.
    #[cfg(unix)]
    fn fake_ys(dir: &Path, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(EXE);
        std::fs::write(&path, format!("#!/bin/sh\necho 'ys {version}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn stranger(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "#!/bin/sh\necho 'something else 1.0'\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A wrapper that exits but leaves a descendant holding its stdout: the end of the output
    /// never comes while the descendant lives.
    #[cfg(unix)]
    #[test]
    fn a_descendant_holding_stdout_does_not_outlast_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_with_path(dir.path());
        let answers = dir.path().join("answers");
        script(&answers, "sleep 30 &\necho 'ys 0.10.0'");
        let silent = dir.path().join("silent");
        script(&silent, "sleep 30 &\nexit 0");

        let started = Instant::now();
        assert_eq!(
            version_within(&answers, &env, Duration::from_secs(3)).as_deref(),
            Some("0.10.0"),
            "the first line is the answer; the rest need not arrive"
        );
        assert_eq!(version_within(&silent, &env, Duration::from_secs(1)), None);
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "took {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_is_replaced_without_asking_only_when_it_was_ours() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let env = env_with_path(bin.path());
        let target = bin.path().join(EXE);
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: Some(fake_ys(app.path(), VERSION)),
            target: Some(target.clone()),
        };

        // A tool on a disk that is not mounted.
        let elsewhere = Path::new("/nonexistent-volume/tools/ys");
        std::os::unix::fs::symlink(elsewhere, &target).unwrap();
        assert_eq!(install(&layout, &env, false).unwrap_err().code, "ys_exists");
        assert_eq!(
            std::fs::read_link(&target).unwrap(),
            elsewhere,
            "a no leaves the link alone"
        );

        // The link a Yardsort.app that has since been deleted left behind.
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink("/Applications/Old/Yardsort.app/Contents/MacOS/ys", &target)
            .unwrap();
        install(&layout, &env, false).unwrap();
        assert_eq!(version_of(&target, &env).as_deref(), Some(VERSION));
    }

    #[test]
    fn reads_the_version_clap_prints() {
        assert_eq!(parse_version("ys 0.10.0\n").as_deref(), Some("0.10.0"));
        assert_eq!(
            parse_version("ys 1.0.0-beta.2").as_deref(),
            Some("1.0.0-beta.2")
        );
        assert_eq!(parse_version("yardsort 0.10.0"), None);
        assert_eq!(parse_version("ys"), None);
        assert_eq!(parse_version("ys: command not found"), None);
    }

    #[test]
    fn orders_versions_the_way_releases_are_numbered() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(is_newer("1.0.0-beta.2", "1.0.0-beta.1"));
        assert!(!is_newer("0.10.0", "0.10.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
    }

    #[test]
    fn a_path_entry_is_added_once_whatever_its_spelling() {
        assert_eq!(
            with_entry(
                r"C:\Tools;%USERPROFILE%\bin",
                r"C:\Users\a\AppData\Local\y\bin"
            ),
            Some(r"C:\Tools;%USERPROFILE%\bin;C:\Users\a\AppData\Local\y\bin".into())
        );
        assert_eq!(with_entry("", r"C:\y\bin"), Some(r"C:\y\bin".into()));
        assert_eq!(
            with_entry(r"C:\Tools;", r"C:\y"),
            Some(r"C:\Tools;C:\y".into())
        );
        assert_eq!(with_entry(r"C:\Tools;c:\Y\BIN\", r"C:\y\bin"), None);
    }

    #[cfg(unix)]
    #[test]
    fn install_copies_ys_where_the_shell_will_find_it() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: Some(fake_ys(app.path(), VERSION)),
            target: Some(bin.path().join("nested").join(EXE)),
        };
        let env = env_with_path(&bin.path().join("nested"));

        let before = status(&layout, &env);
        assert!(!before.installed);
        assert_eq!(before.found, None);
        assert!(before.target_on_path);

        install(&layout, &env, false).unwrap();
        let after = status(&layout, &env);
        assert!(after.installed);
        assert!(after.found_is_ours);
        assert_eq!(after.found_version.as_deref(), Some(VERSION));
        assert!(
            !std::fs::symlink_metadata(layout.target.as_ref().unwrap())
                .unwrap()
                .file_type()
                .is_symlink(),
            "a copy, not a link: an AppImage's mount goes away"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_follows_the_app_when_it_is_updated() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let layout = Layout {
            method: YsMethod::Link,
            bundled: Some(fake_ys(app.path(), "0.1.0")),
            target: Some(bin.path().join(EXE)),
        };
        let env = env_with_path(bin.path());
        install(&layout, &env, false).unwrap();
        assert_eq!(
            status(&layout, &env).found_version.as_deref(),
            Some("0.1.0")
        );

        fake_ys(app.path(), "0.2.0");
        let status = status(&layout, &env);
        assert_eq!(status.found_version.as_deref(), Some("0.2.0"));
        assert!(status.found_is_ours);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_not_ys_is_replaced_only_when_the_user_says_so() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let target = bin.path().join(EXE);
        stranger(&target);
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: Some(fake_ys(app.path(), VERSION)),
            target: Some(target.clone()),
        };
        let env = env_with_path(bin.path());

        let refused = install(&layout, &env, false).unwrap_err();
        assert_eq!(refused.code, "ys_exists");
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("something else"),
            "a no leaves it alone"
        );
        assert_eq!(status(&layout, &env).found_version, None);

        install(&layout, &env, true).unwrap();
        assert_eq!(
            status(&layout, &env).found_version.as_deref(),
            Some(VERSION)
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_older_ys_is_replaced_without_asking() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: Some(fake_ys(app.path(), VERSION)),
            target: Some(fake_ys(bin.path(), "0.0.1")),
        };
        install(&layout, &env_with_path(bin.path()), false).unwrap();
        assert_eq!(
            version_of(layout.target.as_ref().unwrap(), &env_with_path(bin.path())).as_deref(),
            Some(VERSION)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_is_refreshed_only_when_the_app_is_newer() {
        let app = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let env = env_with_path(bin.path());
        let target = bin.path().join(EXE);
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: Some(fake_ys(app.path(), VERSION)),
            target: Some(target.clone()),
        };

        assert_eq!(
            refresh(&layout, &env),
            None,
            "not installed: not installed now either"
        );
        assert!(!target.exists());

        fake_ys(bin.path(), "0.0.1");
        assert!(refresh(&layout, &env).unwrap().contains("updated"));
        assert_eq!(version_of(&target, &env).as_deref(), Some(VERSION));

        fake_ys(bin.path(), "999.0.0");
        assert_eq!(refresh(&layout, &env), None, "a newer one is left alone");
        assert_eq!(version_of(&target, &env).as_deref(), Some("999.0.0"));

        stranger(&target);
        assert_eq!(refresh(&layout, &env), None, "something else is left alone");
        assert_eq!(version_of(&target, &env), None);

        let link = Layout {
            method: YsMethod::Link,
            ..layout
        };
        fake_ys(bin.path(), "0.0.1");
        assert_eq!(refresh(&link, &env), None, "a link needs no refreshing");
    }

    #[test]
    fn without_a_bundled_ys_or_a_target_there_is_nothing_to_install() {
        let bin = tempfile::tempdir().unwrap();
        let env = env_with_path(bin.path());
        let unbundled = Layout {
            method: YsMethod::Copy,
            bundled: None,
            target: Some(bin.path().join(EXE)),
        };
        assert_eq!(
            install(&unbundled, &env, false).unwrap_err().code,
            "ys_not_bundled"
        );
        let packaged = Layout {
            method: YsMethod::Packaged,
            bundled: Some(bin.path().join(EXE)),
            target: None,
        };
        assert_eq!(
            install(&packaged, &env, false).unwrap_err().code,
            "ys_packaged"
        );
        let status = status(&packaged, &env);
        assert!(status.target_on_path);
        assert!(!status.installed);
    }

    #[test]
    fn a_folder_off_the_path_is_reported() {
        let bin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let layout = Layout {
            method: YsMethod::Copy,
            bundled: None,
            target: Some(elsewhere.path().join(EXE)),
        };
        assert!(!status(&layout, &env_with_path(bin.path())).target_on_path);
    }
}

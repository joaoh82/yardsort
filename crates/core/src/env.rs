//! The environment harnesses run in.
//!
//! A GUI app does not inherit the user's shell environment: launched from a dock or desktop
//! launcher, `PATH` lacks everything that `.zshrc`, mise, nvm, cargo or Homebrew add — so
//! `claude` "does not exist" even though it works in every terminal. We fix that the way editors
//! do: run the user's login shell once, ask it for its environment, and use that for every spawn.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use specta::Type;

/// Makes this executable print its environment and exit. See [`print_env_and_exit_if_asked`].
const PRINT_ENV_FLAG: &str = "--yardsort-print-env";
const BEGIN: &[u8] = b"\0YARDSORT-ENV-BEGIN\0";
const END: &[u8] = b"\0YARDSORT-ENV-END\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EnvSource {
    /// Captured from the user's login shell.
    LoginShell,
    /// This process's own environment: always on Windows, and the fallback elsewhere.
    Process,
}

#[derive(Debug, Clone)]
pub struct ShellEnv {
    pub vars: BTreeMap<String, String>,
    pub source: EnvSource,
    /// Why the login shell could not be used, when it could not.
    pub warning: Option<String>,
}

/// Variables by which a coding agent tells its children "you are running inside my session".
/// When Yardsort itself was started from a terminal inside an agent (say, `just dev` run by
/// Claude Code), they would leak into every harness we launch, which then behaves as a nested
/// child — Claude Code, for one, stops saving its transcript, so the session can never be
/// resumed. They describe a process that is not ours; user configuration is left alone.
const FOREIGN_SESSION_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_PID",
];

fn without_foreign_sessions(mut vars: BTreeMap<String, String>) -> BTreeMap<String, String> {
    vars.retain(|name, _| !FOREIGN_SESSION_MARKERS.contains(&name.as_str()));
    vars
}

/// Variables the AppImage runtime sets to describe itself.
const APPIMAGE_RUNTIME_VARS: &[&str] = &["APPDIR", "APPIMAGE", "ARGV0", "OWD"];

/// Variables the AppImage's GTK launch hook (`linuxdeploy-plugin-gtk`) sets outright, overwriting
/// whatever the user had. Some carry no bundle path, so the path filter alone would miss them:
/// `GDK_BACKEND=x11` in particular would push every GTK program started from a terminal onto
/// XWayland.
const APPIMAGE_GTK_HOOK_VARS: &[&str] = &[
    "GDK_BACKEND",
    "GDK_PIXBUF_MODULE_FILE",
    "GIO_EXTRA_MODULES",
    "GSETTINGS_SCHEMA_DIR",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GTK_PATH",
    "GTK_THEME",
];

/// Whether a path leads into *some* AppImage's mount. An AppImage mounts itself at
/// `<temp>/.mount_<name><random>` for as long as it runs, so nothing under there is worth passing
/// to a child: at best it is our own bundle, at worst a dead mount from the version we replaced.
fn in_an_appimage_mount(entry: &str) -> bool {
    let tmpdir = std::env::var("TMPDIR").unwrap_or_default();
    [tmpdir.trim_end_matches('/'), "/tmp"]
        .iter()
        .any(|dir| !dir.is_empty() && entry.starts_with(&format!("{dir}/.mount_")))
}

/// The environment the user launched us with, without what the AppImage added on the way in.
///
/// An AppImage starts us with its bundled libraries, Python, Perl, Qt and GTK modules on
/// `LD_LIBRARY_PATH`, `PYTHONHOME`, `PATH` and a dozen more, all pointing into its mount. Our
/// sessions must not inherit them: the user's own `python3` dies looking for its standard library
/// in the bundle, and every program linked against a library we ship picks up our copy.
///
/// The mount we are running from is not the only one to watch for. An in-app update relaunches
/// the new AppImage from the old process, so we inherit the *previous* version's variables and
/// the new launcher prepends its own; the old mount is still there, and a `git` started with that
/// `LD_LIBRARY_PATH` loads its libraries out of the version we just replaced. Outside an AppImage,
/// and for paths that lead anywhere else, this changes nothing.
fn without_appimage(mut vars: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let appdir = vars
        .get("APPDIR")
        .filter(|_| vars.contains_key("APPIMAGE"))
        .map(|dir| dir.trim_end_matches('/').to_owned())
        .filter(|dir| !dir.is_empty());
    let in_a_bundle = |entry: &str| {
        in_an_appimage_mount(entry)
            || appdir
                .as_deref()
                .is_some_and(|dir| entry == dir || entry.starts_with(&format!("{dir}/")))
    };

    vars.retain(|name, value| {
        if appdir.is_some()
            && (APPIMAGE_RUNTIME_VARS.contains(&name.as_str())
                || APPIMAGE_GTK_HOOK_VARS.contains(&name.as_str()))
        {
            return false;
        }
        // Values that lead nowhere near a bundle are left exactly as they are — including ones
        // that merely contain a colon, which is not always a path separator.
        if !value.split(':').any(in_a_bundle) {
            return true;
        }
        // A search path the bundle prepended to: keep the user's part. Empty entries are what the
        // launcher's `"$APPDIR/…:$VAR"` leaves behind when `VAR` was unset.
        let rest: Vec<&str> = value
            .split(':')
            .filter(|entry| !entry.is_empty() && !in_a_bundle(entry))
            .collect();
        *value = rest.join(":");
        !value.is_empty()
    });
    vars
}

/// What every environment we hand out goes through.
fn cleaned(vars: BTreeMap<String, String>) -> BTreeMap<String, String> {
    without_foreign_sessions(without_appimage(vars))
}

impl ShellEnv {
    pub fn resolve() -> Self {
        match platform::from_login_shell() {
            Ok(Some(vars)) => Self {
                vars: cleaned(vars),
                source: EnvSource::LoginShell,
                warning: None,
            },
            Ok(None) => Self::from_process(None),
            Err(reason) => Self::from_process(Some(reason)),
        }
    }

    fn from_process(warning: Option<String>) -> Self {
        let mut vars = cleaned(std::env::vars().collect());
        // A process's environment on Windows is a snapshot taken when it started. An installer
        // writes the new `PATH` into the registry and broadcasts a change that only new processes
        // see, so "Check again" would re-read the same stale copy and never find an agent
        // installed a minute ago. Read `PATH` back from where the installer put it.
        if let Some(fresh) = platform::path_from_registry() {
            let key = name_of(&vars, "PATH").unwrap_or_else(|| "PATH".to_owned());
            let merged = merge_path(vars.get(&key).map(String::as_str).unwrap_or(""), &fresh);
            vars.insert(key, merged);
        }
        Self {
            vars,
            source: EnvSource::Process,
            warning,
        }
    }

    /// Whether a child should get exactly [`Self::vars`] instead of inheriting our environment
    /// with them layered on top. On Unix `vars` is always complete, and inheriting would bring
    /// back what [`cleaned`] took out (an AppImage's `LD_LIBRARY_PATH`, say). Windows keeps
    /// layering its process environment, as it always has.
    pub fn replaces_inherited(&self) -> bool {
        cfg!(unix) || self.source == EnvSource::LoginShell
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        lookup(&self.vars, key, cfg!(windows))
    }

    /// Locate `program` the way a shell would, using *this* environment's `PATH`.
    pub fn find_program(&self, program: &str, cwd: &std::path::Path) -> Option<PathBuf> {
        which::which_in(program, self.get("PATH"), cwd).ok()
    }

    /// The user's interactive shell and the arguments to start it with.
    pub fn default_shell(&self) -> (String, Vec<String>) {
        platform::default_shell(self)
    }

    pub fn home_dir(&self) -> Option<PathBuf> {
        self.get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
    }
}

/// Windows variable names are case-insensitive, and the ones that matter are not spelled the way
/// everyone writes them: it is `Path` and `ComSpec` there, not `PATH` and `COMSPEC`.
/// The key `vars` actually spells `key` with — Windows says `Path` about as often as `PATH`.
fn name_of(vars: &BTreeMap<String, String>, key: &str) -> Option<String> {
    vars.keys()
        .find(|name| name.eq_ignore_ascii_case(key))
        .cloned()
}

/// The separator, always Windows's: this is only reached where the registry holds `PATH`, and
/// pinning it keeps the function testable on the machines it does not run on.
const SEP: char = ';';
const SEP_STR: &str = ";";

/// `PATH` as it was when we started, plus anything that has been added since.
///
/// A union rather than a replacement: starting Yardsort from a terminal that had added something
/// to `PATH` is a real thing people do, and that entry is not in the registry. Order is kept —
/// ours first, then whatever is new — so nothing that used to resolve one way now resolves
/// another. Comparison ignores case and a trailing separator, as Windows does.
fn merge_path(current: &str, fresh: &str) -> String {
    let tidy = |entry: &str| entry.trim_end_matches(['\\', '/']).to_ascii_lowercase();

    let mut out: Vec<&str> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for entry in current.split(SEP).chain(fresh.split(SEP)) {
        if entry.is_empty() {
            continue;
        }
        let key = tidy(entry);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(entry);
    }
    out.join(SEP_STR)
}

fn lookup<'a>(vars: &'a BTreeMap<String, String>, key: &str, ignore_case: bool) -> Option<&'a str> {
    vars.get(key)
        .or_else(|| {
            ignore_case
                .then(|| vars.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)))
                .flatten()
                .map(|(_, value)| value)
        })
        .map(String::as_str)
}

/// Call first thing in `main`. When the login shell runs us with [`PRINT_ENV_FLAG`], dump the
/// environment it gave us and exit. Using our own binary avoids depending on `env -0` (absent on
/// some systems) or on any particular shell's syntax.
pub fn print_env_and_exit_if_asked() {
    if std::env::args().nth(1).as_deref() != Some(PRINT_ENV_FLAG) {
        return;
    }
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(BEGIN);
    for (key, value) in std::env::vars_os() {
        let _ = out.write_all(key.as_encoded_bytes());
        let _ = out.write_all(b"=");
        let _ = out.write_all(value.as_encoded_bytes());
        let _ = out.write_all(b"\0");
    }
    let _ = out.write_all(END);
    let _ = out.flush();
    std::process::exit(0);
}

/// Extract the variables printed between the markers, ignoring whatever the shell's startup
/// files printed around them.
#[cfg(any(unix, test))]
fn parse_dump(output: &[u8]) -> Option<BTreeMap<String, String>> {
    let start = find(output, BEGIN)? + BEGIN.len();
    let end = start + find(&output[start..], END)?;
    let vars: BTreeMap<_, _> = output[start..end]
        .split(|&b| b == 0)
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (key, value) = entry.split_once('=')?;
            // Facts about the throwaway shell, not about the user's environment.
            let transient = matches!(key, "_" | "SHLVL" | "PWD" | "OLDPWD");
            (!key.is_empty() && !transient).then(|| (key.to_owned(), value.to_owned()))
        })
        .collect();
    (!vars.is_empty()).then_some(vars)
}

#[cfg(any(unix, test))]
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(unix)]
mod platform {
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::process::{Command, Stdio};

    /// Only Windows keeps `PATH` somewhere a running process cannot see change.
    pub(super) fn path_from_registry() -> Option<String> {
        None
    }
    use std::time::{Duration, Instant};

    use super::{cleaned, parse_dump, ShellEnv, PRINT_ENV_FLAG};

    /// Slow shell startup files are common; a hung one must not hang the app.
    const TIMEOUT: Duration = Duration::from_secs(8);

    pub(super) fn from_login_shell() -> Result<Option<BTreeMap<String, String>>, String> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let exe =
            std::env::current_exe().map_err(|e| format!("cannot locate own executable: {e}"))?;
        let exe = exe.to_str().ok_or("own executable path is not UTF-8")?;
        // Single-quote the path; this quoting is understood by sh, bash, zsh and fish alike.
        let script = format!("'{}' {PRINT_ENV_FLAG}", exe.replace('\'', r"'\''"));

        let mut command = Command::new(&shell);
        // The shell's startup files run in what we hand it, and version managers record what
        // they found there, so it starts from the cleaned environment too — not just its output.
        let original: BTreeMap<String, String> = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        let kept = cleaned(original.clone());
        for (key, value) in &original {
            match kept.get(key) {
                None => {
                    command.env_remove(key);
                }
                Some(new) if new != value => {
                    command.env(key, new);
                }
                Some(_) => {}
            }
        }

        let mut child = command
            // Interactive + login, so both profile and rc files run — version managers hook
            // into either.
            .args(["-i", "-l", "-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot run {shell}: {e}"))?;

        let mut stdout = child.stdout.take().expect("stdout was piped");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        });

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() > TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{shell} did not finish within {TIMEOUT:?}"));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                Err(e) => return Err(format!("waiting for {shell} failed: {e}")),
            }
        }

        let output = reader.join().map_err(|_| "reading shell output failed")?;
        parse_dump(&output)
            .map(Some)
            .ok_or_else(|| format!("{shell} produced no environment"))
    }

    pub(super) fn default_shell(env: &ShellEnv) -> (String, Vec<String>) {
        let shell = env.get("SHELL").unwrap_or("/bin/sh").to_owned();
        // macOS terminals start login shells by convention; Linux ones do not.
        let args = if cfg!(target_os = "macos") {
            vec!["-l".to_owned()]
        } else {
            vec![]
        };
        (shell, args)
    }
}

#[cfg(windows)]
mod platform {
    use std::collections::BTreeMap;

    use super::ShellEnv;

    /// Windows GUI apps get the full user environment at launch — and only at launch, which is
    /// what [`path_from_registry`] is for.
    pub(super) fn from_login_shell() -> Result<Option<BTreeMap<String, String>>, String> {
        Ok(None)
    }

    /// `PATH` as the registry has it now: the machine's, then the user's, which is the order
    /// Windows composes them in.
    ///
    /// This is where an installer writes. It broadcasts `WM_SETTINGCHANGE` afterwards, but a
    /// process that is already running keeps the copy it was given, so reading our own
    /// environment again tells us nothing new. Failures are silent on purpose: a missing or
    /// unreadable key just means there is nothing to add, and the launch environment still works.
    pub(super) fn path_from_registry() -> Option<String> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        let read = |root: winreg::HKEY, path: &str| -> Option<String> {
            RegKey::predef(root)
                .open_subkey(path)
                .ok()?
                .get_value::<String, _>("Path")
                .ok()
                .map(|value| expand(&value))
                .filter(|value| !value.is_empty())
        };
        let machine = read(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        );
        let user = read(HKEY_CURRENT_USER, "Environment");
        match (machine, user) {
            (Some(machine), Some(user)) => Some(format!("{machine};{user}")),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        }
    }

    /// Expand `%VAR%` against our own environment, as the registry stores `Path` unexpanded.
    ///
    /// An unknown name is left as it was written: a literal `%…%` is a path that will not match
    /// anything, which is better than silently turning it into an empty entry that would.
    fn expand(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => {
                    let name = &after[..end];
                    match std::env::var(name) {
                        Ok(found) => out.push_str(&found),
                        Err(_) => {
                            out.push('%');
                            out.push_str(name);
                            out.push('%');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    pub(super) fn default_shell(env: &ShellEnv) -> (String, Vec<String>) {
        let cwd = std::env::current_dir().unwrap_or_default();
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if env.find_program(candidate, &cwd).is_some() {
                return (candidate.to_owned(), vec!["-NoLogo".to_owned()]);
            }
        }
        (env.get("COMSPEC").unwrap_or("cmd.exe").to_owned(), vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows bug this exists for: an agent installed while Yardsort was running is on the
    /// registry's `PATH` and not on ours, and **Check again** has to find it.
    #[test]
    fn a_path_entry_added_since_we_started_is_picked_up() {
        let ours = r"C:\Windows\system32;C:\Users\me\AppData\Local\nvm";
        let registry =
            r"C:\Windows\system32;C:\Users\me\AppData\Local\nvm;C:\Users\me\AppData\Roaming\npm";

        let merged = merge_path(ours, registry);

        assert!(
            merged.contains(r"AppData\Roaming\npm"),
            "the new entry should be there: {merged}"
        );
    }

    /// Someone who started Yardsort from a terminal that had added something to `PATH` keeps it:
    /// the registry has never heard of that entry, and replacing rather than merging would drop
    /// it — so an agent that worked a moment ago would stop being found.
    #[test]
    fn what_we_were_launched_with_is_not_thrown_away() {
        let merged = merge_path(
            r"C:\my\toolchain;C:\Windows",
            r"C:\Windows;C:\newly\installed",
        );
        let entries: Vec<&str> = merged.split(';').collect();
        assert_eq!(
            entries,
            [r"C:\my\toolchain", r"C:\Windows", r"C:\newly\installed"],
            "ours first, then what is new, each once"
        );
    }

    #[test]
    fn an_entry_in_both_is_not_repeated_however_it_is_spelt() {
        let merged = merge_path(r"C:\Windows\System32\", r"c:\windows\system32;C:\extra");
        let entries: Vec<&str> = merged.split(';').collect();
        assert_eq!(
            entries,
            [r"C:\Windows\System32\", r"C:\extra"],
            "case and a trailing separator do not make a second entry"
        );
    }

    #[test]
    fn empty_entries_are_dropped_rather_than_carried() {
        assert_eq!(merge_path("", r"C:\only"), r"C:\only");
        assert_eq!(merge_path(r"C:\only", ""), r"C:\only");
        assert_eq!(merge_path("", ""), "");
    }

    #[test]
    fn the_name_of_path_is_found_however_windows_spelt_it() {
        let mut vars = BTreeMap::new();
        vars.insert("Path".to_owned(), "x".to_owned());
        assert_eq!(name_of(&vars, "PATH").as_deref(), Some("Path"));
        assert_eq!(name_of(&BTreeMap::new(), "PATH"), None);
    }

    #[test]
    fn parses_variables_between_markers_and_ignores_shell_noise() {
        let mut dump = b"motd from .zshrc\n".to_vec();
        dump.extend_from_slice(BEGIN);
        dump.extend_from_slice(
            b"PATH=/a:/b\0EMPTY=\0MULTI=line1\nline2\0WITH_EQ=a=b\0SHLVL=3\0_=/x\0",
        );
        dump.extend_from_slice(END);
        dump.extend_from_slice(b"\nlogout\n");

        let vars = parse_dump(&dump).unwrap();

        assert_eq!(vars["PATH"], "/a:/b");
        assert_eq!(vars["EMPTY"], "");
        assert_eq!(vars["MULTI"], "line1\nline2");
        assert_eq!(vars["WITH_EQ"], "a=b");
        assert!(!vars.contains_key("SHLVL") && !vars.contains_key("_"));
    }

    #[test]
    fn rejects_output_without_a_complete_dump() {
        assert!(parse_dump(b"command not found").is_none());
        let mut truncated = BEGIN.to_vec();
        truncated.extend_from_slice(b"PATH=/a\0");
        assert!(parse_dump(&truncated).is_none());
    }

    #[test]
    fn windows_style_lookups_ignore_case_but_prefer_an_exact_match() {
        let vars: BTreeMap<String, String> = [("Path", "C:\\Windows"), ("ComSpec", "cmd.exe")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(lookup(&vars, "PATH", true), Some("C:\\Windows"));
        assert_eq!(lookup(&vars, "COMSPEC", true), Some("cmd.exe"));
        assert_eq!(
            lookup(&vars, "PATH", false),
            None,
            "Unix names are case-sensitive"
        );
        assert_eq!(lookup(&vars, "Path", false), Some("C:\\Windows"));
    }

    #[test]
    fn an_enclosing_agents_session_markers_do_not_reach_our_harnesses() {
        let vars: BTreeMap<String, String> = [
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            // Configuration, not a session marker: the user set these on purpose.
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("ANTHROPIC_API_KEY", "sk-test"),
            ("PATH", "/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let kept: Vec<_> = without_foreign_sessions(vars).into_keys().collect();
        assert_eq!(
            kept,
            ["ANTHROPIC_API_KEY", "CLAUDE_CODE_USE_BEDROCK", "PATH"]
        );
    }

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// Values as a released AppImage leaves them, taken from a running one.
    #[test]
    fn what_an_appimage_adds_does_not_reach_our_sessions() {
        let m = "/tmp/.mount_YardsoJEfjME";
        let launched = vars(&[
            ("APPDIR", m),
            ("APPIMAGE", "/home/u/Applications/Yardsort.AppImage"),
            ("ARGV0", "Yardsort.AppImage"),
            ("OWD", "/home/u"),
            ("GDK_BACKEND", "x11"),
            ("GTK_THEME", "Adwaita:dark"),
            (
                "GTK_PATH",
                &format!("{m}//usr/lib/x86_64-linux-gnu/gtk-3.0:/usr/lib64/gtk-3.0"),
            ),
            ("PYTHONHOME", &format!("{m}/usr/")),
            ("PYTHONPATH", &format!("{m}/usr/share/pyshared/:")),
            (
                "LD_LIBRARY_PATH",
                &format!("{m}/usr/lib/:{m}/usr/lib/x86_64-linux-gnu/:/opt/mine/lib"),
            ),
            (
                "PATH",
                &format!("{m}/usr/bin/:{m}/usr/sbin/:/home/u/bin:/usr/bin"),
            ),
            (
                "XDG_DATA_DIRS",
                &format!("{m}/usr/share/:{m}/usr/share:/usr/local/share:/usr/share"),
            ),
            // The user's own, including a temporary directory that is nobody's mount.
            ("EDITOR", "nvim"),
            ("NOTES", "/tmp/notes/x"),
        ]);

        let env = without_appimage(launched);

        assert_eq!(
            env,
            vars(&[
                ("EDITOR", "nvim"),
                ("LD_LIBRARY_PATH", "/opt/mine/lib"),
                ("NOTES", "/tmp/notes/x"),
                ("PATH", "/home/u/bin:/usr/bin"),
                ("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            ])
        );
    }

    /// After an in-app update the new AppImage is started by the old one, so its environment
    /// carries both mounts: the launcher overwrites some variables and prepends to others.
    #[test]
    fn the_replaced_versions_mount_goes_too() {
        let new = "/tmp/.mount_YardsobDGBCD";
        let old = "/tmp/.mount_YardsoJEfjME";
        let updated = vars(&[
            ("APPDIR", new),
            ("APPIMAGE", "/home/u/Applications/Yardsort.AppImage"),
            ("PYTHONHOME", &format!("{new}/usr/")),
            (
                "LD_LIBRARY_PATH",
                &format!("{new}/usr/lib/:{old}/usr/lib/:{old}/usr/lib64/"),
            ),
            ("PATH", &format!("{new}/usr/bin/:{old}/usr/bin/:/usr/bin")),
            ("XDG_DATA_DIRS", &format!("{old}/usr/share:/usr/share")),
            ("EDITOR", "nvim"),
        ]);

        let env = without_appimage(updated);

        assert_eq!(
            env,
            vars(&[
                ("EDITOR", "nvim"),
                ("PATH", "/usr/bin"),
                ("XDG_DATA_DIRS", "/usr/share"),
            ]),
            "nothing may point into either mount"
        );
    }

    #[test]
    fn a_value_that_is_not_a_path_list_survives_a_bundle_free_environment() {
        let own = vars(&[("GTK_THEME", "Adwaita:dark"), ("TIME", "10:30")]);
        assert_eq!(without_appimage(own.clone()), own);
    }

    #[test]
    fn without_an_appimage_the_environment_is_untouched() {
        let own = vars(&[
            ("GDK_BACKEND", "x11"),
            ("GTK_THEME", "Adwaita:dark"),
            ("APPDIR", "/home/u/some-project"),
            ("PATH", "/home/u/some-project/bin:/usr/bin"),
        ]);
        assert_eq!(without_appimage(own.clone()), own);
    }

    #[test]
    fn process_fallback_carries_the_reason() {
        let env = ShellEnv::from_process(Some("shell timed out".into()));
        assert_eq!(env.source, EnvSource::Process);
        assert_eq!(env.warning.as_deref(), Some("shell timed out"));
        assert!(!env.vars.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn finds_programs_on_this_environments_path_only() {
        let mut env = ShellEnv::from_process(None);
        let cwd = std::env::current_dir().unwrap();
        assert!(env.find_program("sh", &cwd).is_some());
        env.vars.insert("PATH".into(), "/nonexistent".into());
        assert!(env.find_program("sh", &cwd).is_none());
    }
}

// What the launch environment looks like, for the status bar and for bug reports. It lives
// beside the environment it describes so every client can report the same thing.
/// What the launch environment looks like, for the status bar and for bug reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub source: EnvSource,
    pub shell: String,
    pub path_entries: u32,
    pub warning: Option<String>,
}

impl From<&ShellEnv> for EnvInfo {
    fn from(env: &ShellEnv) -> Self {
        let entries = env
            .get("PATH")
            .map(|path| std::env::split_paths(path).count())
            .unwrap_or(0);
        Self {
            source: env.source,
            shell: env.default_shell().0,
            path_entries: u32::try_from(entries).unwrap_or(u32::MAX),
            warning: env.warning.clone(),
        }
    }
}

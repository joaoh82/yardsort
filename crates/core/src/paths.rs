//! Where Yardsort keeps its things, resolved without Tauri.
//!
//! The app asks Tauri for these directories; a command-line client cannot, so the same rules are
//! written out here. They must agree — a CLI that resolved a different directory would open an
//! empty database and show you no projects at all. Two things guard against that: the app checks
//! these against Tauri's answers at startup and complains if they differ (see `lib.rs`), and the
//! CLI refuses to *create* a database, so the worst case is a clear error naming the path it
//! tried rather than a convincing but empty listing.

use std::path::PathBuf;

/// The bundle identifier, as `tauri.conf.json` sets it. Both directories are named after it.
pub const IDENTIFIER: &str = "dev.yardsort.app";

/// The database and the daemon's socket and log. `YARDSORT_DATA_DIR` overrides it, which is how
/// a throwaway profile keeps experiments away from real work.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::legacy::env_var_os("DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    base_data_dir().map(|dir| dir.join(IDENTIFIER))
}

/// `settings.toml`. Under `YARDSORT_DATA_DIR` it sits next to the database instead.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::legacy::env_var_os("DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    base_config_dir().map(|dir| dir.join(IDENTIFIER))
}

/// `dirs::data_dir()`, which is what Tauri's `app_data_dir()` is built on.
fn base_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        xdg("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local").join("share")))
    }
    #[cfg(target_os = "macos")]
    {
        home().map(|h| h.join("Library").join("Application Support"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

/// `dirs::config_dir()`. The same place as the data directory everywhere but Linux.
fn base_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        xdg("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config")))
    }
    #[cfg(not(target_os = "linux"))]
    {
        base_data_dir()
    }
}

/// An XDG variable counts only when it is set to an absolute path, as the spec says.
#[cfg(target_os = "linux")]
fn xdg(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
}

fn home() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialised: these all read process-wide environment variables.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `f` with the given variables set, restoring whatever was there before.
    fn with_env<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let saved: Vec<_> = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in vars {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let out = f();
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        out
    }

    #[test]
    fn a_throwaway_profile_puts_both_in_one_place() {
        with_env(&[("YARDSORT_DATA_DIR", Some("/tmp/ys"))], || {
            assert_eq!(data_dir().unwrap(), PathBuf::from("/tmp/ys"));
            assert_eq!(config_dir().unwrap(), PathBuf::from("/tmp/ys"));
        });
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_follows_xdg_and_falls_back_to_the_usual_places() {
        let vars = [
            ("YARDSORT_DATA_DIR", None),
            ("SWITCHYARD_DATA_DIR", None),
            ("HOME", Some("/home/someone")),
            ("XDG_DATA_HOME", None),
            ("XDG_CONFIG_HOME", None),
        ];
        with_env(&vars, || {
            assert_eq!(
                data_dir().unwrap(),
                PathBuf::from("/home/someone/.local/share/dev.yardsort.app")
            );
            assert_eq!(
                config_dir().unwrap(),
                PathBuf::from("/home/someone/.config/dev.yardsort.app")
            );
        });

        with_env(
            &[("XDG_DATA_HOME", Some("/xdg/data")), vars[0], vars[1]],
            || {
                assert_eq!(
                    data_dir().unwrap(),
                    PathBuf::from("/xdg/data/dev.yardsort.app")
                );
            },
        );

        // A relative XDG path is ignored, per the spec — not joined onto the current directory.
        with_env(
            &[
                ("XDG_DATA_HOME", Some("relative/path")),
                ("HOME", Some("/home/someone")),
                vars[0],
                vars[1],
            ],
            || {
                assert_eq!(
                    data_dir().unwrap(),
                    PathBuf::from("/home/someone/.local/share/dev.yardsort.app")
                );
            },
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_keeps_both_under_application_support() {
        with_env(
            &[
                ("YARDSORT_DATA_DIR", None),
                ("SWITCHYARD_DATA_DIR", None),
                ("HOME", Some("/Users/someone")),
            ],
            || {
                let expected =
                    PathBuf::from("/Users/someone/Library/Application Support/dev.yardsort.app");
                assert_eq!(data_dir().unwrap(), expected);
                assert_eq!(config_dir().unwrap(), expected);
            },
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_uses_roaming_appdata_for_both() {
        with_env(
            &[
                ("YARDSORT_DATA_DIR", None),
                ("SWITCHYARD_DATA_DIR", None),
                ("APPDATA", Some(r"C:\Users\someone\AppData\Roaming")),
            ],
            || {
                let expected = PathBuf::from(r"C:\Users\someone\AppData\Roaming\dev.yardsort.app");
                assert_eq!(data_dir().unwrap(), expected);
                assert_eq!(config_dir().unwrap(), expected);
            },
        );
    }
}

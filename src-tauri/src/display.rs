//! Which display server the window talks to on Linux.
//!
//! The AppImage's GTK launch hook (`linuxdeploy-plugin-gtk`) sets `GDK_BACKEND=x11`
//! unconditionally, to dodge a crash some Wayland systems once had with bundled libraries
//! (tauri-apps/tauri#8541). On a Wayland desktop that puts us under XWayland, where typing feels
//! delayed and text typed by tools such as `wtype` (Omarchy's dictation) arrives dropped or
//! garbled. So when that hook — and nobody else — chose X11 on a Wayland session, we go back to
//! Wayland.
//!
//! In case that crash is still out there, a Wayland start leaves a marker that the loaded page
//! removes. A marker still present at the next start means Wayland never came up; that version
//! then stays on X11. `YARDSORT_GDK_BACKEND` overrides all of this.

use std::path::{Path, PathBuf};

use crate::legacy;

/// Present while a Wayland start has not yet shown the page.
const PENDING: &str = "wayland-pending";
/// Holds the version whose Wayland start failed; that version stays on X11.
const FAILED: &str = "wayland-failed";

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Leave `GDK_BACKEND` as it is.
    Keep,
    /// The user said which backend to use.
    Override(String),
    /// Try Wayland, guarded by the pending marker.
    Wayland,
    /// The last Wayland start never came up: stay on X11 and remember it for this version.
    RecordFailure,
}

struct Facts<'a> {
    override_backend: Option<String>,
    appimage: bool,
    gdk_backend: Option<String>,
    wayland_session: bool,
    pending: bool,
    failed_version: Option<String>,
    version: &'a str,
}

fn decide(facts: &Facts) -> Decision {
    if let Some(backend) = &facts.override_backend {
        return Decision::Override(backend.clone());
    }
    let forced_by_appimage = facts.appimage && facts.gdk_backend.as_deref() == Some("x11");
    if !forced_by_appimage || !facts.wayland_session {
        return Decision::Keep;
    }
    if facts.pending {
        return Decision::RecordFailure;
    }
    if facts.failed_version.as_deref() == Some(facts.version) {
        return Decision::Keep;
    }
    Decision::Wayland
}

/// Where the markers live: the data directory, as Tauri will resolve it later.
fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = legacy::env_var_os("DATA_DIR") {
        return Some(dir.into());
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))?;
    Some(base.join("dev.yardsort.app"))
}

/// Call at the top of `main`, before anything starts GTK or another thread.
pub fn choose_backend() {
    let Some(dir) = data_dir() else { return };
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let facts = Facts {
        override_backend: legacy::env_var_os("GDK_BACKEND")
            .and_then(|value| value.into_string().ok()),
        appimage: var("APPIMAGE").is_some(),
        gdk_backend: var("GDK_BACKEND"),
        wayland_session: var("WAYLAND_DISPLAY").is_some(),
        pending: dir.join(PENDING).exists(),
        failed_version: std::fs::read_to_string(dir.join(FAILED))
            .ok()
            .map(|text| text.trim().to_owned()),
        version: env!("CARGO_PKG_VERSION"),
    };
    match decide(&facts) {
        Decision::Keep => {}
        Decision::Override(backend) => std::env::set_var("GDK_BACKEND", backend),
        Decision::Wayland => {
            // Without the marker we would have no way to notice a crash; better stay on X11.
            let marked =
                std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(PENDING), ""));
            if marked.is_ok() {
                std::env::set_var("GDK_BACKEND", "wayland");
            }
        }
        Decision::RecordFailure => {
            eprintln!(
                "yardsort: the last start on Wayland did not come up; staying on X11 for this \
                 version (set YARDSORT_GDK_BACKEND=wayland to try again)"
            );
            let _ = std::fs::write(dir.join(FAILED), facts.version);
            let _ = std::fs::remove_file(dir.join(PENDING));
        }
    }
}

/// The page has loaded, so whichever backend we chose works.
pub fn window_is_up() {
    if let Some(dir) = data_dir() {
        let _ = std::fs::remove_file(dir.join(PENDING));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appimage_on_wayland() -> Facts<'static> {
        Facts {
            override_backend: None,
            appimage: true,
            gdk_backend: Some("x11".into()),
            wayland_session: true,
            pending: false,
            failed_version: None,
            version: "0.3.2",
        }
    }

    #[test]
    fn an_appimage_forced_onto_xwayland_goes_back_to_wayland() {
        assert_eq!(decide(&appimage_on_wayland()), Decision::Wayland);
    }

    #[test]
    fn a_wayland_start_that_never_came_up_falls_back_to_x11_for_that_version() {
        let crashed = Facts {
            pending: true,
            ..appimage_on_wayland()
        };
        assert_eq!(decide(&crashed), Decision::RecordFailure);

        let after = Facts {
            failed_version: Some("0.3.2".into()),
            ..appimage_on_wayland()
        };
        assert_eq!(decide(&after), Decision::Keep);

        let next_version = Facts {
            failed_version: Some("0.3.1".into()),
            ..appimage_on_wayland()
        };
        assert_eq!(decide(&next_version), Decision::Wayland);
    }

    #[test]
    fn other_installs_and_x11_sessions_are_left_alone() {
        let not_appimage = Facts {
            appimage: false,
            ..appimage_on_wayland()
        };
        let x11_session = Facts {
            wayland_session: false,
            ..appimage_on_wayland()
        };
        let own_choice = Facts {
            gdk_backend: None,
            ..appimage_on_wayland()
        };
        for facts in [not_appimage, x11_session, own_choice] {
            assert_eq!(decide(&facts), Decision::Keep);
        }
    }

    #[test]
    fn the_override_wins_over_everything() {
        let facts = Facts {
            override_backend: Some("x11".into()),
            pending: true,
            ..appimage_on_wayland()
        };
        assert_eq!(decide(&facts), Decision::Override("x11".into()));
    }
}

//! In-app updates.
//!
//! A copy of Yardsort that owns its own files — the macOS app, a Windows install, a Linux
//! AppImage — can replace itself: check, download, verify the signature, install, restart. A copy
//! that a package manager owns (`.deb`, `.rpm`, the AUR package built from the `.deb`) must not;
//! it is only *told* that a newer version exists. Either way nothing happens without the user
//! asking for it.
//!
//! Updates are verified against the public key in `tauri.conf.json`. The private half signs them
//! in the release workflow and is the project's most important secret: see `docs/releasing.md`.

use std::sync::Mutex;

use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::utils::config::BundleType;
use tauri::utils::platform::bundle_type;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{IpcError, IpcResult};

/// How this copy of the app gets new versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    /// It can download and install an update itself.
    SelfUpdating,
    /// A package manager owns the files; the user updates through it.
    PackageManager,
    /// A development build. Never updates.
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub install_kind: InstallKind,
    /// The newer release, if there is one.
    pub available: Option<AvailableUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    /// The release notes, as written on the release.
    pub notes: Option<String>,
    /// Where to read about it and download it by hand.
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u32,
    /// `None` when the server did not say how big the download is.
    pub total: Option<u32>,
}

/// The update found by the last check, kept so that "install" installs exactly what was shown.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

fn install_kind_for(debug: bool, bundle: Option<&BundleType>) -> InstallKind {
    match bundle {
        _ if debug => InstallKind::Development,
        Some(BundleType::Deb | BundleType::Rpm) => InstallKind::PackageManager,
        Some(_) => InstallKind::SelfUpdating,
        // Not packaged by Tauri at all: someone built and copied the binary themselves.
        None => InstallKind::PackageManager,
    }
}

pub fn install_kind() -> InstallKind {
    install_kind_for(cfg!(debug_assertions), bundle_type().as_ref())
}

fn release_url(version: &str) -> String {
    format!("https://github.com/joaoh82/yardsort/releases/tag/v{version}")
}

fn failed(action: &str, error: impl std::fmt::Display) -> IpcError {
    IpcError::new("update_failed", format!("Could not {action}: {error}"))
}

/// Ask whether a newer release exists. Touches nothing on disk.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> IpcResult<UpdateStatus> {
    let mut builder = app.updater_builder();
    // For testing the whole flow against a release server of one's own.
    if let Some(endpoint) = crate::legacy::env_var_os("UPDATE_ENDPOINT") {
        let url = endpoint
            .to_string_lossy()
            .parse()
            .map_err(|e| failed("use YARDSORT_UPDATE_ENDPOINT", e))?;
        builder = builder
            .endpoints(vec![url])
            .map_err(|e| failed("use YARDSORT_UPDATE_ENDPOINT", e))?;
    }
    let updater = builder
        .build()
        .map_err(|e| failed("check for updates", e))?;
    let update = updater
        .check()
        .await
        .map_err(|e| failed("check for updates", e))?;

    let status = UpdateStatus {
        current_version: app.package_info().version.to_string(),
        install_kind: install_kind(),
        available: update.as_ref().map(|update| AvailableUpdate {
            version: update.version.clone(),
            notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
            url: release_url(&update.version),
        }),
    };
    *app.state::<PendingUpdate>()
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = update;
    Ok(status)
}

/// Download the update found by the last check, verify its signature, install it and restart.
/// Only for [`InstallKind::SelfUpdating`] copies.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle, progress: Channel<DownloadProgress>) -> IpcResult<()> {
    if install_kind() != InstallKind::SelfUpdating {
        return Err(IpcError::new(
            "update_not_supported",
            "This copy of Yardsort is updated through your package manager.",
        ));
    }
    let update = app
        .state::<PendingUpdate>()
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| IpcError::new("no_update", "Check for updates first."))?;

    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let clamp = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
                let _ = progress.send(DownloadProgress {
                    downloaded: clamp(downloaded),
                    total: total.map(clamp),
                });
            },
            || {},
        )
        .await
        .map_err(|e| failed("install the update", e))?;

    // On Windows the installer has already taken over; elsewhere, start the new version.
    restart(&app)
}

/// Start the new version and leave.
///
/// Tauri's restart hands the new process our environment as it is, and ours is what the old
/// AppImage's launcher made: its mount on `LD_LIBRARY_PATH`, `XDG_DATA_DIRS` and a dozen more.
/// The new launcher only prepends its own, so a library the new version no longer bundles is
/// still found in the old mount — which stays mounted while the new process holds it — and the
/// user runs a mix of two versions until they start the app afresh. So an AppImage is started
/// by hand, with the environment the user launched us with (see `without_appimage`). Everything
/// else restarts as Tauri does it; the macOS app, for one, may have been renamed by the update.
fn restart(app: &AppHandle) -> IpcResult<()> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = app.env().appimage.clone() {
        let args = app.env().args_os.into_iter().skip(1);
        relaunch(&appimage, args, std::env::vars().collect())
            .map_err(|e| failed("start the new version", e))?;
        app.exit(0);
        return Ok(());
    }
    app.restart()
}

/// Start `appimage` with `args` and the user's part of `vars`, detached from us.
#[cfg(target_os = "linux")]
fn relaunch(
    appimage: &std::ffi::OsStr,
    args: impl IntoIterator<Item = std::ffi::OsString>,
    vars: std::collections::BTreeMap<String, String>,
) -> std::io::Result<()> {
    std::process::Command::new(appimage)
        .args(args)
        .env_clear()
        .envs(yardsort_core::env::without_appimage(vars))
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_copies_that_own_their_files_update_themselves() {
        use InstallKind::{Development, PackageManager, SelfUpdating};
        for (bundle, expected) in [
            (Some(BundleType::AppImage), SelfUpdating),
            (Some(BundleType::Nsis), SelfUpdating),
            (Some(BundleType::Msi), SelfUpdating),
            (Some(BundleType::App), SelfUpdating),
            // dpkg, rpm and pacman (the AUR package unpacks the .deb) own these files.
            (Some(BundleType::Deb), PackageManager),
            (Some(BundleType::Rpm), PackageManager),
            (None, PackageManager),
        ] {
            assert_eq!(
                install_kind_for(false, bundle.as_ref()),
                expected,
                "{bundle:?}"
            );
            assert_eq!(
                install_kind_for(true, bundle.as_ref()),
                Development,
                "debug builds never update"
            );
        }
    }

    /// The relaunched AppImage sees the environment the user started us with: nothing that leads
    /// into our mount or the one we replaced, and nothing the launcher's hook overwrote.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_new_version_is_not_started_with_the_old_ones_environment() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("env");
        let appimage = dir.path().join("Yardsort.AppImage");
        yardsort_core::program::testing::executable(
            &appimage,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {0}\nenv >> {0}\n",
                out.display()
            ),
        );
        let old = "/tmp/.mount_YardsoIdMAJc";
        let new = "/tmp/.mount_YardsogDGmCf";
        let vars = [
            ("APPDIR", new.to_owned()),
            ("APPIMAGE", appimage.display().to_string()),
            ("OWD", format!("{old}/usr")),
            ("GDK_BACKEND", "wayland".to_owned()),
            ("GTK_PATH", format!("{new}//usr/lib/gtk-3.0")),
            (
                "LD_LIBRARY_PATH",
                format!("{new}/usr/lib/:{old}/usr/lib/:/opt/mine/lib"),
            ),
            (
                "XDG_DATA_DIRS",
                format!("{new}/usr/share/:{old}/usr/share:/usr/share"),
            ),
            ("PATH", std::env::var("PATH").unwrap()),
            ("EDITOR", "nvim".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();

        relaunch(
            appimage.as_os_str(),
            [std::ffi::OsString::from("--flag")],
            vars,
        )
        .unwrap();

        let started = std::time::Instant::now();
        let dump = loop {
            match std::fs::read_to_string(&out) {
                Ok(text) if text.contains("EDITOR=") => break text,
                _ if started.elapsed() > Duration::from_secs(10) => panic!("never started"),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let mut lines = dump.lines();
        assert_eq!(lines.next(), Some("--flag"), "arguments are passed on");
        let env: Vec<&str> = lines.collect();
        assert!(
            !env.iter().any(|line| line.contains(".mount_")),
            "a mount reached the new version:\n{dump}"
        );
        assert!(env.contains(&"EDITOR=nvim"));
        assert!(env.contains(&"LD_LIBRARY_PATH=/opt/mine/lib"));
        assert!(env.contains(&"XDG_DATA_DIRS=/usr/share"));
        assert!(
            !env.iter().any(|line| line.starts_with("GDK_BACKEND=")),
            "the new version chooses its own display backend"
        );
    }

    #[test]
    fn the_manual_download_link_points_at_the_release() {
        assert_eq!(
            release_url("0.3.1"),
            "https://github.com/joaoh82/yardsort/releases/tag/v0.3.1"
        );
    }
}

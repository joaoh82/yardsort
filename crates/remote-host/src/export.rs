//! Exporting this computer's account: everything its server keeps of it, as one JSON file its
//! person keeps (`remote_proto::export`). The desktop app asks from Settings → Remote, a headless
//! host from `yardsort-host account export`; both are a person's act, never an agent's.
//!
//! The file holds every synced note and workspace name of the account, so it is written readable
//! by this user only, and nothing of it is logged.

use std::io::Write;
use std::path::Path;

use remote_client::ClientError;
use remote_proto::export::AccountExport;
use remote_proto::ErrorCode;

use crate::account::{client, enabled, now_ms, refused_here, Failure, Home, Result};

/// The whole export, every page of it, from this computer's server.
pub async fn export(home: &impl Home) -> Result<AccountExport> {
    let row = enabled(home)?;
    let (client, access) = client(home, &row).await?;
    client
        .export(&access, now_ms())
        .await
        .map_err(|error| match error {
            ClientError::Remote(ref e) if e.code == ErrorCode::NotFound => Failure::new(
                "remote_refused",
                "This server cannot export an account yet. Its operator has to update it.",
            ),
            error => refused_here(error),
        })
}

/// Write `export` to `path` as JSON, readable by this user only. An existing file is kept unless
/// `replace`; it is then replaced whole, never left half written.
pub fn save(export: &AccountExport, path: &Path, replace: bool) -> Result<()> {
    let failed = |error: std::io::Error| {
        Failure::new(
            "export_not_saved",
            format!(
                "The export could not be saved at {}: {error}",
                path.display()
            ),
        )
    };
    if !replace && path.exists() {
        return Err(Failure::new(
            "export_not_saved",
            format!("{} is there already. Choose another name.", path.display()),
        ));
    }
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(failed)?;
    serde_json::to_writer_pretty(&mut file, export).map_err(|e| failed(e.into()))?;
    file.write_all(b"\n").map_err(failed)?;
    file.as_file().sync_all().map_err(failed)?;
    // The temporary file is this user's only already; persisting keeps that.
    if replace {
        file.persist(path).map_err(|e| failed(e.error))?;
    } else {
        file.persist_noclobber(path).map_err(|e| failed(e.error))?;
    }
    Ok(())
}

/// A file name for an export made now: `yardsort-account-2026-11-17.json`.
pub fn file_name(exported_at: i64) -> String {
    let days = exported_at.div_euclid(86_400_000);
    let (year, month, day) = civil(days);
    format!("yardsort-account-{year:04}-{month:02}-{day:02}.json")
}

/// The calendar date `days` after 1970-01-01, in UTC (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use remote_proto::export::{ExportedAccount, EXPORT_FORMAT};
    use remote_proto::notify::NotificationPreferences;
    use remote_proto::ServerId;

    fn sample() -> AccountExport {
        AccountExport {
            format: EXPORT_FORMAT,
            server: ServerId::new("4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e").unwrap(),
            server_name: "Example".into(),
            exported_at: 1_800_000_000_000,
            account: ExportedAccount {
                id: "account".into(),
                issuer: "https://id.example".into(),
                subject: "ada".into(),
                created_at: 1,
                devices: Vec::new(),
                hosts: Vec::new(),
                grants: Vec::new(),
                push_routes: Vec::new(),
                notifications: NotificationPreferences::default(),
            },
            records: Vec::new(),
        }
    }

    #[test]
    fn an_export_is_saved_whole_and_an_existing_file_is_kept_unless_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file_name(sample().exported_at));
        assert!(path.ends_with("yardsort-account-2027-01-15.json"));
        save(&sample(), &path, false).unwrap();
        let read: AccountExport =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read, sample());

        std::fs::write(&path, "mine").unwrap();
        let refused = save(&sample(), &path, false).unwrap_err();
        assert_eq!(refused.code, "export_not_saved");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine", "kept");
        save(&sample(), &path, true).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no stray file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_export_is_readable_by_this_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.json");
        save(&sample(), &path, false).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(
            file_name(1_763_337_600_000),
            "yardsort-account-2025-11-17.json"
        );
    }
}

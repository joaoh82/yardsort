//! This machine as a host: which server it is on, what it may keep secret, and which devices its
//! person let in.
//!
//! **Grants are this host's own record.** One is kept only when this host's person approves a
//! device here ([`keep_approved`]); what the server lists can take a grant away
//! ([`Store::retain_remote_grants`]) but never add one. A device is let in when both agree: the
//! server, which knows the device is signed in and its grant is in force, and this record, which
//! knows this host's person said yes ([`grant_for`]). A server that lied could not add a device.
//!
//! **Credentials** never go in the database, a settings file or a terminal's environment. They
//! are kept in a [`CredentialStore`]: the OS credential store on a desktop (the app provides
//! it), or [`FileCredentials`] where there is none, which is a directory only its owner can read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use remote_proto::account::Secret;
use remote_proto::{DeviceId, Grant, GrantId, ProjectGrant, Scope};

use crate::store::{RemoteGrantRow, Store, StoreResult};

/// The credential-store entry for this host's refresh credential.
pub const REFRESH_ENTRY: &str = "remote-host-refresh";

/// Somewhere to keep a secret by name.
pub trait CredentialStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Secret>, String>;
    fn set(&self, name: &str, secret: &Secret) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Secrets as files in one directory, each readable by its owner alone: for a host with no OS
/// credential store (a server, a container), where the directory is a mount kept apart from the
/// data directory and its backups.
///
/// On Unix the directory is made `0700` and each file `0600`, and an existing directory others
/// can read is refused. On Windows the files take the directory's permissions, which under the
/// user's profile are the user's own.
pub struct FileCredentials {
    dir: PathBuf,
}

impl FileCredentials {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| {
                        format!(
                            "{} can be read by others and cannot be made private: {e}",
                            dir.display()
                        )
                    },
                )?;
            }
        }
        Ok(Self { dir })
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        let plain = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !plain {
            return Err(format!("{name:?} is not a credential name"));
        }
        Ok(self.dir.join(name))
    }
}

impl CredentialStore for FileCredentials {
    fn get(&self, name: &str) -> Result<Option<Secret>, String> {
        match std::fs::read_to_string(self.path(name)?) {
            Ok(text) => Ok(Some(Secret::new(text.trim_end().to_owned()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, name: &str, secret: &Secret) -> Result<(), String> {
        let path = self.path(name)?;
        // Written beside it, then renamed over it, so a crash leaves the old one or the new one.
        let partial = path.with_extension("partial");
        write_private(&partial, secret.expose().as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &path).map_err(|e| e.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Secrets in this process alone, for tests.
#[derive(Default)]
pub struct MemoryCredentials {
    secrets: std::sync::Mutex<std::collections::HashMap<String, Secret>>,
}

impl CredentialStore for MemoryCredentials {
    fn get(&self, name: &str) -> Result<Option<Secret>, String> {
        Ok(self.lock()?.get(name).cloned())
    }

    fn set(&self, name: &str, secret: &Secret) -> Result<(), String> {
        self.lock()?.insert(name.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.lock()?.remove(name);
        Ok(())
    }
}

impl MemoryCredentials {
    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, std::collections::HashMap<String, Secret>>, String> {
        self.secrets.lock().map_err(|_| "poisoned".to_owned())
    }
}

/// A device this host's person let in, as kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalGrant {
    pub id: GrantId,
    pub device_name: String,
    pub grant: Grant,
    pub approved_at: i64,
}

/// Keep what this host's person just approved for `device`. The only way a grant is written.
pub fn keep_approved(
    store: &Store,
    id: &GrantId,
    device: &DeviceId,
    device_name: &str,
    scopes: &BTreeSet<Scope>,
    projects: &ProjectGrant,
    approved_at: i64,
) -> StoreResult<()> {
    store.keep_remote_grant(&RemoteGrantRow {
        id: id.to_string(),
        device_id: device.to_string(),
        device_name: device_name.to_owned(),
        scopes: serde_json::to_string(scopes).unwrap_or_else(|_| "[]".into()),
        projects: serde_json::to_string(projects).unwrap_or_default(),
        approved_at,
    })
}

/// The devices let in here. A row that no longer reads (written by a newer build, say) is left
/// out rather than read as more than it says.
pub fn local_grants(store: &Store) -> StoreResult<Vec<LocalGrant>> {
    Ok(store
        .remote_grants()?
        .into_iter()
        .filter_map(|row| {
            let id = GrantId::new(row.id).ok()?;
            let device = DeviceId::new(row.device_id).ok()?;
            let scopes: BTreeSet<Scope> = serde_json::from_str(&row.scopes).ok()?;
            let projects: ProjectGrant = serde_json::from_str(&row.projects).ok()?;
            Some(LocalGrant {
                id,
                device_name: row.device_name,
                grant: Grant {
                    device,
                    scopes,
                    projects,
                },
                approved_at: row.approved_at,
            })
        })
        .collect())
}

/// What `device` may do here: its grant, if this host's person approved one and the server
/// still has it in force (`server_in_force`, the grant the server named for it). Anything else,
/// including a grant only the server knows of, or one for another device, is nothing.
pub fn grant_for(
    store: &Store,
    device: &DeviceId,
    server_in_force: &GrantId,
) -> StoreResult<Option<Grant>> {
    Ok(local_grants(store)?
        .into_iter()
        .find(|g| g.grant.device == *device && g.id == *server_in_force)
        .map(|g| g.grant))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
        (dir, store)
    }

    fn id(text: &str) -> GrantId {
        GrantId::new(text).unwrap()
    }

    fn device(text: &str) -> DeviceId {
        DeviceId::new(text).unwrap()
    }

    fn host(server: &str, host: &str) -> crate::store::RemoteHostRow {
        crate::store::RemoteHostRow {
            server_url: format!("https://{server}.example"),
            server_id: server.into(),
            host_id: host.into(),
            device_id: "host-device".into(),
            name: "Workstation".into(),
            enabled_at: 1,
        }
    }

    fn keep(store: &Store, grant: &str, phone: &str, at: i64) {
        keep_approved(
            store,
            &id(grant),
            &device(phone),
            "Phone",
            &[Scope::WorkspaceRead].into(),
            &ProjectGrant::All,
            at,
        )
        .unwrap();
    }

    #[test]
    fn a_device_is_let_in_only_when_this_host_and_the_server_agree() {
        let (_dir, store) = store();
        store.set_remote_host(&host("srv", "h1")).unwrap();
        keep(&store, "g1", "phone", 10);
        let let_in = grant_for(&store, &device("phone"), &id("g1"))
            .unwrap()
            .unwrap();
        assert_eq!(let_in.device, device("phone"));
        assert_eq!(let_in.scopes, [Scope::WorkspaceRead].into());
        // A grant only the server knows of.
        assert_eq!(
            grant_for(&store, &device("phone"), &id("g2")).unwrap(),
            None
        );
        // Another device naming this one's grant.
        assert_eq!(
            grant_for(&store, &device("laptop"), &id("g1")).unwrap(),
            None
        );
        // A new approval for the same device replaces the old one.
        keep(&store, "g3", "phone", 20);
        assert_eq!(
            grant_for(&store, &device("phone"), &id("g1")).unwrap(),
            None
        );
        assert!(grant_for(&store, &device("phone"), &id("g3"))
            .unwrap()
            .is_some());
    }

    #[test]
    fn the_server_can_take_grants_away_but_never_add_one() {
        let (_dir, store) = store();
        store.set_remote_host(&host("srv", "h1")).unwrap();
        keep(&store, "g1", "phone", 10);
        keep(&store, "g2", "laptop", 11);
        let dropped = store
            .retain_remote_grants(&["g2".into(), "g-unknown".into()])
            .unwrap();
        assert_eq!(dropped, 1);
        let left: Vec<_> = local_grants(&store)
            .unwrap()
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(left, vec![id("g2")]);
        assert!(store.drop_remote_grant("g2").unwrap());
        assert!(!store.drop_remote_grant("g2").unwrap());
        assert!(local_grants(&store).unwrap().is_empty());
    }

    #[test]
    fn grants_belong_to_the_host_and_server_they_were_given_on() {
        let (_dir, store) = store();
        assert_eq!(store.remote_host().unwrap(), None);
        store.set_remote_host(&host("srv", "h1")).unwrap();
        keep(&store, "g1", "phone", 10);
        // Signing in again on the same server, as the same host, keeps them.
        store
            .set_remote_host(&crate::store::RemoteHostRow {
                device_id: "new-device".into(),
                ..host("srv", "h1")
            })
            .unwrap();
        assert_eq!(local_grants(&store).unwrap().len(), 1);
        assert_eq!(
            store.remote_host().unwrap().unwrap().device_id,
            "new-device"
        );
        // Another server's host does not inherit them.
        store.set_remote_host(&host("other", "h9")).unwrap();
        assert!(local_grants(&store).unwrap().is_empty());
        keep(&store, "g2", "phone", 10);
        store.forget_remote_host().unwrap();
        assert_eq!(store.remote_host().unwrap(), None);
        assert!(local_grants(&store).unwrap().is_empty());
    }

    #[test]
    fn a_row_this_build_cannot_read_lets_nobody_in() {
        let (_dir, store) = store();
        store
            .keep_remote_grant(&RemoteGrantRow {
                id: "g1".into(),
                device_id: "phone".into(),
                device_name: "Phone".into(),
                scopes: r#"["workspace:read","teleport:anywhere"]"#.into(),
                projects: r#"{"kind":"all"}"#.into(),
                approved_at: 1,
            })
            .unwrap();
        assert!(local_grants(&store).unwrap().is_empty());
        assert_eq!(
            grant_for(&store, &device("phone"), &id("g1")).unwrap(),
            None
        );
    }

    #[test]
    fn file_credentials_are_private_and_survive_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("credentials");
        let store = FileCredentials::new(&root).unwrap();
        assert_eq!(store.get(REFRESH_ENTRY).unwrap(), None);
        store
            .set(REFRESH_ENTRY, &Secret::new("ys1.rt.first"))
            .unwrap();
        store
            .set(REFRESH_ENTRY, &Secret::new("ys1.rt.second"))
            .unwrap();
        assert_eq!(
            store.get(REFRESH_ENTRY).unwrap(),
            Some(Secret::new("ys1.rt.second"))
        );
        assert!(!root.join("remote-host-refresh.partial").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&root), 0o700);
            assert_eq!(mode(&root.join(REFRESH_ENTRY)), 0o600);
        }
        store.delete(REFRESH_ENTRY).unwrap();
        store.delete(REFRESH_ENTRY).unwrap();
        assert_eq!(store.get(REFRESH_ENTRY).unwrap(), None);
        // A name is never a path.
        for bad in ["../escape", "a/b", "", "UPPER"] {
            assert!(store.set(bad, &Secret::new("x")).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_credentials_directory_others_can_read_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("credentials");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        FileCredentials::new(&root).unwrap();
        let mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}

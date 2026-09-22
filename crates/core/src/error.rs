use pty_host::HostError;
use serde::Serialize;
use specta::Type;

/// The one error shape that crosses IPC: a stable `code` for the UI to branch on and a
/// human-readable `message` to show.
#[derive(Debug, Clone, Serialize, Type)]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

pub type IpcResult<T> = Result<T, IpcError>;

impl IpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl From<HostError> for IpcError {
    fn from(error: HostError) -> Self {
        let code = match &error {
            HostError::UnknownSession(_) => "unknown_session",
            HostError::SessionExited(_) => "session_exited",
            HostError::OpenPty(_) => "pty_unavailable",
            HostError::Spawn { .. } => "spawn_failed",
            HostError::Io(_) => "io",
        };
        Self::new(code, error.to_string())
    }
}

impl From<crate::git::GitError> for IpcError {
    fn from(error: crate::git::GitError) -> Self {
        let code = match &error {
            crate::git::GitError::NotInstalled => "git_not_installed",
            crate::git::GitError::Failed { .. } => "git_failed",
            crate::git::GitError::Io(_) => "io",
        };
        Self::new(code, error.to_string())
    }
}

impl From<crate::store::StoreError> for IpcError {
    fn from(error: crate::store::StoreError) -> Self {
        Self::new("database", error.to_string())
    }
}

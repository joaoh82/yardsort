//! Identifiers, one type per thing identified.
//!
//! The remote design keeps apart what is easy to blur: the server a person signed in to, the
//! account on it, a device's grant, the host that runs agents, the project and workspace there,
//! a conversation, one process run, and the terminal a process draws in. Giving each its own type
//! means a workspace id cannot be handed to something expecting a project id without the compiler
//! saying so.
//!
//! None of them is authorization. Knowing an id grants nothing: every request is checked against
//! the caller's [`crate::Grant`] whatever ids it names.
//!
//! On the wire each is a plain string. Every one that arrives from outside is checked before it
//! is used: at most [`MAX_ID_LEN`] bytes of ASCII letters, digits and `-_.:`. That excludes paths,
//! whitespace and control characters, so an id can never smuggle one into a log or a query.

use serde::{Deserialize, Serialize};

/// Longest id accepted. Local ids are UUIDs (36 bytes); the slack is for ids a server issues.
pub const MAX_ID_LEN: usize = 128;

/// Why a string is not an id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("an id cannot be empty")]
    Empty,
    #[error("an id is at most {MAX_ID_LEN} bytes")]
    TooLong,
    #[error("an id may only contain ASCII letters, digits and - _ . :")]
    BadCharacter,
}

/// Whether `text` is acceptable as an id of any kind.
pub fn check_id(text: &str) -> Result<(), IdError> {
    if text.is_empty() {
        return Err(IdError::Empty);
    }
    if text.len() > MAX_ID_LEN {
        return Err(IdError::TooLong);
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
    {
        return Err(IdError::BadCharacter);
    }
    Ok(())
}

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Result<Self, IdError> {
                let text = text.into();
                check_id(&text)?;
                Ok(Self(text))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(text: String) -> Result<Self, IdError> {
                Self::new(text)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// A Yardsort server: managed or self-hosted. Accounts, grants and hosts all belong to one.
    ServerId
);
id!(
    /// A device a person enrolled: a phone, or another computer acting as a client.
    DeviceId
);
id!(
    /// A machine that runs agents and answers for its own workspaces.
    HostId
);
id!(
    /// A project, as its host knows it. Host-scoped: the same repository on two machines has two
    /// project ids until sync links them explicitly.
    ProjectId
);
id!(
    /// A workspace, as its host knows it. Host-scoped, like [`ProjectId`].
    WorkspaceId
);
id!(
    /// One agent conversation's record. Resuming it starts a new run, not a new session.
    SessionRecordId
);
id!(
    /// One process: a single launch of a harness, shell or program.
    RunId
);
id!(
    /// A terminal a client can view, on its host. Opaque to clients.
    TerminalId
);
id!(
    /// A memory entry.
    MemoryId
);
id!(
    /// One recorded activity event. Stable: the same fact keeps its id across pages and
    /// reconnects, so a client can drop one it already has.
    EventId
);
id!(
    /// A control lease on one terminal.
    LeaseId
);
id!(
    /// Chosen by the client, echoed in the reply, so replies can be matched to requests.
    RequestId
);

/// An account is not an email address: it is who an identity provider says signed in, on one
/// server. Two servers' accounts are never the same account, whatever their emails say.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRef {
    pub server: ServerId,
    /// The identity provider's issuer URL.
    pub issuer: String,
    /// The provider's subject for this person: stable, and meaningless anywhere else.
    pub subject: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_and_server_ids_are_ids() {
        for ok in [
            "0b6c1f0e-8f8e-4a57-9a43-5c1d2a0e9f11",
            "host_01HZX",
            "srv:example.test",
            "a",
        ] {
            assert!(WorkspaceId::new(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn paths_whitespace_and_control_characters_are_not() {
        for bad in [
            "",
            "/home/someone/project",
            "C:\\Users\\someone",
            "a b",
            "line\nbreak",
            "esc\u{1b}[2J",
            "ünïcode",
        ] {
            assert!(WorkspaceId::new(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            WorkspaceId::new("x".repeat(MAX_ID_LEN + 1)),
            Err(IdError::TooLong)
        );
        assert!(WorkspaceId::new("x".repeat(MAX_ID_LEN)).is_ok());
    }

    #[test]
    fn a_bad_id_fails_to_decode_rather_than_reaching_a_handler() {
        let decoded: Result<WorkspaceId, _> = serde_json::from_str("\"../../etc/passwd\"");
        assert!(decoded.is_err());
        let decoded: WorkspaceId = serde_json::from_str("\"ws-1\"").unwrap();
        assert_eq!(decoded.as_str(), "ws-1");
    }
}

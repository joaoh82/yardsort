//! What a caller is allowed to do: the scopes and resources of one device's grant.
//!
//! A host checks every request against a [`Grant`] before it reads anything. A grant comes from
//! pairing ([`crate::pairing`]): a device asks, and the host's person approves it at the host,
//! which keeps what they chose. Nothing in this crate can mint a grant from a request: it is
//! always the host's own input.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, ProjectId};

/// One permission. Scopes are separate so that, for instance, a device can watch a terminal
/// without being able to type into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    /// Projects, workspaces and their conversations' metadata.
    #[serde(rename = "workspace:read")]
    WorkspaceRead,
    /// Recorded activity: runs and lifecycle events.
    #[serde(rename = "activity:read")]
    ActivityRead,
    /// Approved memory entries. Never candidates, never rejected or revoked ones.
    #[serde(rename = "memory:read")]
    MemoryRead,
    /// Watching a terminal.
    #[serde(rename = "terminal:read")]
    TerminalRead,
    /// Typing into a terminal and resizing it. This is the host user's own power over their
    /// machine, whatever runs in the terminal — no smaller.
    #[serde(rename = "terminal:control")]
    TerminalControl,
    /// Seeing and revoking enrolled devices.
    #[serde(rename = "device:manage")]
    DeviceManage,
    /// Listing workflow runs and reading one run. Not deciding it.
    #[serde(rename = "workflow:read")]
    WorkflowRead,
    /// Approving, answering, or resuming a workflow the desktop is running.
    /// A host token is still refused, even with this scope.
    #[serde(rename = "workflow:decide")]
    WorkflowDecide,
    /// A scope this build does not know. Accepted on parse and then dropped, so a newer
    /// peer does not make pairing fail. Never stored and never granted.
    #[serde(other)]
    Unknown,
}

/// Read and write a set of scopes. A name this build does not know is dropped.
pub mod scope_set {
    use std::collections::BTreeSet;
    use std::fmt;

    use serde::de::{SeqAccess, Visitor};
    use serde::ser::SerializeSeq;
    use serde::{Deserializer, Serializer};

    use super::Scope;

    pub fn serialize<S>(scopes: &BTreeSet<Scope>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let known: Vec<&Scope> = scopes
            .iter()
            .filter(|scope| **scope != Scope::Unknown)
            .collect();
        let mut seq = serializer.serialize_seq(Some(known.len()))?;
        for scope in known {
            seq.serialize_element(scope)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeSet<Scope>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visit;
        impl<'de> Visitor<'de> for Visit {
            type Value = BTreeSet<Scope>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a list of scopes")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut set = BTreeSet::new();
                while let Some(scope) = seq.next_element::<Scope>()? {
                    if scope != Scope::Unknown {
                        set.insert(scope);
                    }
                }
                Ok(set)
            }
        }
        deserializer.deserialize_seq(Visit)
    }
}

/// Which projects a grant reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "projects")]
pub enum ProjectGrant {
    /// Every project on the host, including ones added later.
    All,
    /// Only these.
    Only(BTreeSet<ProjectId>),
}

/// A device's permission on one host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub device: DeviceId,
    #[serde(with = "scope_set")]
    pub scopes: BTreeSet<Scope>,
    pub projects: ProjectGrant,
}

impl Grant {
    pub fn allows(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }

    pub fn reaches(&self, project: &ProjectId) -> bool {
        match &self.projects {
            ProjectGrant::All => true,
            ProjectGrant::Only(projects) => projects.contains(project),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_have_their_wire_names() {
        let json = serde_json::to_string(&[Scope::WorkspaceRead, Scope::TerminalControl]).unwrap();
        assert_eq!(json, r#"["workspace:read","terminal:control"]"#);
    }

    #[test]
    fn a_grant_reaches_only_its_projects() {
        let mine = ProjectId::new("p1").unwrap();
        let theirs = ProjectId::new("p2").unwrap();
        let grant = Grant {
            device: DeviceId::new("d1").unwrap(),
            scopes: [Scope::WorkspaceRead].into(),
            projects: ProjectGrant::Only([mine.clone()].into()),
        };
        assert!(grant.reaches(&mine));
        assert!(!grant.reaches(&theirs));
        assert!(grant.allows(Scope::WorkspaceRead));
        assert!(!grant.allows(Scope::MemoryRead));
    }

    #[test]
    fn an_unknown_scope_is_accepted_and_dropped() {
        #[derive(Deserialize)]
        struct Asked {
            #[serde(with = "super::scope_set")]
            scopes: BTreeSet<Scope>,
        }
        let asked: Asked = serde_json::from_str(
            r#"{"scopes":["workspace:read","workflow:later","workflow:read"]}"#,
        )
        .unwrap();
        assert_eq!(
            asked.scopes,
            [Scope::WorkspaceRead, Scope::WorkflowRead].into()
        );
        let back = serde_json::to_string(&asked.scopes).unwrap();
        assert!(!back.contains("later"));
        assert!(back.contains("workflow:read"));
    }
}

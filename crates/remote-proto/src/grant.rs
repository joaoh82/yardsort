//! What a caller is allowed to do: the scopes and resources of one device's grant.
//!
//! A host checks every request against a [`Grant`] before it reads anything. Where the grant
//! comes from is not decided here. In this slice only tests construct one; enrollment, approval
//! on the host and revocation (phase R2) will. Nothing in this crate can mint a grant from a
//! request: it is always the host's own input.

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
}

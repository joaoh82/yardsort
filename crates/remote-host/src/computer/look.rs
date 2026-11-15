//! Reading a workspace on another computer from this one: what changed in it, its file tree, and
//! a file's contents. See docs/design/44-other-computers-like-local.md.
//!
//! **By that host's ids, and a path inside the workspace.** Nothing here names a folder on that
//! computer, and the host answers nothing outside the workspace. It needs the control a grant
//! gives (`terminal:control`), which the host checks again: a computer let only watch reads none
//! of it.
//!
//! **Nothing is written, there or here.** What is read is shown and not kept.

use std::sync::Arc;

use remote_client::channel::WholeFile;
use remote_proto::{
    FileList, FileOf, FileReadParams, WorkspaceChanges, WorkspaceFilesParams, WorkspaceId,
};

use super::start::{no_such, open, refused};
use super::ComputerHome;
use crate::account::Failure;

/// What the host answers that can be read this way: one from before minor 13 cannot.
pub const LOOK: &str = "file.read";

/// What changed in `workspace` on `host`.
pub async fn changes<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
) -> Result<WorkspaceChanges, Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, LOOK)?;
    channel
        .changes(workspace)
        .await
        .map_err(|e| refused(&e, false))
}

/// One folder of `workspace`'s file tree on `host`. `dir` is inside the workspace; empty for
/// its root.
pub async fn files<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    dir: &str,
) -> Result<FileList, Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, LOOK)?;
    channel
        .files(WorkspaceFilesParams {
            workspace,
            dir: dir.to_owned(),
        })
        .await
        .map_err(|e| refused(&e, false))
}

/// A file of `workspace` on `host`, whole: on disk, or on one side of a change.
pub async fn file<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    path: &str,
    old_path: Option<&str>,
    of: FileOf,
) -> Result<WholeFile, Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, LOOK)?;
    channel
        .whole_file(FileReadParams {
            workspace,
            path: path.to_owned(),
            old_path: old_path.map(str::to_owned),
            of,
            offset: 0,
        })
        .await
        .map_err(|e| refused(&e, false))
}

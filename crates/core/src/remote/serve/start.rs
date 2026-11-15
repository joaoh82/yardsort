//! Starting agents and taking uploads on a device's channel: see [`crate::remote::start`].

use std::io::Write;
use std::path::PathBuf;

use base64::Engine;
use remote_proto::{
    clip, ErrorCode, Grant, HarnessList, Operation, ProjectCloneParams, ProjectCloned,
    ProjectGrant, ProjectRef, RemoteError, Reply, Request, Response, SessionResumeParams,
    SessionStartParams, ShellStartParams, Started, Upload, UploadBeginParams, UploadChunkParams,
    Uploaded, WorkspaceRemoveParams, WorkspaceRenameParams, WorkspaceStartParams, MAX_LABEL,
    MAX_TEXT, MAX_UPLOADS,
};

use super::{internal, lock, Context, Visitor};
use crate::remote::start::{new_upload, start_error, uploads_dir, NewWorkspace, Starting};
use crate::remote::HostFacade;

/// An upload under way: the file it is written to, and how much of it has come.
pub(super) struct Uploading {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
    written: u64,
}

impl Uploading {
    /// It will not be finished: what came of it goes.
    pub(super) fn abandon(self) {
        drop(self.file);
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(super) fn is_start(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::HarnessList
            | Operation::WorkspaceStart(_)
            | Operation::SessionStart(_)
            | Operation::UploadBegin(_)
            | Operation::UploadChunk(_)
            | Operation::SessionResume(_)
            | Operation::ProjectBranches(_)
            | Operation::ProjectClone(_)
            | Operation::ShellStart(_)
            | Operation::WorkspaceRename(_)
            | Operation::WorkspaceArchive(_)
            | Operation::WorkspaceDelete(_)
            | Operation::WorkspaceChanges(_)
            | Operation::WorkspaceFiles(_)
            | Operation::FileRead(_)
    )
}

impl Visitor {
    /// A start or an upload. `None` when the reply is sent later, from the start's own thread.
    pub(super) fn start(
        &self,
        context: &Context<'_>,
        facade: &HostFacade<'_>,
        grant: &Grant,
        request: &Request,
    ) -> Result<Option<Reply>, RemoteError> {
        let starting = context.starting.clone().ok_or_else(internal)?;
        match &request.operation {
            Operation::HarnessList => Ok(Some(Reply::Harnesses(HarnessList {
                harnesses: starting
                    .harnesses()
                    .into_iter()
                    .map(|mut h| {
                        h.id = clip(&h.id, MAX_LABEL);
                        h.name = clip(&h.name, MAX_TEXT);
                        h
                    })
                    .collect(),
            }))),
            Operation::WorkspaceStart(WorkspaceStartParams {
                project,
                prompt,
                harness,
                name,
                model,
                effort,
                base_branch,
                existing_branch,
            }) => {
                let project = facade.reachable_project(grant, project)?;
                let harness = chosen(starting.as_ref(), harness.as_deref())?;
                // An effort is one of the harness's own, never a word passed on to its program.
                if let Some(effort) = effort {
                    let offered = starting
                        .harnesses()
                        .into_iter()
                        .find(|h| h.id == harness)
                        .is_some_and(|h| h.efforts.contains(effort));
                    if !offered {
                        return Err(RemoteError::new(
                            ErrorCode::InvalidRequest,
                            "That agent has no such effort on this computer.",
                        ));
                    }
                }
                let new = NewWorkspace {
                    project: project.id,
                    prompt: prompt.clone(),
                    harness: harness.clone(),
                    name: name.clone(),
                    model: model.clone(),
                    effort: effort.clone(),
                    base_branch: base_branch.clone(),
                    existing_branch: existing_branch.clone(),
                };
                self.later(request, move || {
                    let (workspace, session) =
                        starting.workspace(&new).map_err(|e| start_error(&e))?;
                    started(&workspace, &session.id.0, harness)
                });
                Ok(None)
            }
            Operation::ProjectBranches(ProjectRef { project }) => {
                let project = facade.reachable_project(grant, project)?;
                self.later(request, move || {
                    starting
                        .branches(&project.id)
                        .map(Reply::Branches)
                        .map_err(|e| start_error(&e))
                });
                Ok(None)
            }
            Operation::ProjectClone(ProjectCloneParams { remote, name }) => {
                // The new project is none of those a narrower grant names, so that device
                // could not reach what it had cloned.
                if grant.projects != ProjectGrant::All {
                    return Err(RemoteError::new(
                        ErrorCode::Forbidden,
                        "This device is let into some of this computer's projects, not all, so \
                         it cannot add one.",
                    ));
                }
                let (remote, name) = (remote.trim().to_owned(), name.clone());
                self.later(request, move || {
                    let (project, name) = starting
                        .clone_project(&remote, name.as_deref())
                        .map_err(|e| start_error(&e))?;
                    Ok(Reply::Cloned(ProjectCloned {
                        project: remote_proto::ProjectId::new(&project).map_err(|_| internal())?,
                        name: clip(&name, MAX_TEXT),
                    }))
                });
                Ok(None)
            }
            Operation::ShellStart(ShellStartParams { workspace }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                self.later(request, move || {
                    let session = starting.shell(&row.id).map_err(|e| start_error(&e))?;
                    started(&row.id, &session.id.0, String::new())
                });
                Ok(None)
            }
            Operation::WorkspaceRename(WorkspaceRenameParams { workspace, name }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                starting
                    .rename(&row.id, name)
                    .map_err(|e| start_error(&e))?;
                Ok(Some(Reply::Done))
            }
            Operation::WorkspaceArchive(WorkspaceRemoveParams { workspace, force }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                let force = *force;
                self.later(request, move || {
                    starting
                        .archive(&row.id, force)
                        .map(|()| Reply::Done)
                        .map_err(|e| start_error(&e))
                });
                Ok(None)
            }
            Operation::WorkspaceDelete(WorkspaceRemoveParams { workspace, force }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                let force = *force;
                self.later(request, move || {
                    starting
                        .delete(&row.id, force)
                        .map(|()| Reply::Done)
                        .map_err(|e| start_error(&e))
                });
                Ok(None)
            }
            Operation::SessionStart(SessionStartParams {
                workspace,
                harness,
                prompt,
            }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                let harness = chosen(starting.as_ref(), harness.as_deref())?;
                let prompt = prompt.clone();
                self.later(request, move || {
                    let session = starting
                        .session(&row.id, &harness, prompt.as_deref())
                        .map_err(|e| start_error(&e))?;
                    started(&row.id, &session.id.0, harness)
                });
                Ok(None)
            }
            Operation::SessionResume(SessionResumeParams { session, prompt }) => {
                let (row, workspace) = facade.reachable_session(grant, session)?;
                let prompt = prompt.clone();
                self.later(request, move || {
                    let session = starting
                        .resume(&row.id, prompt.as_deref())
                        .map_err(|e| start_error(&e))?;
                    started(&workspace.id, &session.id.0, row.harness_id)
                });
                Ok(None)
            }
            Operation::WorkspaceChanges(remote_proto::WorkspaceRef { workspace }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                self.later(request, move || {
                    let set = starting.changes(&row.id).map_err(|e| start_error(&e))?;
                    Ok(Reply::Changes(remote_proto::look::bounded_changes(
                        set.uncommitted.into_iter().map(changed).collect(),
                        set.committed.into_iter().map(changed).collect(),
                        set.base,
                    )))
                });
                Ok(None)
            }
            Operation::WorkspaceFiles(remote_proto::WorkspaceFilesParams { workspace, dir }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                let dir = dir.clone();
                self.later(request, move || {
                    let entries = starting.files(&row.id, &dir).map_err(|e| start_error(&e))?;
                    Ok(Reply::Files(remote_proto::look::bounded_files(
                        entries
                            .into_iter()
                            .map(|entry| remote_proto::FileEntry {
                                name: entry.name,
                                dir: entry.is_dir,
                            })
                            .collect(),
                    )))
                });
                Ok(None)
            }
            Operation::FileRead(read) => {
                let (row, _) = facade.reachable_workspace(grant, &read.workspace)?;
                let read = read.clone();
                self.later(request, move || {
                    let content = starting
                        .content(&row.id, &read.path, read.old_path.as_deref(), read.of)
                        .map_err(|e| start_error(&e))?;
                    piece(content, read.offset).map(Reply::File)
                });
                Ok(None)
            }
            Operation::UploadBegin(UploadBeginParams {
                workspace,
                name,
                size,
            }) => {
                let (row, _) = facade.reachable_workspace(grant, workspace)?;
                let mut uploads = lock(&self.uploads);
                if uploads.len() >= MAX_UPLOADS {
                    return Err(RemoteError::new(
                        ErrorCode::RateLimited,
                        "Too many uploads at once. Wait for one to finish.",
                    ));
                }
                let dir = uploads_dir(&starting.data_dir(), &row.id);
                let (path, file) = new_upload(&dir, name).map_err(|_| {
                    RemoteError::new(ErrorCode::Internal, "The computer could not keep the file.")
                })?;
                let upload = self
                    .next_upload
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                uploads.insert(
                    upload,
                    Uploading {
                        path,
                        file,
                        size: *size,
                        written: 0,
                    },
                );
                Ok(Some(Reply::Upload(Upload { upload })))
            }
            Operation::UploadChunk(chunk) => self.chunk(chunk).map(Some),
            _ => Err(internal()),
        }
    }

    fn chunk(&self, chunk: &UploadChunkParams) -> Result<Reply, RemoteError> {
        let mut uploads = lock(&self.uploads);
        let uploading = uploads
            .get_mut(&chunk.upload)
            .ok_or_else(RemoteError::not_found)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&chunk.data)
            .map_err(|_| {
                RemoteError::new(ErrorCode::InvalidRequest, "That piece is not base64.")
            })?;
        let refused = |message: &str| RemoteError::new(ErrorCode::InvalidRequest, message);
        let failure = if chunk.offset != uploading.written {
            Some(refused(
                "That piece came out of order. Upload the file again.",
            ))
        } else if uploading.written + bytes.len() as u64 > uploading.size {
            Some(refused("That is more than the file was said to be."))
        } else if uploading.file.write_all(&bytes).is_err() {
            Some(RemoteError::new(
                ErrorCode::Internal,
                "The computer could not keep the file.",
            ))
        } else {
            None
        };
        if let Some(error) = failure {
            // A file with a piece missing is no use to anyone.
            if let Some(upload) = uploads.remove(&chunk.upload) {
                upload.abandon();
            }
            return Err(error);
        }
        uploading.written += bytes.len() as u64;
        if uploading.written < uploading.size {
            return Ok(Reply::Done);
        }
        let done = uploads.remove(&chunk.upload).ok_or_else(internal)?;
        if done.file.sync_all().is_err() {
            done.abandon();
            return Err(RemoteError::new(
                ErrorCode::Internal,
                "The computer could not keep the file.",
            ));
        }
        Ok(Reply::Uploaded(Uploaded {
            upload: chunk.upload,
            path: done.path.to_string_lossy().into_owned(),
        }))
    }

    /// Answer `request` from a thread of its own, once `work` is done.
    fn later(
        &self,
        request: &Request,
        work: impl FnOnce() -> Result<Reply, RemoteError> + Send + 'static,
    ) {
        let out = std::sync::Arc::clone(&self.out);
        let id = request.request_id.clone();
        std::thread::spawn(move || {
            let response = match work() {
                Ok(reply) => Response {
                    request_id: Some(id),
                    outcome: remote_proto::Outcome::Result(reply),
                },
                Err(error) => Response::error(Some(id), error),
            };
            let bytes = remote_proto::encode_response(&response);
            out.text(String::from_utf8(bytes).expect("JSON is UTF-8"));
        });
    }
}

/// A changed file as a device is told it.
fn changed(file: crate::changes::FileChange) -> remote_proto::ChangedFile {
    use crate::changes::ChangeKind as Here;
    use remote_proto::ChangeKind as There;
    remote_proto::ChangedFile {
        path: file.path,
        old_path: file.old_path,
        kind: match file.kind {
            Here::Added => There::Added,
            Here::Modified => There::Modified,
            Here::Deleted => There::Deleted,
            Here::Renamed => There::Renamed,
            Here::Untracked => There::Untracked,
            Here::Conflicted => There::Conflicted,
        },
        additions: file.additions,
        deletions: file.deletions,
    }
}

/// The piece of a file's contents that starts at `offset`, or why none of it is sent. Only text
/// is sent: a picture and anything else that is not text is said to be binary.
fn piece(
    content: crate::changes::Content,
    offset: u64,
) -> Result<remote_proto::FileChunk, RemoteError> {
    use crate::changes::Content;
    use remote_proto::look::not_sent;
    use remote_proto::{FileChunk, FileState, MAX_FILE_CHUNK};
    let text = match content {
        Content::Text { text } => text,
        Content::Absent => return Ok(not_sent(FileState::Absent, 0)),
        Content::Image { .. } | Content::Binary => return Ok(not_sent(FileState::Binary, 0)),
        Content::NotEditable { .. } => return Ok(not_sent(FileState::NotShown, 0)),
        Content::TooLarge { bytes } => return Ok(not_sent(FileState::TooLarge, bytes.into())),
    };
    let bytes = text.as_bytes();
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    if start > bytes.len() {
        return Err(RemoteError::new(
            ErrorCode::InvalidRequest,
            "That is past the end of the file.",
        ));
    }
    let end = bytes.len().min(start + MAX_FILE_CHUNK);
    Ok(FileChunk {
        state: FileState::Text,
        size: bytes.len() as u64,
        offset,
        data: base64::engine::general_purpose::STANDARD.encode(&bytes[start..end]),
        done: end == bytes.len(),
    })
}

/// The harness a device asked for, if this host can start it, or the host's default.
fn chosen(starting: &dyn Starting, wanted: Option<&str>) -> Result<String, RemoteError> {
    let harnesses = starting.harnesses();
    let found = match wanted {
        Some(wanted) => harnesses.into_iter().find(|h| h.id == wanted),
        None => harnesses.into_iter().next(),
    };
    found.map(|h| h.id).ok_or_else(|| {
        RemoteError::new(
            ErrorCode::InvalidRequest,
            match wanted {
                Some(_) => "That agent is not one this computer can start.",
                None => "This computer has no agent it can start.",
            },
        )
    })
}

fn started(workspace: &str, terminal: &str, harness: String) -> Result<Reply, RemoteError> {
    Ok(Reply::Started(Started {
        workspace: remote_proto::WorkspaceId::new(workspace).map_err(|_| internal())?,
        terminal: remote_proto::TerminalId::new(terminal).map_err(|_| internal())?,
        harness,
    }))
}

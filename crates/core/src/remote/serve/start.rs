//! Starting agents and taking uploads on a device's channel: see [`crate::remote::start`].

use std::io::Write;
use std::path::PathBuf;

use base64::Engine;
use remote_proto::{
    clip, ErrorCode, Grant, HarnessList, Operation, RemoteError, Reply, Request, Response,
    SessionStartParams, Started, Upload, UploadBeginParams, UploadChunkParams, Uploaded,
    WorkspaceStartParams, MAX_LABEL, MAX_TEXT, MAX_UPLOADS,
};

use super::{internal, lock, Context, Visitor};
use crate::remote::start::{new_upload, start_error, uploads_dir, Starting};
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
            }) => {
                let project = facade.reachable_project(grant, project)?;
                let harness = chosen(starting.as_ref(), harness.as_deref())?;
                let prompt = prompt.clone();
                self.later(request, move || {
                    let (workspace, session) = starting
                        .workspace(&project.id, &prompt, &harness)
                        .map_err(|e| start_error(&e))?;
                    started(&workspace, &session.id.0, harness)
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

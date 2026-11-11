//! Accepting a notice from a computer, and handing it to the phones of that account.
//!
//! The computer names a category and an event key. The words are chosen here. A repeat of the
//! same key, from the same computer or another, is one notice. Preferences that refuse it are
//! remembered, so turning a category back on does not send what was skipped.

use remote_proto::account::DeviceKind;
use remote_proto::notify::{
    valid_event_key, NoticeCategory, NoticeReceipt, NoticeSubmission, NoticeView,
    NotificationPreferences, PushRegistration, ReachableReceipt, NOTICE_TTL_MS,
};
use remote_proto::{ErrorCode, RemoteError};
use sha2::{Digest, Sha256};

use crate::push::{PushMessage, PushOutcome};
use crate::records::{DeliveryEnd, IncomingNotice, Records};
use crate::service::{unreadable, Caller, ControlService};

/// The sentence a category shows. The same words for every account.
pub fn copy(category: NoticeCategory, kind: &str) -> Option<(&'static str, &'static str)> {
    Some(match (category, kind) {
        (NoticeCategory::Task, "progress") => (
            "A task moved forward",
            "Open it on this phone. The work stays on the computer.",
        ),
        (NoticeCategory::Task, "complete") => (
            "A task finished",
            "Open it on this phone. The work stays on the computer.",
        ),
        (NoticeCategory::Review, "attention") => (
            "A review needs attention",
            "Open it on this phone. The review stays on the computer.",
        ),
        (NoticeCategory::Failure, "failed") => (
            "Something failed",
            "Open it on this phone. The details stay on the computer.",
        ),
        (NoticeCategory::DesktopUpdate, "available") => (
            "A desktop update is available",
            "Installing it stays on the computer.",
        ),
        (NoticeCategory::Workflow, "question") => (
            "A workflow has a question",
            "Answering it stays on the computer.",
        ),
        (NoticeCategory::Workflow, "approval") => (
            "A workflow is waiting for approval",
            "Approving it stays on the computer.",
        ),
        _ => return None,
    })
}

pub fn wire_name(category: NoticeCategory) -> &'static str {
    match category {
        NoticeCategory::Task => "task",
        NoticeCategory::Review => "review",
        NoticeCategory::Failure => "failure",
        NoticeCategory::DesktopUpdate => "desktopUpdate",
        NoticeCategory::Workflow => "workflow",
    }
}

fn category_of(name: &str) -> Option<NoticeCategory> {
    Some(match name {
        "task" => NoticeCategory::Task,
        "review" => NoticeCategory::Review,
        "failure" => NoticeCategory::Failure,
        "desktopUpdate" => NoticeCategory::DesktopUpdate,
        "workflow" => NoticeCategory::Workflow,
        _ => return None,
    })
}

impl<R: Records> ControlService<R> {
    pub(crate) async fn submit_notice(
        &self,
        caller: &Caller,
        submission: NoticeSubmission,
        now: i64,
    ) -> Result<NoticeReceipt, RemoteError> {
        if caller.kind != DeviceKind::Host {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "A computer submits activity.",
            ));
        }
        if !submission.category.allows(&submission.kind)
            || !valid_event_key(submission.category, &submission.event_key)
        {
            return Err(RemoteError::new(
                ErrorCode::InvalidRequest,
                "That notice is not one Yardsort sends.",
            ));
        }
        let (title, body) = copy(submission.category, &submission.kind).ok_or_else(|| {
            RemoteError::new(
                ErrorCode::InvalidRequest,
                "That notice is not one Yardsort sends.",
            )
        })?;
        let preferences = self
            .records
            .notification_preferences(&caller.account)
            .await
            .map_err(unreadable)?;
        let id = uuid::Uuid::new_v4().to_string();
        let record = self
            .records
            .accept_notice(
                &caller.account,
                &IncomingNotice {
                    id,
                    event_key: submission.event_key,
                    category: wire_name(submission.category).to_owned(),
                    kind: submission.kind,
                    title: title.to_owned(),
                    body: body.to_owned(),
                    expires_at: now.saturating_add(NOTICE_TTL_MS),
                    suppressed: !preferences.allows(submission.category),
                },
                now,
            )
            .await
            .map_err(unreadable)?;
        if record.fresh && !record.suppressed {
            self.deliver(&caller.account, now).await;
        }
        Ok(NoticeReceipt { id: record.id })
    }

    pub(crate) async fn save_push_token(
        &self,
        caller: &Caller,
        registration: PushRegistration,
        now: i64,
    ) -> Result<(), RemoteError> {
        if caller.kind != DeviceKind::Phone {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "A phone registers its push token.",
            ));
        }
        if !registration.token_ok() {
            return Err(RemoteError::new(
                ErrorCode::InvalidRequest,
                "That push token cannot be used.",
            ));
        }
        let platform = match registration.platform {
            remote_proto::notify::PushPlatform::Apns => "apns",
            remote_proto::notify::PushPlatform::Fcm => "fcm",
        };
        let saved = self
            .records
            .save_push_token(
                &caller.account,
                &caller.device,
                platform,
                &registration.token,
                now,
            )
            .await
            .map_err(unreadable)?;
        if !saved {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "A phone registers its push token.",
            ));
        }
        self.deliver(&caller.account, now).await;
        Ok(())
    }

    pub(crate) async fn notification_preferences(
        &self,
        caller: &Caller,
    ) -> Result<NotificationPreferences, RemoteError> {
        self.records
            .notification_preferences(&caller.account)
            .await
            .map_err(unreadable)
    }

    pub(crate) async fn set_notification_preferences(
        &self,
        caller: &Caller,
        preferences: NotificationPreferences,
    ) -> Result<(), RemoteError> {
        self.records
            .set_notification_preferences(&caller.account, &preferences)
            .await
            .map_err(unreadable)
    }

    pub(crate) async fn mark_reachable(
        &self,
        caller: &Caller,
        now: i64,
    ) -> Result<ReachableReceipt, RemoteError> {
        if caller.kind != DeviceKind::Phone {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "A phone says it can be reached.",
            ));
        }
        Ok(ReachableReceipt {
            sent: self.deliver(&caller.account, now).await,
        })
    }

    pub(crate) async fn read_notice(
        &self,
        caller: &Caller,
        id: &str,
    ) -> Result<NoticeView, RemoteError> {
        let Some(page) = self
            .records
            .notice_for(&caller.account, id)
            .await
            .map_err(unreadable)?
        else {
            return Err(RemoteError::not_found());
        };
        let Some(category) = category_of(&page.category) else {
            return Err(RemoteError::not_found());
        };
        Ok(NoticeView {
            deep_link: format!("yardsort://notice/{}/{}", category.slug(), page.id),
            id: page.id,
            category,
            kind: page.kind,
            title: page.title,
            body: page.body,
        })
    }

    async fn deliver(&self, account: &crate::records::AccountId, now: i64) -> u32 {
        let ready = match self.records.prepare_flush(account, now).await {
            Ok(ready) => ready,
            Err(error) => {
                tracing::error!("notices: {error}");
                return 0;
            }
        };
        let mut sent = 0u32;
        for item in ready {
            let claimed = self
                .records
                .claim_delivery(&item.notice_id, &item.device)
                .await
                .unwrap_or(false);
            if !claimed {
                continue;
            }
            let category = category_of(&item.category)
                .map(|c| c.slug())
                .unwrap_or("task");
            let outcome = self
                .push
                .send(PushMessage {
                    platform: match item.platform.as_str() {
                        "fcm" => remote_proto::notify::PushPlatform::Fcm,
                        _ => remote_proto::notify::PushPlatform::Apns,
                    },
                    token: item.token,
                    collapse: collapse(&account.0, &item.event_key),
                    title: item.title,
                    body: item.body,
                    deep_link: format!("yardsort://notice/{category}/{}", item.notice_id),
                    category: category.to_owned(),
                })
                .await;
            let end = match outcome {
                PushOutcome::Sent => {
                    sent += 1;
                    DeliveryEnd::Sent
                }
                PushOutcome::Unreachable => DeliveryEnd::Again,
                PushOutcome::Gone => DeliveryEnd::Gone,
            };
            if let Err(error) = self
                .records
                .finish_delivery(&item.notice_id, &item.device, end)
                .await
            {
                tracing::error!("notices: {error}");
            }
        }
        sent
    }
}

/// A collapse id of 64 hex characters. It does not reveal the event key.
fn collapse(account: &str, event_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account.as_bytes());
    hasher.update([0]);
    hasher.update(event_key.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

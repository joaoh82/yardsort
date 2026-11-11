//! A notice the phone may show. The host names a category and a stable event key. The server
//! chooses the words, and the phone opens the item. Answering, approving and installing stay
//! on the computer, so this message has no code and no action.

use serde::{Deserialize, Serialize};

/// How long a notice may wait to be delivered.
pub const NOTICE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// How many queued notices one phone is handed when it can be reached again.
pub const FLUSH_CAP: usize = 8;

/// What the notice is about. The wire name is camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeCategory {
    Task,
    Review,
    Failure,
    DesktopUpdate,
    Workflow,
}

impl NoticeCategory {
    /// The path segment of its deep link.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Review => "review",
            Self::Failure => "failure",
            Self::DesktopUpdate => "desktop-update",
            Self::Workflow => "workflow",
        }
    }

    /// The event key prefix this category uses.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Task => "task:",
            Self::Review => "review:",
            Self::Failure => "failure:",
            Self::DesktopUpdate => "desktop-update:",
            Self::Workflow => "workflow:",
        }
    }

    /// Whether `kind` is one this category sends.
    pub fn allows(self, kind: &str) -> bool {
        matches!(
            (self, kind),
            (Self::Task, "progress" | "complete")
                | (Self::Review, "attention")
                | (Self::Failure, "failed")
                | (Self::DesktopUpdate, "available")
                | (Self::Workflow, "question" | "approval")
        )
    }
}

/// What a host submits. The server ignores anything else in the JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeSubmission {
    pub event_key: String,
    pub category: NoticeCategory,
    pub kind: String,
}

/// Whether `key` may name an event of `category`. Upper case is refused, so a one-time code
/// cannot be hidden in the key.
pub fn valid_event_key(category: NoticeCategory, key: &str) -> bool {
    let Some(rest) = key.strip_prefix(category.prefix()) else {
        return false;
    };
    !rest.is_empty()
        && key.len() <= 200
        && key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b':' | b'.' | b'_' | b'-')
        })
}

/// Which push service a phone's token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushPlatform {
    Apns,
    Fcm,
}

/// A phone registering the token the operating system gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushRegistration {
    pub platform: PushPlatform,
    pub token: String,
}

impl PushRegistration {
    /// A token the server can keep and hand to a provider. Empty, huge, or with spaces is refused.
    pub fn token_ok(&self) -> bool {
        let token = self.token.as_str();
        (1..=4096).contains(&token.len()) && token.bytes().all(|b| (0x21..=0x7e).contains(&b))
    }
}

/// What reaches the phone, per category, and a mute that covers all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationPreferences {
    pub muted: bool,
    pub task: bool,
    pub review: bool,
    pub failure: bool,
    pub desktop_update: bool,
    pub workflow: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            muted: false,
            task: true,
            review: true,
            failure: true,
            desktop_update: true,
            workflow: true,
        }
    }
}

impl NotificationPreferences {
    /// Whether a notice of `category` is delivered.
    pub fn allows(&self, category: NoticeCategory) -> bool {
        if self.muted {
            return false;
        }
        match category {
            NoticeCategory::Task => self.task,
            NoticeCategory::Review => self.review,
            NoticeCategory::Failure => self.failure,
            NoticeCategory::DesktopUpdate => self.desktop_update,
            NoticeCategory::Workflow => self.workflow,
        }
    }
}

/// The server's answer when a host submits a notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeReceipt {
    pub id: String,
}

/// What the phone reads for one notice. The event key stays on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeView {
    pub id: String,
    pub category: NoticeCategory,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub deep_link: String,
}

/// How many queued notices were handed to a push provider on this call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReachableReceipt {
    pub sent: u32,
}

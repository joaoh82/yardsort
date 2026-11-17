//! Other computers' Notifications lists, as this one last read them, and saying there that a
//! row was seen. See docs/design/46-notifications.md.
//!
//! **Read, not kept.** A host's list is read with everything else it shows
//! ([`super::Shown::notifications`]) and again as soon as the host says it changed. Nothing of
//! it is written here: a host that is away shows no rows, and has them again when it is back.
//!
//! **Seen is said there.** The host keeps what was seen, for its own window and for every
//! device. Here the rows are shown as seen at once, and the host's next word puts right
//! anything it did not take.
//!
//! **Nothing here answers what waits.** A row that needs an answer is answered where its
//! question is: in the agent's terminal there, or at that computer.

use std::sync::Arc;

use remote_proto::notification::{NotificationPart, NotificationSeenParams, NotificationSummary};
use remote_proto::pairing::GrantState;
use remote_proto::HostDescription;

use super::start::{open, refused};
use super::{lock, ComputerHome, Link};
use crate::account::{now_ms, Failure};

/// What a host that gives its list answers: one from before minor 15, or with nothing keeping
/// its list, does not.
pub const LIST: &str = "notification.list";
/// What a host that says when its list changes answers.
pub const FOLLOW: &str = "notification.follow";

/// Whether `host` gives its Notifications list.
pub fn lists(host: &HostDescription) -> bool {
    host.capabilities.iter().any(|op| op == LIST)
}

/// Whether `host` says when its list changes.
pub fn follows(host: &HostDescription) -> bool {
    lists(host) && host.capabilities.iter().any(|op| op == FOLLOW)
}

/// One row of another computer's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct There {
    pub host_id: String,
    pub host_name: String,
    pub row: NotificationSummary,
}

/// Every row of every host this computer is let in to and can reach now, as each was last read.
pub fn listed<H: ComputerHome>(home: &Arc<H>) -> Vec<There> {
    let known = lock(home.computer());
    let mut rows = Vec::new();
    for grant in &known.grants {
        if grant.state != GrantState::Approved {
            continue;
        }
        let Some(link) = known.links.get(&grant.host) else {
            continue;
        };
        let view = link.view.lock().expect("host view");
        if view.link != Link::Online {
            continue;
        }
        let Some(page) = view.shown.as_ref().and_then(|s| s.notifications.as_ref()) else {
            continue;
        };
        rows.extend(page.notifications.iter().map(|row| There {
            host_id: grant.host.to_string(),
            host_name: grant.host_name.clone(),
            row: row.clone(),
        }));
    }
    rows
}

/// These finished rows of `host`'s list were looked at here: said there, and shown so here at
/// once.
pub async fn seen<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    ids: Vec<String>,
) -> Result<(), Failure> {
    if ids.is_empty() {
        return Ok(());
    }
    let channel = open(home.as_ref(), host, LIST)?;
    channel
        .notifications_seen(NotificationSeenParams {
            ids: ids.clone(),
            all: false,
        })
        .await
        .map_err(|e| refused(&e, false))?;
    settle(home.as_ref(), host, |row| ids.contains(&row.id));
    Ok(())
}

/// Everything finished is seen, on every host that can be told so now. One that cannot keeps
/// its rows, and the failure of one does not stop the rest: the first is what is said.
pub async fn seen_all<H: ComputerHome>(home: &Arc<H>) -> Result<(), Failure> {
    let hosts: Vec<String> = {
        let mut hosts: Vec<String> = listed(home)
            .into_iter()
            .filter(|there| there.row.part == NotificationPart::Finished)
            .map(|there| there.host_id)
            .collect();
        hosts.dedup();
        hosts
    };
    let mut failed = None;
    for host in hosts {
        let told = async {
            let channel = open(home.as_ref(), &host, LIST)?;
            channel
                .notifications_seen(NotificationSeenParams {
                    ids: Vec::new(),
                    all: true,
                })
                .await
                .map_err(|e| refused(&e, false))
        };
        match told.await {
            Ok(()) => settle(home.as_ref(), &host, |_| true),
            Err(failure) => failed = failed.or(Some(failure)),
        }
    }
    failed.map_or(Ok(()), Err)
}

/// Show `host`'s finished rows that `gone` picks as seen, in what was last read of it.
fn settle(home: &impl ComputerHome, host: &str, gone: impl Fn(&NotificationSummary) -> bool) {
    let changed = {
        let known = lock(home.computer());
        let Some(link) = known.links.iter().find(|(id, _)| id.as_str() == host) else {
            return;
        };
        let mut view = link.1.view.lock().expect("host view");
        let Some(page) = view
            .shown
            .as_mut()
            .and_then(|shown| shown.notifications.as_mut())
        else {
            return;
        };
        let now = now_ms();
        let mut left = 0u32;
        for row in &mut page.notifications {
            if row.part == NotificationPart::Finished && gone(row) {
                row.part = NotificationPart::Earlier;
                row.settled_at = Some(now);
                left += 1;
            }
        }
        page.finished = page.finished.saturating_sub(left);
        left > 0
    };
    if changed {
        home.notifications_changed();
    }
}

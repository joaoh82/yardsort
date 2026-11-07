//! The presets: triggers Yardsort puts together, each an event and a built-in workflow, with at
//! most one question. Their filters are the defaults — others' posts, drafts left out. A trigger
//! made from one is an ordinary trigger that remembers which; nothing about it is special after
//! it is saved.

use super::TriggerEvent;

/// A trigger, ready made.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// What `ys trigger add --preset` takes.
    pub id: &'static str,
    /// What the trigger is called.
    pub name: &'static str,
    /// What it does, in words.
    pub does: &'static str,
    pub events: &'static [TriggerEvent],
    /// Always a workflow, built in.
    pub workflow: &'static str,
    /// The input it asks for, by id, and the question: who reviews, which test.
    pub asks: Option<(&'static str, &'static str)>,
}

impl Preset {
    /// The quiet time it gets: its events' own.
    pub fn quiet_ms(&self) -> i64 {
        self.events
            .iter()
            .map(|event| event.default_quiet_ms())
            .max()
            .unwrap_or(0)
    }
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "review-when-done",
        name: "Review when the work is done",
        does: "When the agent says its work is done, a second agent reviews the branch and the \
               first is told what it found.",
        events: &[TriggerEvent::WorkspaceDone],
        workflow: "review-branch",
        asks: Some(("reviewer", "Who reviews")),
    },
    Preset {
        id: "review-when-opened",
        name: "Review the pull request when it opens",
        does: "When the workspace's pull request opens, or a draft is marked ready, a second \
               agent reviews it on GitHub.",
        events: &[TriggerEvent::PrOpened],
        workflow: "code-review",
        asks: Some(("reviewer", "Who reviews")),
    },
    Preset {
        id: "address-reviews",
        name: "Address reviews",
        does: "When someone other than you reviews or comments on the workspace's pull request, \
               two minutes after the last one, the agent is told to address it.",
        events: &[TriggerEvent::PrReview, TriggerEvent::PrComment],
        workflow: "address-review",
        asks: None,
    },
    Preset {
        id: "fix-checks",
        name: "Fix failing checks",
        does: "When the checks on the pull request's newest commit fail, the agent is told which, \
               and to fix them.",
        events: &[TriggerEvent::PrChecksFailed],
        workflow: "fix-checks",
        asks: None,
    },
    Preset {
        id: "smoke-when-done",
        name: "Smoke test when the work is done",
        does: "When the agent says its work is done, a smoke test runs and the agent is told \
               how it went.",
        events: &[TriggerEvent::WorkspaceDone],
        workflow: "smoke-then-tell",
        asks: Some(("suite", "Which smoke test")),
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

//! A smoke test run waiting for its person, and their answer. Since minor 10.
//!
//! A test file from a repository runs only once a person has seen what it does: on the host that
//! is the approval card (`docs/design/32-smoke-tests.md`). `test.waiting` gives a device the same
//! card for a workspace's oldest waiting run, and `test.decide` its answer.
//!
//! What the card says is a file of the repository put in words: the commands it would run, as
//! written, and the *names* of the project's test environment, never a value. That is what a
//! terminal in the workspace would show, so reading it needs `terminal:read`. Letting a run go
//! is running programs as the host's user, which is what typing into a terminal does, so
//! deciding needs `terminal:control`.
//!
//! A run is let go only on what was shown. A card that could not be sent whole says so
//! ([`WaitingTest::whole`]), and the host then takes a refusal for it and nothing else.

use serde::{Deserialize, Serialize};

use crate::ids::{SessionRecordId, WorkspaceId};

/// The most steps a card lists. A file with more is not [`WaitingTest::whole`].
pub const MAX_STEPS: usize = 60;
/// Longest sentence for one step, in characters. A longer one is cut, and the card is not whole.
pub const MAX_STEP_WORDS: usize = 400;
/// The most names of the test environment a card lists, and the most inputs.
pub const MAX_NAMES: usize = 40;
/// Longest input value, in characters.
pub const MAX_INPUT: usize = 256;
/// Longest test file a card carries, in characters, for what changed since it was approved.
pub const MAX_DEFINITION: usize = 10_000;
/// Longest content hash, in bytes: a SHA-256 in hex.
pub const MAX_HASH: usize = 64;

/// `test.waiting`: the runs of one workspace waiting for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestWaitingParams {
    pub workspace: WorkspaceId,
}

/// Who asked for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestAskedBy {
    /// An agent, with `ys test run`.
    Agent,
    /// The person, in the app, or something they set up there: a schedule, a trigger.
    App,
    Workflow,
    /// A way of asking from a newer host.
    #[serde(other)]
    Other,
}

/// One step of a test, in a sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestStep {
    pub id: String,
    /// `setup`, `steps` or `teardown`.
    pub phase: String,
    pub words: String,
}

/// A name of the project's test environment a file reads, and whether it is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEnvUse {
    pub name: String,
    pub set: bool,
}

/// An input the run was asked for with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestInput {
    pub name: String,
    pub value: String,
}

/// A run waiting for a person, as the approval card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingTest {
    pub run: String,
    /// The test's name, as its file gives it.
    pub name: String,
    pub asked_by: TestAskedBy,
    /// The conversation whose agent asked, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRecordId>,
    pub asked_at: i64,
    pub inputs: Vec<TestInput>,
    /// Every step, in the order it runs. Empty when the file does not check out.
    pub steps: Vec<TestStep>,
    /// The names the file templates in.
    pub env: Vec<TestEnvUse>,
    /// A step runs a program, which is handed the project's whole test environment.
    pub runs_a_program: bool,
    /// Every name in the project's test environment: what such a step can read.
    pub environment: Vec<String>,
    /// This exact file was approved before, and the project asks to see every run.
    pub approved_before: bool,
    /// The file as it was when last approved, when that is another version of it and both fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The file the run would carry out, sent with `previous` to show what changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    /// The hash of that file: `test.decide` names it, so an answer is to what was shown.
    pub content_hash: String,
    /// Everything the card says is here, uncut. When false — a file too long, or one that does
    /// not check out — the run cannot be let go from this device, only refused.
    pub whole: bool,
    /// Why not, for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
}

/// The reply to `test.waiting`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestWaitingPage {
    /// The oldest run waiting, in full. One at a time, as on the host. Boxed: a card is far
    /// larger than any other reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<Box<WaitingTest>>,
    /// How many more wait behind it.
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestDecision {
    /// Approve the file as shown, and run it.
    Run,
    /// Do not run it. Nothing is recorded against the file.
    Refuse,
}

/// `test.decide`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestDecideParams {
    pub workspace: WorkspaceId,
    pub run: String,
    /// The hash the card showed.
    pub content_hash: String,
    pub decision: TestDecision,
}

impl TestDecideParams {
    pub fn invalid(&self) -> Option<&'static str> {
        if crate::ids::check_id(&self.run).is_err() {
            return Some("That run id is not one Yardsort uses.");
        }
        let hash_ok = !self.content_hash.is_empty()
            && self.content_hash.len() <= MAX_HASH
            && self.content_hash.bytes().all(|b| b.is_ascii_hexdigit());
        (!hash_ok).then_some("That is not a test file's hash.")
    }
}

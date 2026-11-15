// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The protocol's numbers, as `crates/remote-proto` defines them. Each is the Rust constant of the
 * same name; a change there is a change here.
 */

/** The major this client speaks. */
export const PROTOCOL_MAJOR = 1;
/** The minor this client speaks: 14, committing, pushing, and opening and merging a pull request. */
export const PROTOCOL_MINOR = 14;

/** The largest JSON message either side reads, in bytes. */
export const MAX_CONTROL_FRAME = 64 * 1024;
/** The largest chunk of terminal output in one frame, in bytes. */
export const MAX_OUTPUT_CHUNK = 64 * 1024;
/** Bytes before an output frame's payload. */
export const OUTPUT_HEADER_LEN = 20;
/** The most input one `terminal.input` carries, in bytes. */
export const MAX_INPUT_BYTES = 32 * 1024;
/** The most a page holds, whatever a request asks for. */
export const MAX_PAGE = 200;
/** A control lease lapses this long after it was last renewed. */
export const LEASE_TTL_MS = 30_000;
/** How often a lease holder renews. */
export const LEASE_RENEW_MS = 10_000;
/** Longest client name in a `Hello`, in bytes. */
export const MAX_CLIENT_NAME = 128;
/** The most activity subscriptions one connection holds at once. */
export const MAX_SUBSCRIPTIONS = 16;

/** Longest id, in bytes: ASCII letters, digits and `-_.:`. */
export const MAX_ID_LEN = 128;
/** Longest cursor, in bytes: printable ASCII. */
export const MAX_CURSOR_LEN = 256;
/** Longest name or title, in characters. */
export const MAX_TEXT = 256;
/** Longest one-time code a workflow decision may carry, in characters. */
export const MAX_CODE_LEN = 64;
/** Longest artifact body a workflow run detail includes, in characters. */
export const MAX_ARTIFACT_PREVIEW = 2_000;
/** Longest label (a harness, a model, a program, an author, a task's source), in characters. */
export const MAX_LABEL = 64;
/** Longest memory note, in characters. */
export const MAX_NOTE = 1000;
/** The most tasks one workspace lists. */
export const MAX_TASKS = 8;
/** Longest task link, in bytes. */
export const MAX_LINK = 512;
/** The most other terminals one sessions page lists. */
export const MAX_TERMINALS = 32;

/** Longest message `workspace.start`, `session.start` or `session.resume` carries, in bytes. */
export const MAX_PROMPT = 16 * 1024;
/** The largest file one upload carries, in bytes. */
export const MAX_UPLOAD = 20 * 1024 * 1024;
/** The most one `upload.chunk` carries, in bytes before base64. */
export const MAX_UPLOAD_CHUNK = 32 * 1024;
/** Longest file name an upload gives, in characters. */
export const MAX_UPLOAD_NAME = 128;

/** Longest one-time code, refresh credential or ticket, in bytes. */
export const MAX_SECRET_LEN = 256;
/** Longest redirect a device may name, in bytes. */
export const MAX_REDIRECT_LEN = 512;
/** A PKCE verifier's length bounds (RFC 7636 §4.1). */
export const MIN_VERIFIER_LEN = 43;
export const MAX_VERIFIER_LEN = 128;
/** Characters in a pairing code, not counting the dash it is shown with. */
export const CODE_LEN = 8;
/** How long an offer can be redeemed, and how long a request then waits for an answer. */
export const OFFER_TTL_MS = 5 * 60_000;
/** The most projects a grant names one by one. */
export const MAX_GRANT_PROJECTS = 200;

/** How long a relay ticket can be presented. */
export const TICKET_TTL_MS = 60_000;
/** The longest route: letters, digits, `-` and `_`. */
export const MAX_ROUTE_LEN = 64;

/** Longest single text of a conversation's turn, in characters. Minor 8. */
export const MAX_TURN_TEXT = 16_000;
/** The most of the agent's texts one turn carries. Minor 8. */
export const MAX_SAID = 24;
/** Longest command in an open question, in characters. Minor 8. */
export const MAX_COMMAND = 2_000;
/** The most open questions a conversation page lists. Minor 8. */
export const MAX_ASKS = 8;
/** Longest question id, in bytes. Minor 8. */
export const MAX_ASK_ID = 64;

/** The most steps a waiting smoke test's card lists. Minor 10. */
export const MAX_TEST_STEPS = 60;
/** Longest sentence for one step of a smoke test, in characters. Minor 10. */
export const MAX_STEP_WORDS = 400;
/** The most environment names, and the most inputs, a smoke test's card lists. Minor 10. */
export const MAX_TEST_NAMES = 40;
/** Longest input value of a smoke test's run, in characters. Minor 10. */
export const MAX_TEST_INPUT = 256;
/** Longest smoke test file a card carries, in characters. Minor 10. */
export const MAX_TEST_DEFINITION = 10_000;
/** Longest content hash of a smoke test file, in bytes. Minor 10. */
export const MAX_TEST_HASH = 64;

/** The most checks one pull request is described with, by name. Minor 11. */
export const MAX_PULL_CHECKS = 40;
/** Longest name of a pull request's check, in characters. Minor 11. */
export const MAX_CHECK_NAME = 100;
/** The most reviewers, and the most reviews waited for, a pull request is described with. Minor 11. */
export const MAX_REVIEWERS = 12;
/** The most workspaces one pull request names. Minor 11. */
export const MAX_PULL_WORKSPACES = 8;

/** The most models, and the most efforts, one harness lists. Minor 12. */
export const MAX_CHOICES = 32;
/** Longest name a start or a rename may give a workspace, in characters. Minor 12. */
export const MAX_WORKSPACE_NAME = 80;
/** Longest branch name a request or a reply carries, in bytes. Minor 12. */
export const MAX_BRANCH = 255;
/** The most branches one `project.branches` reply lists. Minor 12. */
export const MAX_BRANCHES = 100;
/** Longest repository address a request or a reply carries, in bytes. Minor 12. */
export const MAX_REMOTE = 512;
/** Longest name a clone may be given, in bytes. Minor 12. */
export const MAX_PROJECT_NAME = 100;

/** Longest path inside a workspace a request or a reply carries, in bytes. Minor 13. */
export const MAX_PATH = 1024;
/** The largest file a computer sends, in bytes: what its own window shows. Minor 13. */
export const MAX_FILE = 1024 * 1024;
/** The most one `file.read` reply carries, in bytes before base64. Minor 13. */
export const MAX_FILE_CHUNK = 32 * 1024;
/**
 * How much of a list of paths one reply carries, in bytes of path. A list cut short says so with
 * `more`. Minor 13.
 */
export const MAX_LISTED_BYTES = 40 * 1024;

/** Longest commit message a request carries, in bytes. Minor 14. */
export const MAX_COMMIT_MESSAGE = 16 * 1024;
/** Longest pull request title, in characters. Minor 14. */
export const MAX_PULL_REQUEST_TITLE = 256;
/** Longest pull request body a request carries, in bytes. Minor 14. */
export const MAX_PULL_REQUEST_BODY = 32 * 1024;
/** The most unpushed commits a reply names. Minor 14. */
export const MAX_UNPUSHED = 20;
/** Longest commit id a request or a reply carries, in characters. Minor 14. */
export const MAX_OID = 64;
/** Longest link in a reply about a pull request, in bytes. Minor 14. */
export const MAX_URL = 1024;

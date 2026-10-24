# 26 — Remote access: fit report and the first contract slice

Status: **R0 and R1 done, not wired to anything** · 21–22 October 2026 · Companion to
[25, the remote, mobile and sync proposal](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md). It
records the repository as found when the work started, where the proposal's premises needed
adjusting, the decisions taken, what this slice built, and the ledger later phases update.

**Nothing a user can see changed.** There is no listener, no setting, no account and no mobile app.
What exists is a versioned protocol crate and a read-only facade a future authenticated transport
will put in front of the host. Remote terminal viewing and control are **not** available. They are
specified, and every host refuses them.

## 1 · Current state at the start of the work

| Fact                  | Value                                                                                                                                                                                                                                                                                                       |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Branch / HEAD         | `ys/implement-remote-workspace` at `eec6446` ("Design: proposal for remote access, a mobile companion and sync"), clean tree                                                                                                                                                                                |
| Relation to `main`    | `origin/main` fetched on 2026-10-21; identical to HEAD (0 ahead, 0 behind)                                                                                                                                                                                                                                  |
| Open pull requests    | None (`gh pr list --state open` was empty). Remote branches with unmerged work: `ys/blog-and-tutorials`, `ys/ci-timeouts`, `ys/some-case-session-workspace`, `ys/website-answer-content`, `docs/branch-ruleset`, `changelog-0.15.0`. None touches the daemon, the store or remote access                    |
| Since the snapshot    | The proposal was written against `74df92c` (v0.20.0). `74df92c..HEAD` is 8 commits, 23 files, +823/−88: v0.20.1, v0.20.2 (AppImage GPU and relaunch fixes), a test-stub fix (#118) and the proposal itself. None of it moves a boundary the proposal relies on                                              |
| License               | GPL-3.0-only (`LICENSE`, `Cargo.toml` `[workspace.package]`, `package.json`). The new crate inherits it. No new third-party dependency beyond `serde`, `serde_json` and `thiserror`, all already in the tree                                                                                                |
| Remote/auth/sync code | **None.** No HTTP or WebSocket server, relay, pairing, account or sync code anywhere. `reqwest` exists only in `src-tauri`, outbound (Assist, drafting). The only `TcpListener` is in `src-tauri/src/assist/jev.rs` tests. `#[cfg_attr(mobile, …)]` in `src-tauri/src/lib.rs` is Tauri template boilerplate |
| Migrations            | `0001`–`0015`. This slice adds none                                                                                                                                                                                                                                                                         |

## 2 · How the paths that matter actually work

All references are to HEAD `eec6446`.

**Launch.** The app and `ys` share `Launcher::in_workspace` (`crates/core/src/launch.rs:148`).
It writes the `agent_runs` row before the spawn (`start_recorded`, `:243`) and the `sessions` row
after it, and labels the PTY `workspace`, `harness`, `harnessSession`, `record` and `run`
(`:61–69`). `Launcher::start` (`:348`) appends only ids to the user's resolved login-shell
environment: `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID` and `YARDSORT_SESSION_RECORD_ID`. The
child otherwise gets the **whole** user environment, including any tokens the user exported
(`launch_plan`, `:532`). **Resume and fork are app-only**: `src-tauri/src/sessions.rs::continue_session`
(`:126`). `ys` has no resume. A headless host (R5) needs that moved into the core first.

**Daemon lifetime.** The daemon is the app binary re-run with `--yardsort-daemon <socket> [spool]`
(`crates/core/src/daemon.rs:35`). There is one per data directory, started under `daemon.lock`
(`connect_or_start`, `:223`). It exits once nobody is connected and nothing is running, after
`IDLE_GRACE` = 10 s (`crates/pty-ipc/src/server.rs:27`, `watch_for_idleness` `:163`).
`DaemonClient` is itself a `TerminalHost`. Clients are unbounded, one thread each. Access control
is filesystem permissions only: a `0600` socket, or a named pipe's default descriptor. The handshake
carries no credential, and a protocol mismatch is only logged (`handle`, `Op::Hello`). The app
decides whether to replace the daemon (`daemon.rs:126–153`). `pty_ipc::PROTOCOL` = 1 (`proto.rs:17`).

**Snapshot and ordering.** `Session::attach` (`crates/pty-host/src/session.rs:179`) renders the
snapshot, hands it to the sink and registers the sink under the same `view` lock that
`deliver` (`:380`) holds while fanning a batch out. **The boundary is atomic** for every viewer.
Over the socket, the client picks the stream id and registers its sink before asking, and one
FIFO writer per connection keeps the order. **There are no sequence numbers, offsets or epochs
anywhere.** A reattach is always a full snapshot. (R1's third slice added numbering inside
`pty-host`; see §6. The daemon's protocol still carries none.)

**Terminal queries.** The host answers the cursor-position query (`ESC [ 6 n`) itself **only while
no viewer is attached**. It decides this under the view lock (`deliver`), so a session with zero
or one viewer gets exactly one reply. **With two viewers attached** (the app and `ys attach`, which
the code allows), **both answer**, because no single-responder election exists. ConPTY's startup
query is why the unattended reply exists. The proposal's "existing handling of terminal queries"
holds for one viewer, not for many.

**Input and resize.** Any client may `write`, `paste` or `resize`, and the last resize wins. There is
no controller, lease or generation. `ys attach` polls its own size every 200 ms and resizes
(`crates/cli/src/commands/attach.rs`). The webview resizes on every fit.

**Slow clients.** Each daemon connection has a queue capped at 32 MiB
(`server.rs:33`, `Conn::send`). Past the cap the connection is marked gone and its sinks
detach. The socket is not closed, so later responses on it are dropped and that client's requests
time out after 30 s rather than seeing a clean disconnect. A remote path must not inherit this:
it needs a bounded queue that detaches with `resnapshot_required`.

**Events.** SQLite is canonical: `agent_runs`, `agent_events` (unique on `producer, source_key`)
and `sessions`. Four feeds write them through `crates/core/src/activity.rs`:

- the launcher's own lifecycle events;
- the daemon's exit spool, `<data>/activity/spool`, imported and deleted by the next client;
- the hook inbox, `<data>/activity/inbox`;
- harness session files read in place.

Spool and inbox are transient delivery, not history. NDJSON exists only as `ys activity export`.
`EventRow.seq` (`crates/core/src/store.rs:251`) is a monotonic integer, which is the natural
`activity.subscribe` cursor.

**Memory.** Agents propose through `ys memory propose`, always as a `candidate`
(`crates/core/src/memory.rs::propose`, `:110`). Only `memory::decide` (`:145`) approves, and only
Tauri commands call it (`src-tauri/src/memory.rs`). The CLI has no approve command; that is the
enforcement, structural rather than an environment check. Launch context gets up to 40 approved
entries (`launch.rs:163–182`), skippable per launch (`skip_memory`). **Sharing with agents is on by
default in the code** (`Store::memory_shared`, `store.rs:1338`: no row means shared). The comment
in the shipped migration `0010_project_memory.sql` says the opposite and is stale; it cannot be
edited, and the code is what runs. `ys memory list` refuses when sharing is off.

**Credentials.** The `keyring` crate is used only in `src-tauri` (`src-tauri/src/assist/key.rs`,
service `dev.yardsort.app`), for the TypeSafe and Anthropic keys. The webview can set or forget a
key but never read it. Forge access is the user's own `gh` login. Yardsort stores no forge token.
**The core has no credential abstraction**, which a headless host (R5) will need.

**UI transport.** Only `src/lib/ipc.ts` calls the core. The bindings come from tauri-specta
(`src-tauri/src/lib.rs::ipc_builder`). Terminal output reaches the webview as raw bytes over a
Tauri `Channel` (`src-tauri/src/terminal.rs::pty_attach`). Busy and quiet are `HostEvent`s derived
from output **timing** (`pty-host` pump, 3 s quiet). `pty_write` and `pty_resize` take only a
session id: that is the trusted local boundary and must stay local.

**One owner per profile.** `crates/core/src/presence.rs` is the app's exclusive `app.lock`, which
`ys workflow` already uses to ask "is the app running?". It is the obvious lock for "one host
runtime per data directory" (proposal §4), and nothing new is needed for it.

## 3 · Where the proposal needs adjusting

| Proposal said                                                             | Found                                                              | Decision                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Baseline `74df92c`, v0.20.0                                               | HEAD `eec6446`, v0.20.2 + #118                                     | No boundary moved. Recorded above.                                                                                                                                                                                                                                                                                                                                         |
| "Existing handling of terminal queries"                                   | One responder only with ≤ 1 viewer; N viewers answer N times today | Single responder is **R2 work, not a property to preserve**. The controller's terminal answers, or the host's own when nobody controls; every other viewer's renderer is told not to. This is a release gate, especially on Windows.                                                                                                                                       |
| Snapshot then live output "at a specific output sequence"                 | Atomic, but with no sequence numbers                               | Add a per-session chunk counter in `pty-host`'s `deliver`, under the view lock that already makes attach atomic. The epoch is the daemon session's lifetime. Contract fixed now in `remote_proto::terminal`.                                                                                                                                                               |
| One controller across desktop and remote                                  | Every local client may write and resize                            | **The lease must live in the daemon** (`pty-ipc` + `pty-host`), not in a remote layer, or the app and `ys attach` bypass it. That is a daemon protocol change (`PROTOCOL` 2). The existing upgrade rule applies: a busy old daemon is left alone, and remote terminals stay unavailable until it is idle-replaced. Local clients become the `Local` controller implicitly. |
| Slow clients detached with a resnapshot error                             | Daemon marks a slow client gone but keeps the socket               | The remote path gets its own bounded queue (`MAX_QUEUED_OUTPUT` = 1 MiB) and a clean `resnapshot_required`. Fixing the daemon's half-dead socket is a separate small change.                                                                                                                                                                                               |
| "Separate server, user, device, host, logical project/workspace identity" | Local ids are UUIDs; no host identity exists                       | Protocol ids are **host-scoped** and equal the local UUIDs. One newtype per kind (`remote_proto::ids`) keeps them apart at compile time. Logical sync ids (R6) will be a mapping table beside the local ids, never a replacement. No host id is stored before Remote is enabled (R2).                                                                                      |
| Memory read respects "existing project access policy"                     | The share switch governs what reaches **agents**                   | `memory.list` returns approved entries under `memory:read`, regardless of the share switch, as the desktop Memory view does. The proposal's §9 rule ("do not infer network permission from local memory-sharing settings") points the same way. No remote memory write exists.                                                                                             |
| `packages/remote-contracts` TS DTOs                                       | No mobile code yet                                                 | Deferred to R3. The language-neutral contract today is the hand-written JSON corpus in `crates/remote-proto/fixtures/v1/`, which the Rust tests pin; the TS client will be tested against the same files.                                                                                                                                                                  |
| Vision: "Remote / SSH / cloud workspaces" and "accounts" out of scope     | Owner decision in the proposal reverses this                       | [01-vision](01-vision.md) now points here. Local operation stays account-free.                                                                                                                                                                                                                                                                                             |

The proposal's corrections in its own §2 all held when checked:

- SQLite is canonical, and there is no second event log.
- There is no MCP memory delivery.
- The license is GPL-3.0-only.
- The daemon is not a remote server.

## 4 · Ownership

| Owner                       | Owns                                                                    | Notes                                                                                                   |
| --------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `crates/pty-host`           | Processes, PTY I/O, headless VT, snapshots, the attach boundary         | Numbers output and resizes per session (R1). Gains lease arbitration in R2. Still no Tauri, no network. |
| `crates/pty-ipc`            | The local wire, one daemon per profile                                  | Stays local-only, behind file permissions. **Never exposed to a network.**                              |
| `crates/remote-proto` (new) | The remote protocol's types, limits, versioning, framing                | No transport, no store, no I/O.                                                                         |
| `crates/core::remote` (new) | Answering remote reads for one host, authorizing each against a `Grant` | Read-only. Never writes, attaches, resizes or spawns.                                                   |
| Host runtime (R2)           | Holding `app.lock`; enrollment and grants; the outbound connector       | Desktop app first, a headless entry point in R5.                                                        |
| Server (R2+)                | Accounts, devices, relay, tickets; later sync                           | Separate crate. Depends on `remote-proto`, not on the core.                                             |
| Desktop app, `ys`           | Local UI and CLI, unchanged                                             | The only routes that approve memory, launch, delete.                                                    |

## 5 · Decisions

| Question                 | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Where the contract lives | `crates/remote-proto`: `serde`, `serde_json` and `thiserror` only, so a server and a mobile conformance test can depend on it without SQLite, git or a PTY.                                                                                                                                                                                                                                                                                                                                                                      |
| Versioning               | `PROTOCOL_MAJOR` 1 / `PROTOCOL_MINOR` 0, independent of the app, of `pty_ipc::PROTOCOL` and of any sync schema. `Hello` offers majors; another major gets `protocol_incompatible` with `supportedProtocols`. Unknown fields are ignored. An unknown `op` is `unsupported`, not malformed.                                                                                                                                                                                                                                        |
| Envelope                 | `{requestId, protocol, op, params}` in; `{requestId, result: {kind, …}}` or `{requestId, error: {code, message, retryable}}` out.                                                                                                                                                                                                                                                                                                                                                                                                |
| Errors                   | Stable snake_case codes, one per row of the proposal's list, plus `not_found`, `lease_lost`, `resnapshot_required`, `unsupported`, `internal`, and `unknown` for a newer peer's code. Only `host_offline`, `rate_limited` and `internal` are retryable. Messages never echo the request and never carry store errors, which can name files.                                                                                                                                                                                      |
| Untrusted input          | Size is checked before parsing (64 KiB). Every id is ≤ 128 bytes of `[A-Za-z0-9-_.:]`, so no path or control character gets through. A cursor is ≤ 256 printable bytes. Page sizes are clamped to 200. Input is ≤ 32 KiB. Nesting is bounded by serde_json's recursion limit (tested).                                                                                                                                                                                                                                           |
| Response size            | Every response fits the same 64 KiB a request does. A page holds fewer items than asked when they are large, and its cursor goes on from the last one, so nothing is skipped. Text is clipped to protocol limits, ending in `…`: names and titles 256 characters, labels 64, memory notes 1000. A workspace lists at most 8 tasks, leaving out one whose link is over 512 bytes, and a sessions page at most 32 other terminals. Together these keep one item at its largest within a page (tested), so a list can always go on. |
| Authorization shape      | `Grant { device, scopes, projects: All \| Only(set) }`, using the proposal's six scopes. Checked in order: protocol, bounds, scope (`forbidden`), project reach (`not_found`, identical to a missing id). Nothing in this slice mints a grant.                                                                                                                                                                                                                                                                                   |
| What may cross           | No paths, argv, environment, labels, harness-native session ids, first messages or terminal output. The session `title` crosses: the desktop shows it, and it may quote the first message, so clients must treat it as user content and never log it.                                                                                                                                                                                                                                                                            |
| Session state            | `process`: `running`, `ended`, `gone` or `unknown` (no daemon). `terminal.activity`: `starting`, `busy`, `quiet` or `exited`, from timing only. `quiet` is never presented as "needs your input". A daemon session is matched to a record only when its `workspace` label agrees.                                                                                                                                                                                                                                                |
| Pagination               | Order is the app's own. The cursor is `k1.<last id>`. If that item has gone, the answer is `cursor_expired` rather than a silently skipped or repeated item. Activity has its own cursors (below).                                                                                                                                                                                                                                                                                                                               |
| Activity                 | `activity.list` under `activity:read`, per workspace, optionally per conversation. Pages are oldest first. `older` walks back; `newer` returns only what came after, which is how a reconnecting client catches up. Cursors are `a1.o<seq>` / `a1.n<seq>` on `agent_events.seq`, and expire when the event they name is gone (cleared, pruned) or the filter no longer shows it.                                                                                                                                                 |
| What an event shows      | Only the `metadata` privacy class. Each payload goes through an allowlist of fields (`ActivityDetail`), each checked by class: short codes, names that are not paths, counts, and paths inside the workspace (no absolute path, drive or `..`). Left behind: Codex's home folder, native session ids, the PTY id, tool-use ids, launch-error text, anything not listed.                                                                                                                                                          |
| Other terminals          | `session.list` adds `terminals` on its first page: the daemon's sessions in the workspace that belong to no record of it, with a purpose (`shell`, `runCommand`, `program`, `agent` for a forgotten record, `other` when recording was off) from labels and the activity run, and the program's base name. Never argv or path.                                                                                                                                                                                                   |
| Terminal output frames   | Binary: kind, version, stream `u32`, epoch `u32`, seq `u64`, then ≤ 64 KiB of raw bytes. Never JSON.                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Terminal control         | Specified (`attach`, `detach`, `acquire`, `renew`, `release`, `input`, `resize`; lease id + generation + input seq; 30 s lease, 10 s renewal) and **refused by every host** (`unsupported`, not advertised) until R2 enforces it in the daemon.                                                                                                                                                                                                                                                                                  |
| Memory                   | Read-only, approved only. There is no remote route to propose, approve, edit or revoke, so the facade adds no way for an agent to approve its own proposal.                                                                                                                                                                                                                                                                                                                                                                      |

## 6 · What shipped in this slice

- **`crates/remote-proto`.** Ids, scopes and grants, the read model, the request/response
  envelope, version negotiation, errors, bounds, the terminal attach/control schema, and the
  output frame codec. Nine hand-written JSON fixtures in `fixtures/v1/` pin the wire format.
- **`crates/core/src/remote.rs`.** `HostFacade { store, git, terminals, host }` with
  `welcome`, `handle(grant, request)` and `handle_bytes(grant, bytes)`. It answers
  `host.describe`, `workspace.list`, `workspace.describe`, `session.list` and `memory.list` from
  `Projects::list`, `Store` and `TerminalHost::list`. It refuses everything else.
- No migration, no setting, no Tauri command, no CLI command, no listener. `src/lib/bindings.ts`
  is unchanged.

Second slice, 22 October 2026:

- **`activity.list`** in the protocol (`ActivityListParams`, `ActivityPage`, `ActivityEvent`,
  `ActivityDetail`, `Provenance`, `EventId`) and in the facade. The projection lives in
  `crates/core/src/remote/activity.rs`. The store gained `events_window` (either side of a
  sequence number, oldest first) and `event_matches` (whether a cursor's event is still there).
  It reads the existing `agent_events_timeline` index. No migration.
- **Other terminals** on `session.list` (`WorkspaceTerminal`, `TerminalPurpose`). The run
  command's label is now the named constant `launch::PROJECT_RUN_LABEL`.
- Two new fixtures (`request-activity-list.json`, `response-activity.json`), and a terminal in
  `response-sessions.json`. A sessions page without `terminals`, as a host from the first slice
  sends, still reads (tested). Both additions are additive within 1.0. Nothing had shipped, so
  the minor stays 0.

Third slice, 22 October 2026, in `crates/pty-host`:

- **Numbered output.** Each session counts the items of its output stream: every batch, and
  every resize that changes the size. `PtyHost::attach_numbered` hands a viewer a snapshot
  tagged with the item it holds (`Numbered::Snapshot { seq, size, bytes }`), then
  `Numbered::Output` and `Numbered::Resized` items numbered from `seq + 1`. A resize is an item
  because output drawn at the new size must not reach a viewer before it hears of the size; the
  view lock is now held across the PTY's own resize to guarantee that order. The epoch is the
  session's life in one daemon, as §3 planned. The plain `attach`, the `TerminalHost` trait and
  `pty_ipc` are unchanged, so the daemon's protocol does not carry numbers yet (R2).
- **A numbered viewer watches; it never answers.** The host still answers cursor queries when
  only numbered viewers are attached, which is what a remote viewer needs (the protocol forbids
  viewers to answer) and what keeps ConPTY from stalling at startup.
- **Found and fixed: a snapshot cut inside a sequence.** Batches end wherever a read ended. A
  viewer that attached between the two halves of an escape sequence got the second half as text
  (`1mred` for a red `red`). One that attached inside a character lost it. Worse, `vt100` 0.16
  drops the byte after a character split across two `process` calls when more multibyte text
  follows, so the host's own headless terminal could lose a space for good. This affected the
  desktop's re-attach and `ys attach` today, rarely. `snapshot::Headless` now wraps the parser.
  `unfinished.rs` follows the DEC parser's state machine and ends each snapshot with the open
  sequence's raw bytes (a string over 4 KiB keeps only its introducer, so the viewer still
  swallows the rest). A control carried out inside a sequence (LF, TAB, BS) is not among those
  bytes: its effect is already in the snapshot, and replayed it moved the viewer's cursor twice,
  which review caught. A split character is held back from the parser, at most three bytes, and
  handed to it with the next batch. Viewers' streams are never held back or delayed. A
  CHANGELOG line covers the user-visible part.

## 7 · Verification

- `cargo test -p remote-proto`: 24 tests.
  - Fixture round-trips, so a shape change has to be deliberate.
  - Another major refused with the majors spoken here; a newer minor's unknown fields accepted;
    an unknown op is `unsupported`.
  - Malformed, oversized (> 64 KiB) and 10 000-deep requests refused without echoing content.
  - Id validation against paths, whitespace and escape sequences.
  - Output-frame layout, round trip, truncation, foreign kind, future version and oversize.
- `cargo test -p yardsort-core remote::`: 10 tests against the real things: a SQLite store,
  git repositories with real worktrees in temp dirs, and agents as real processes in real PTYs
  (`/bin/sh` on Unix, `cmd.exe` on Windows).
  - Two projects. A grant for one cannot list, describe, list sessions of or read memory of the
    other, and gets the same `not_found` for the other's ids as for ids that exist nowhere.
  - Missing scopes are `forbidden`.
  - Pages follow cursors; a cursor whose item was forgotten expires.
  - Archived workspaces appear only when asked for.
  - Only approved memory appears, with the share switch off, and reading it changes no entry's
    state.
  - A live agent is `running` with its terminal's size and activity. A removed one is `gone`.
    One started in another workspace is not attributed. Without a daemon, state is `unknown`.
    After a kill, the terminal shows `exited`.
  - The serialized responses contain neither the temp directory's name, nor the process's argv
    or environment, nor the `harnessSession` label, nor the first message.
  - Terminal operations are `forbidden` without scope and `unsupported` with it. The terminal
    host the facade is given panics on any call but `list`, so the test proves the facade never
    attached, typed, resized, spawned or killed.
- On Linux, 2026-10-21, `just check` passed: formatting, clippy with warnings as errors, types,
  every Rust test and the 642 frontend tests. `just bindings-check` passed, and
  `src/lib/bindings.ts` is unchanged. The run unset the `YARDSORT_*` variables of the Yardsort
  terminal it ran in, which `launch::tests` otherwise trip on. `just lint-windows` and
  `cargo clippy -p remote-proto --target x86_64-pc-windows-msvc` are clean.
- The proposal file failed `prettier --check` at HEAD (its tables were not aligned), which made
  `just check` fail on `main`. It was reformatted here. The change is whitespace only, apart from
  its new status line.
- Second slice, on Linux, 2026-10-22:
  - `remote-proto`: 25 tests. The activity fixtures round-trip, and a sessions page without
    `terminals` reads as having none.
  - `remote::`: 17 tests. Three are unit tests of the field checks: codes, names, and paths
    inside the workspace.
  - Activity pages walk back to the oldest event and stop. A catch-up with nothing new returns
    the same cursor, and three new events come back alone and in order. Events from another
    workspace or of the `content` class never appear between them.
  - Activity is `forbidden` without `activity:read`. Another project's workspace, and a
    conversation from another workspace, get the same `not_found` as ids that exist nowhere.
  - A cursor that is foreign, malformed or negative expires, and so does one whose event the
    session filter would not show, or that was cleared.
  - Hostile payloads keep their listed fields. On the wire there is no PTY id, home folder, Codex
    home, native thread id, absolute or escaping path, command, tool-use id, launch-error text,
    prompt text or `effort`. Making the path or reason checks pass everything through fails the
    test.
  - Seven real PTYs: a conversation, an earlier process of it, the run command, a recorded shell,
    an unrecorded process, an agent whose record is gone, and another workspace's shell. The four
    that belong to no conversation are listed once, each with its purpose and the base name `sh`
    (`cmd` on Windows). No argv, `/bin/` or run id is on the wire. Terminals come only on the
    first page, and not without a daemon.
  - `just check` passed (`yardsort-core` 541 tests, the app crate 150, frontend 642), with the
    `YARDSORT_*` variables unset. So did `just bindings-check` (bindings unchanged),
    `just lint-windows` and the Windows clippy of `remote-proto`.
- Third slice, on Linux, 2026-10-22:
  - `pty-host` unit tests: 12. Eight sample outputs (colours, true colour, multibyte text and
    emoji, an OSC title, OSC 8 links, a DCS string, the alternate screen, line-drawing charsets,
    `CAN` and abandoned sequences, editing and mode sequences) are each cut at every byte. Both a
    host fed the two halves as separate batches, and a viewer sent the snapshot then the second
    half, must end with the same screen, cursor and modes as a terminal fed the whole. Taking
    the open sequence out of the snapshot fails it at the first mid-sequence cut; not holding
    back a split character fails it on the host side at `caf` + half of `é`.
  - `pty-host` integration tests: 19, three new, against real processes in real PTYs. On all
    platforms, a numbered viewer of a short program gets an unbroken run of numbers after its
    snapshot, and one that attaches after the exit gets a snapshot at the last number. On Unix:
    - A program prints 150 coloured, multibyte lines, waits for input, then prints 150 more. One
      viewer attaches at the start and another at the pause. The terminal is resized, the
      program continues, and a third viewer attaches after the exit.
    - All three have unbroken numbers. The two live ones were sent identical items after the
      second's snapshot, including the resize. The first viewer's stream up to the second's
      snapshot number draws exactly that snapshot's screen. All three draw the same final screen
      and cursor.
    - With only a numbered viewer attached, a program asking for the cursor position gets the
      host's reply. Counting numbered viewers as terminals makes it hang, and the test fails.
  - `just check` passed with the `YARDSORT_*` variables unset: `pty-host` 12 + 19,
    `yardsort-core` 541, `remote-proto` 25, the app crate 150, frontend 642. So did
    `just bindings-check` (bindings unchanged) and `just lint-windows`. The latter caught a
    Unix-only test helpers left unused on Windows, now gated.
- Review fixes, on Linux, 2026-10-22:
  - A ninth sample puts LF, TAB, BS and CR inside CSI and escape sequences. Keeping those
    controls in the snapshot fails it at the first cut after the LF.
  - Every facade test now also checks that each response it reads is within 64 KiB. Three new
    tests: fifty approved notes at the length limit in four-byte characters (over 100 KB at the
    default page size) come back over several pages, each within a frame, every note once;
    two hundred events with every name at its limit page back from the newest and forward from
    the oldest without a gap; and the largest possible item of every list fits a page with room
    for the envelope and cursors. A long session title comes back clipped to 256 characters.
    Each fails with its fix taken out.
- **Not run here:** tests on Windows and macOS. CI covers them; the core cannot be compiled for
  MSVC from Linux, since it bundles SQLite. No mobile device, emulator or simulator was involved,
  because nothing mobile exists yet.

## 8 · Progress ledger

Each later session picks one gate, updates this table, and records what it found.

| Phase                          | State                                                                                                               | Gate                                                                                                                                                                                                                                                                                                         |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| R0 Fit                         | **Done** (this document)                                                                                            | HEAD and paths recorded; no overlapping work found                                                                                                                                                                                                                                                           |
| R1 Contract and host facade    | **Done** in three slices: read contracts, facade, terminal schema, activity reads, other terminals, numbered output | Met: local behaviour preserved, incompatible clients refused, auth boundary documented. Moved on: numbers over the daemon protocol and `activity.subscribe` (R2, with a transport); a TS conformance run over `fixtures/v1/` (R3)                                                                            |
| R2 Identity and relay          | **In progress**, recorded in [27](27-remote-r2.md): sign-in and device credentials built                            | Persisted host identity at enrollment; credential store in the core (or a documented secret mount); grants from enrollment; lease and single query responder in the daemon (`PROTOCOL` 2); outbound connector; relay; two-account isolation; revocation kills a live stream; relay failure leaves PTYs alive |
| R3 Mobile vertical slice       | Not started                                                                                                         | Physical iPhone and Android, real sessions                                                                                                                                                                                                                                                                   |
| R4 Mobile hardening            | Not started                                                                                                         |                                                                                                                                                                                                                                                                                                              |
| R5 Headless and desktop remote | Not started                                                                                                         | Needs resume/fork moved from `src-tauri/src/sessions.rs` into the core first                                                                                                                                                                                                                                 |
| R6 Durable sync                | Not started                                                                                                         | Logical-id mapping table; outbox written in the same transaction as each opted-in mutation                                                                                                                                                                                                                   |
| R7 Managed launch              | Not started                                                                                                         | Owner-authorized only                                                                                                                                                                                                                                                                                        |

## 9 · Remaining risks and the next slice

- **Lease enforcement is a daemon protocol change.** Users with agents running on an old daemon
  will not get remote terminals until that daemon is idle and replaced. This is by design: we
  never kill work to upgrade.
- **Multi-viewer query replies are already duplicated locally** (app plus `ys attach`). No report
  traces a problem to it, but R2 must fix it before any remote viewer adds a third responder.
- **Children get the user's whole environment.** Remote enrollment must keep device credentials
  out of it, and the facade's tests prove nothing about a future transport's logs.
- **`title` is user content** and can quote a first message. It crosses the wire; clients must not
  log it.
- **Workspace listing asks git for each workspace's head**, as the sidebar does. That is fine at
  tens of workspaces; the proposal's 100-workspace p95 budget is unmeasured.

- **Activity details are only as private as their producers' normalizers plus the allowlist.**
  A new adapter field is invisible remotely until it is added to `ActivityDetail` on purpose, which
  is the intended default. Tool names and model names are free text from the agent, bounded and
  checked not to be paths, but otherwise shown as given, as the desktop timeline shows them.
- **A shell started while activity recording was off** lists as `other`: the host keeps no other
  record of what it is.
- **Snapshots do not carry a scroll region or a saved cursor.** `vt100`'s `state_formatted`
  emits neither. A viewer that attaches while a program has a scroll region set scrolls the whole
  screen until the program sets one again. This predates remote access and affects local
  re-attach too; fixing it means rendering those two states ourselves.
- **No replay window.** A reconnecting viewer always gets a fresh snapshot. Resuming from a
  number without one (`AttachStart::Resumed`) needs a bounded buffer of recent items per session,
  sized against memory, in R2.
- **The tracker follows `vte`'s state machine as written for 0.15.** A `vt100` upgrade that
  changes how sequences end could put the two out of step. The every-byte test is what would
  notice, so it runs on every build.

**Next: R2, identity and relay**, now started: see [27](27-remote-r2.md). Its gate was the
owner's choice of identity provider for the self-host recipe (proposal §16): ZITADEL. It also needs a lease and a single query responder enforced in
the daemon (`pty_ipc::PROTOCOL` 2, which can carry the numbering added here), a host identity
persisted at enrollment, and a credential store usable without the desktop app (open question
35).

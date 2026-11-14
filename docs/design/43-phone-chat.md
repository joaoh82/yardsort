# 43 · Chat: agent sessions on the phone

_Proposed and approved on 2026-11-14, and built the same day. The Figma frames are on the page
**Agent session · proposal** of the phone's design file
(<https://www.figma.com/design/ezlPCcxVNBFKP0bkGODMHZ>, node 24:2). Not yet run on a phone: see
[§ what is left](#what-is-left)._

## The problem

The phone showed an agent's conversation as its terminal, at the computer's size. On a phone that
scrolls sideways, and going back through what happened meant scrolling through the terminal's
history a line at a time. It worked, but a person reading a conversation on a phone wants what was
asked and what was answered, in one column they scroll up and down, and to answer it.

## What was decided

The owner's choices, on 2026-11-14:

1. **A conversation opens as Chat**: a column at the phone's width, read from the agent's own
   record. The terminal stays one tap away, in the same header (**Chat | Terminal**).
2. **Chat is the default** for every conversation it can show, and **the last choice is
   remembered** for each conversation.
3. **A permission the agent asks for is answered on the phone, through the agent's own hook**,
   and the card shows the command it wants to run.
4. **The terminal, while only watched, is zoomed out to fit the phone's width**, and pinched to
   read. It no longer scrolls sideways at its normal size.

## What Chat is built from

Nothing is read from the terminal. `AGENTS.md`'s _never parse agent output_ holds.

| Chat shows                     | From                                                                                                                                                                                                                                      |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What you typed and the replies | The harness's own transcript, through the readers that build `ys workspace context` ([29](29-conversation-context.md)): Claude Code, Codex, Cursor, OpenCode, Grok, OMP and Pi. Read on the computer each time it is asked, kept nowhere. |
| The steps of a turn            | The activity the phone already follows: `tool.started`, `tool.completed` and `tool.failed`, with the tool, the file and how long it took ([10](10-agent-events-stage-1.md)).                                                              |
| Working, Done, Needs you       | The same activity: a prompt or a tool since the last `turn.completed` is working; `approval.requested`, or an open question, is needs you.                                                                                                |
| The question's card            | The question the agent's `PermissionRequest` hook wrote, below.                                                                                                                                                                           |
| Sending a message              | Typed into the terminal, as the desktop's composer does: control is taken, the text and Enter sent, and control given back at once. The terminal is not resized.                                                                          |

A conversation of a custom harness has no reader, so it opens on its terminal and offers no Chat.
So does any conversation on a computer from before this change, which does not answer
`conversation.read`.

## The protocol: minor 8

Two operations ([`remote_proto::conversation`](../../crates/remote-proto/src/conversation.rs)):

- **`conversation.read { workspace, session, from? }`** needs `terminal:read`: the conversation's
  text is what its terminal shows a device that may watch it. Without `from` it gives the newest
  turns that fit in one message, and with it the turns from that one on. Each text is cut at
  16,000 characters and a turn keeps its last 24 texts; a turn that lost anything says so (`cut`),
  and the phone points to the terminal for the rest. A turn too large for a message on its own is
  cut until it fits, so the newest always comes. The reply carries the conversation's open
  questions, and whether its agent answers them through Yardsort.
- **`approval.answer { workspace, session, ask, decision }`** needs `terminal:control`: answering
  is what a key pressed in the terminal would do. A question that is not open any more — answered
  at the terminal, by another device, or the agent moved on — is `not_found`.

The phone reads again when the conversation's activity says something happened (twice: an agent
writes its reply just after the hook that says its turn ended), and every four seconds while the
agent works.

## Answering a permission through the hook

What was measured with Claude Code 2.1.325, on 2026-11-14, with a `PermissionRequest` hook that
waited before answering:

- **The terminal shows its question at once**, while the hook is still running. Nobody at the
  computer waits on the hook.
- **A key pressed in the terminal answers it**, and Claude Code goes on. The hook is not stopped,
  and what it prints later is ignored.
- **The hook's answer, when it comes first, closes the terminal's question.** Claude Code reports
  _Denied by PermissionRequest hook_ for a denial.

So the hook waits for a device, and whichever answer comes first is the one Claude Code takes:

1. Claude Code's `PermissionRequest` hook runs Yardsort's hook as before, which reports
   `approval.requested`. For an agent Yardsort launched, it then writes the question to
   `<data>/activity/asks/<id>.json` — the tool, the command, the file it would touch — and waits
   for `<id>.answer` beside it, looking four times a second, for up to 590 seconds. That hook is
   given Claude Code's own 600-second timeout; every other keeps 5.
2. `approval.answer` writes the answer file, once: a second answer is refused.
3. The hook prints Claude Code's decision (`allow`, or `deny` with a message) and exits 0.
4. **When the agent moves on without it**, the question is closed: the agent's next hook about the
   same tool call (`PostToolUse`, `PostToolUseFailure` or `PermissionDenied`, matched by a digest
   of the tool and its input) or the end of its turn removes the file, and the waiting hook exits
   without printing anything. A question older than 590 seconds is stale whatever happened.

The command is the one thing here that is not metadata. It is held in that file only while the
question is open, is never written to the database, and goes only to a device that may type into
that terminal. Claude Code is the only agent answered this way so far; for the others a card says
the agent is waiting and opens the terminal. OpenCode's `permission.ask` and Pi's
`tool_approval_requested` could do the same through their plugin and extension.

## The terminal, zoomed out

The terminal's page ([`terminal/page.ts`](../../apps/mobile/terminal/page.ts)) sets its viewport
to the terminal's own width whenever that is wider than the phone, with the scale that fits it, and
up to four times that to pinch in. Once the terminal fits the phone — as it does when the phone has
control — the viewport is the phone's width again. The page's pixels are not changed by zooming,
so the cell size the app works out what fits from is the same.

## On the phone

- [`src/lib/chat.ts`](../../apps/mobile/src/lib/chat.ts): the conversation's turns, earlier pages
  and refreshing, and steps and state from activity.
- [`src/lib/markdown.ts`](../../apps/mobile/src/lib/markdown.ts) and
  [`src/ui/markdown.tsx`](../../apps/mobile/src/ui/markdown.tsx): the Markdown agents write, with
  no new dependency. Code wraps; a table is a card per row, each cell under its column's name; a
  link asks before it opens.
- [`src/ui/chat.tsx`](../../apps/mobile/src/ui/chat.tsx): the column, folded steps, a reply past 24
  lines folded behind **Show all**, the question's card, **Latest** after scrolling up, and the
  composer, with **Stop** (Esc) while it works, a picture, and **Keys**, which opens the terminal.
- [`src/lib/views.ts`](../../apps/mobile/src/lib/views.ts): Chat or Terminal for each
  conversation, by its id only, in `views.json` in the app's documents folder, the newest 200.
- [`src/app/terminal.tsx`](../../apps/mobile/src/app/terminal.tsx): one screen for both. It finds
  the conversation behind a terminal itself, so a notification's link and the workspace's rows are
  unchanged. An ended conversation can be read in Chat too, with no composer.

## Where it differs from the Figma file

- The steps' summary counts each tool by its own name (`read 2 · edit 3 · bash 1`), as the frames
  show, but in lower case whatever the agent calls it.
- **Keys** opens the terminal instead of a sheet of keys over Chat: the terminal has the whole row
  and the control that goes with it.

## Tests

- Core: reading a recorded Claude Code transcript through the facade, whole and in pages, a huge
  turn cut to fit, the grants that may and may not read it or answer; the question files, the
  first answer standing, closing by the same call or the end of the turn, stale and unsafe ids;
  the hook waiting and printing, and a sibling hook closing it.
- Protocol: fixtures for both requests and the reply, read by Rust and the TypeScript client.
- Phone: the Markdown reader, steps and state, paging, the remembered view; screens for Chat by
  default, the switch remembered, a question answered, a message sent with control given back, a
  terminal someone is typing in, and a computer from before Chat. The terminal's page in a real
  browser: zoomed out to fit, allowed to pinch, back to the phone's width once it fits.

## End to end on this computer

On 2026-11-14, Linux, Claude Code 2.1.325 in tmux with Yardsort's hook settings and the `ys` built
from this change, an answer file written as `approval.answer` writes it:

- **Deny from the phone:** the terminal showed its question while the hook waited; the answer
  closed it, and Claude Code said _Denied by PermissionRequest hook — Denied from Yardsort on a
  phone_. The question's file was gone.
- **Allow from the phone:** the command ran, and the file it wrote was there.
- **Answered at the terminal** (1, Yes): the command ran, its `PostToolUse` closed the question,
  and no hook was left waiting.

## What is left

- **Run it on a phone**: Chat against a real Claude Code conversation, a question answered from
  the phone, the pinch on iOS and Android's WebViews. A headless browser does not pinch.
- OpenCode's and Pi's questions, through their own plugin and extension.

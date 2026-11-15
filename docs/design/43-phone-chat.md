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

| Chat shows                     | From                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What you typed and the replies | The harness's own transcript, through the readers that build `ys workspace context` ([29](29-conversation-context.md)): Claude Code, Codex, Cursor, OpenCode, Grok, OMP and Pi. Read on the computer each time it is asked, kept nowhere.                                                                                                                                                                                                            |
| The steps of a turn            | The activity the phone already follows: `tool.started`, `tool.completed` and `tool.failed`, with the tool, the file and how long it took ([10](10-agent-events-stage-1.md)).                                                                                                                                                                                                                                                                         |
| Working, Done, Needs you       | The same activity: a prompt or a tool since the last `turn.completed` is working; an open question is needs you, and for an agent not answered through Yardsort so is an `approval.requested` nothing came after. Working whose terminal has been quiet for 15 seconds is **Idle** (Esc ends a turn with no hook), and a conversation whose terminal is gone is ended: both from the terminal's activity in `session.list`, checked every 5 seconds. |
| The question's card            | The question the agent's `PermissionRequest` hook wrote, below.                                                                                                                                                                                                                                                                                                                                                                                      |
| Sending a message              | Typed into the terminal, as the desktop's composer does: control is taken, the text and Enter sent, and control given back at once. The terminal is not resized.                                                                                                                                                                                                                                                                                     |

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
- **Yes pressed in the terminal answers it**, and Claude Code goes on. The hook is not stopped,
  and what it prints later is ignored.
- **No, or Esc, pressed in the terminal ends the hook** — and runs no other hook: no
  `PermissionDenied` (that one is for denials Claude Code makes itself) and, for Esc, no `Stop`.
  The transcript gets the call's `tool_result` (_The user doesn't want to proceed…_) either way.
  Found by the code review of this change, and measured after it.
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
4. **When the agent moves on without it**, the question is closed, three ways:
   - the agent's next hook about the same tool call (`PostToolUse`, `PostToolUseFailure` or
     `PermissionDenied`, matched by a digest of the tool and its input), or the end of its turn,
     removes the file;
   - the waiting hook reads what Claude Code appends to its own transcript, finds the newest
     `tool_use` with the question's tool and input, and stops waiting once a `tool_result` names
     it — Yes, No or Esc at the terminal;
   - the waiting hook touches its file every 2 seconds, and a file untouched for 10 is a question
     nobody waits on: Claude Code ended the hook. The next read or answer removes it.

   A question older than 590 seconds is stale whatever happened. An answer written just as its
   question closed is refused, not reported as taken.

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
- **No, or Esc, at the terminal:** Claude Code ended the hook, and its question's file was left
  untouched from then on, so the next read closed it.

## A smoke test waiting, answered in Chat

Asked for by the owner on 2026-11-15, after the first days of real use: an agent ran
`ys test run`, the run waited for its approval card ([32](32-smoke-tests.md#trust)), and the
person was looking at the conversation on a phone, with no way to let it go. The frame is **S1**
on the page **Pull requests · proposal** of the phone's design file (node 36:317).

**Protocol minor 10** ([`remote_proto::smoke`](../../crates/remote-proto/src/smoke.rs)):

- **`test.waiting { workspace }`** needs `terminal:read`. It gives the workspace's oldest waiting
  run as the card shows it, and how many wait behind it: who asked and the conversation that did,
  the inputs, every step in words, the names of the test environment the file templates and, when
  a step runs a program, every name in it, and both versions of the file when it changed since it
  was approved. What it says is a file of the repository in words, with every variable left as
  written, so it is what a terminal in the workspace would show. No value of the test environment
  is in it, and a test pins that.
- **`test.decide { workspace, run, contentHash, decision }`** needs `terminal:control`: letting a
  run go is running programs as the computer's user, which is what typing into a terminal does,
  and what starting an agent from the phone already rests on. `run` approves the file at that
  hash for the project and queues the run, as the window's **Run** does; `refuse` ends it as
  refused and records nothing. A run that waits no more is `not_found`; a hash that is not the
  run's is `stale`.

**The gate is still a person seeing the file.** The card is built on the computer by the same
code the window's card uses (`smoke::runs::review`, moved into the core for this), once for the
reading and again for the deciding. One that does not fit a message whole — more than 60 steps, a
step's sentence past 400 characters, a changed file past 10,000 characters, or a file that does
not check out — is sent marked not `whole`, the phone offers only **Don't run**, and the computer
refuses a `run` for it whatever the phone sends.

**No agent can reach it.** There is still no `ys` command that approves. These operations are
answered only on a device's channel, for a device the person paired at the computer; the grant
they need is the one that already lets that device type any command into a terminal there.
Unlike a workflow's decision ([41](41-mobile-workflow-controls.md)) there is no one-time code and
no limit to phones: a workflow's gate is a sign-off an agent must not forge on a computer where
it can run `ys`, while this gate is passed by anything that may type, by design.

**Where it runs.** The desktop app answers, and tells its window and its runner at once, so the
card there goes and the run starts. A computer with no screen queues no runs and does not say it
answers these.

**On the phone.** Chat asks what waits every five seconds and when the conversation's activity
says something happened, and shows the card under the turns, in every conversation of the
workspace, since the run belongs to the workspace and not to one agent. The card says who asked:
this conversation's agent by its name, another agent, a workflow, or the person at the computer.
A run this conversation's agent asked for makes the header say **Needs you**: its agent is
waiting in `ys test wait`, which otherwise reads as working, then idle. What changed is drawn on
the phone from the two versions, as the changed lines and two lines either side.

## Projects that fold

Also asked for on 2026-11-15: on a computer's screen, a project's heading folds its workspaces
away (frame **F1**, node 33:164). A folded heading keeps the count and says how many agents are
running inside, so folding never hides that work is going on. Which projects are folded is kept
on the phone, by the computer's and the project's ids, in `folds.json` beside the views; the
newest 200 are kept.

## Found on the iPhone: where the screen starts

The owner's first use of Chat on an iPhone, 2026-11-15, with a screenshot: with the keyboard up,
the field sat a header's height above it with empty room between, and the bar with the agent's
name and **Chat | Terminal** was drawn under the status bar.

Both had one cause. The screen took its place from the header's height, as if it always started
below the header. That holds on Android and on iOS before 26. From iOS 26 the header is glass
laid over the screen, which starts at the top of the window (31, where it was first met for
screens that scroll). A keyboard's place is given on the window, so the lift was too large by the
header's height, and a bar that is no scroll view got no room made for it.

`AboveKeyboard` (`apps/mobile/src/ui/above-keyboard.tsx`) now asks the system where the screen
starts. It lifts by that, and when the header covers the top it starts its content below the
header. Chat uses it, and so does the terminal, where a conversation's bar had the same fault
and the field stood a bar's height above the keyboard. A terminal that is no conversation's has
no bar, and still makes its own room under the header.

Two screen tests answer for the system, once as iOS 26 and once as Android, and the first fails
with the old offset. On the Android emulator the bar sits directly under the header and the field
directly on the keyboard, as before. No iPhone was at hand: the iOS 26 case rests on the
screenshot, the arithmetic and the tests until the next build is on the phone.

## What is left

- **See the fix above on the iPhone**: the bar below the header, and no room between the field
  and the keyboard, in Chat and in a conversation's terminal.

- **Run it on a phone**: Chat against a real Claude Code conversation, a question answered from
  the phone, the pinch on iOS and Android's WebViews. A headless browser does not pinch.
- OpenCode's and Pi's questions, through their own plugin and extension.
- **A smoke test answered from a real phone.** `.yardsort/tests/remote-phone-lets-a-test-run.yaml`
  passed on 2026-11-15 on Linux, 17 of 17 steps: the real window with Remote on against the
  stand-in server, a pretend phone that may type, and a real run. The phone read the card, was
  refused when it named another file's hash, let the run go, and the app carried it out. What
  that does not show: a real phone doing it, and the window's own card leaving on the phone's
  answer, which the test does not look at.
- A notice to the phone when a smoke test starts waiting. Today the card shows when Chat is open.
- A manual step's **Pass** and **Fail** from the phone.

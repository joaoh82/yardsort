# Terminals & sessions

The middle of the window is a real terminal. Agents run in it exactly as they do anywhere else —
Yardsort does not re-implement their interface or read their output.

## Tabs

Each workspace has its own row of tabs.

- The agent buttons on the right of the tab bar start that agent in this workspace, with no
  prompt. Each carries that agent's mark, which a tab running a conversation wears too, so a row
  of tabs says at a glance which agent is in which. (To start one _with_ a prompt and a fresh branch, use the [composer](workspaces.md).)
- **+** or `Ctrl+Shift+T` / `⌘T` opens a **shell** in the workspace folder — for running tests, a
  dev server, or git by hand, next to the agent.
- **×** or `Ctrl+Shift+W` / `⌘W` closes a tab and stops what runs in it.

Clicking a workspace that has nothing running — and no earlier conversations — opens a shell
there, so choosing a workspace always lands you somewhere useful.

Terminals keep running when you look elsewhere. Switch workspaces or tabs freely; when you come
back the screen is repainted exactly as it was, scrollback included.

### Copy, paste, links

|                    | Linux / Windows | macOS     |
| ------------------ | --------------- | --------- |
| Copy the selection | `Ctrl+Shift+C`  | `⌘C`      |
| Paste              | `Ctrl+Shift+V`  | `⌘V`      |
| Open a link        | `Ctrl`+click    | `⌘`+click |

Plain `Ctrl+C`, `Ctrl+V` and a plain click belong to the program in the terminal. See
[Keyboard shortcuts](shortcuts.md) for why.

### Typing to an agent

- **`Shift+Enter` adds a line** to the message you are writing, in Claude Code, Codex, Grok and
  OpenCode alike — the same as in kitty, Ghostty or foot. The key is sent in the encoding those
  terminals use, and only to a program that reads it: an agent, or anything else that asks the
  terminal for it. A shell at its prompt gets a plain `Enter`, as before.
- **Drop a file on the terminal** to give the agent its path. The terminal is outlined while you
  hover; on release the path lands at the cursor, quoted for the shell if it needs it, with a
  space after it so you can keep typing. Drop several and you get all of them. It is a paste, so
  it goes to whatever program is in the tab — a shell gets an argument it can use as it is. To
  put a path in the first message, before anything is running, drop the file on the
  [composer](workspaces.md#starting-one-the-composer) instead.

## Agents keep working when you close the window

Terminals do not belong to the window. They belong to a small background process — the
**daemon** — that Yardsort starts the first time it needs one and talks to over a private local
socket. Closing Yardsort is a disconnect, not a kill: whatever your agents were doing, they carry
on doing. So is Yardsort crashing, or being killed.

Open it again and it finds that daemon, reattaches, and repaints every terminal from where it got
to. Conversations that never stopped are still _running_ — they are not listed as
[interrupted](#resume), because nothing interrupted them. Any agent that is waiting for you wears
the **ringed** dot, so you can see at a glance what wants reading.

Because leaving processes behind is not something to discover by accident, **closing with agents
still working asks first**. The dialog names them and offers three answers:

- **Leave them running** — close; they carry on and are waiting next time. This is the default.
- **Stop them** — stop every agent, and the daemon with them.
- **Cancel** — stay where you are.

Nothing is stopped unless you say so. A shell sitting at a prompt is not counted as work, so a
window with only shells in it simply closes.

The daemon shuts itself down once there is nothing left to look after: no window open and no
agent running. There is one per profile (see `YARDSORT_DATA_DIR` in
[Settings](settings.md#environment-variables)), and the status bar shows its process id.

To go back to the old behaviour — terminals inside Yardsort, stopping when it does — start it
with `YARDSORT_NO_DAEMON=1`.

## Status dots

The dot on a tab, and the one on each workspace in the sidebar, tell you what is going on without
opening anything:

| Dot            | Meaning                                                                                                                                        |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| **pulsing**    | printing right now — an agent at work                                                                                                          |
| **solid**      | running but quiet for a few seconds — most likely waiting for you                                                                              |
| **ringed**     | an agent is waiting and you have not looked at it yet — it finished a long stretch of work, or it was already waiting when you opened Yardsort |
| **grey**       | nothing running                                                                                                                                |
| **red** (tabs) | the program exited with an error                                                                                                               |

This comes purely from terminal activity. Agents animate a spinner while they think, so for them
silence really does mean "your turn".

## The badge on a workspace row

The dot says whether something is running; the badge at the right of a workspace row in the
sidebar says what its **agents** have got to, so you do not have to open a workspace to find out:

| Badge           | Meaning                                                                                                                                   |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| **1**, **2**, … | that many agents are running but quiet — your turn. Filled in solid if they finished while you were elsewhere and you have not looked yet |
| **✓**           | every agent here has ended, cleanly                                                                                                       |
| **nothing**     | they are all working — the pulsing dot already says so, and a row that lights up only when it wants something is worth glancing at        |
| **✗**           | one of them exited with an error                                                                                                          |

Shells are not counted. A prompt sitting there is not news, and a workspace whose agent has
finished should not look busy because a shell is still open beside it.

## Notifications

When an agent has been busy for a while (eight seconds or more), goes quiet, and Yardsort is
**not** the window you are looking at, you get a desktop notification: _"claude is waiting —
project / workspace"_. Shells never notify, and neither does anything you are watching.

Turn it off in [Settings → General](settings.md#general).

## Sessions: resume and fork

Agents save their conversations on disk. Yardsort keeps a **record** of each one — which agent,
which conversation, your first message as its title — so you can return to it after the process
is gone: because you closed the tab, the agent exited, or you chose to stop it on quitting.

### Resume

Same conversation, new process.

- A workspace with nothing running lists its **previous sessions**. Press **Resume** on one.
- A tab whose agent has ended shows a bar with **Resume**; the dead terminal is replaced.

![Previous sessions](../images/sessions.png)

Sessions labelled _interrupted_ ended without saying so — Yardsort was killed, or the machine
went down. They resume like any other. An agent that simply kept working while Yardsort was
closed is not interrupted and needs no resuming: it is
[still running](#agents-keep-working-when-you-close-the-window), and comes back by itself.

### Fork

A **copy** of the conversation that goes its own way, in a new tab. The original is untouched.
Use it to try a different approach without losing the first. You can fork an ended session from
the list or the bar, and a running one by right-clicking its tab.

### Forget

**×** on a previous session removes it from Yardsort's list. The agent's own saved conversation
is not touched.

### What can be resumed

| Agent             | Resume / fork                                                                                                                                                                                                                                      |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code, Grok | Any recorded session. Yardsort chooses the conversation's id up front, so it can always name it.                                                                                                                                                   |
| Codex, OpenCode   | The **most recent** conversation in the workspace. These agents choose their own ids, and "continue the latest one here" is the only handle they offer. Older entries say so rather than offering a button that would open the wrong conversation. |

If a session cannot be continued — its agent was disabled or removed in settings, for example —
the list says why.

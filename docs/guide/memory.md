# Memory

A project's memory is a short list of lessons about it that its agents should know — "the tests
need `TZ=UTC`", "never edit `src/lib/bindings.ts` by hand", "the staging database is read-only".
You write them, and agents propose them; only the ones you approve ever reach an agent, and not
at all in a project where you turn that off.

It is not a record of what happened — that is [Activity](activity.md) — and not a conversation
the agents keep. It is what you would tell a new colleague on their first day in the repository.

## How it fills up

Nothing is ever added to a memory on its own. An entry gets there one of two ways: you type it
in the Memory view, or an agent runs `ys memory propose` and you approve what it proposed. An
agent only knows to propose when it has been told to, and it is told in the _Project memory_
section of its first message — which it gets while **Give this project's agents its memory**
is ticked, as it is unless you untick it. So a project whose switch you turned off, or whose
agents are started from the tab bar with no message, stays empty until you write to it yourself.

## The Memory view

Open **Memory…** from a [project's menu](projects.md#the-project-menu), or click the count on
a project's row when agents have proposed something. The view has three lists:

- **Waiting for you** — proposals from agents. **Approve** keeps one, **Reject** sets it aside,
  **Edit** rewrites it first. Each says where it came from: which agent, in which workspace.
- **Approved** — what agents are given. **Edit** rewrites an entry; **Revoke** stops giving it to
  agents without forgetting it.
- **Rejected and revoked** — kept, with their history, so a proposal you turned down is
  recognised when an agent makes it again. **Approve** or **Restore** brings one back.

Type in the box at the top and press **Add** to write an entry yourself. What you write is
approved as you write it. An entry is one short paragraph, at most 500 characters; line breaks
are folded into spaces, because each entry is quoted to an agent as one list item.

## Giving agents the memory

**Give this project's agents its memory**, in the view, is ticked from the start: the notes are
yours, about your project, going to agents you start in it. Untick it for a project whose agents
should be told nothing, and it stays off. While it is ticked:

- **Every agent you start in the project with a first message** gets the approved entries after
  that message, under _Project memory_. Each is cited, like
  `(memory ab12cd34, from claude in fix-login)`, and framed as a note rather than an
  instruction: where one disagrees with what you ask, your request wins. The section ends by
  asking the agent, before it finishes, to propose what the next agent should know. The composer
  says so under its pickers, with **Show** to read exactly what will be added, and a box to leave
  it out of that one launch. An agent started from the tab bar with no message gets nothing added.
- **An empty memory still asks.** With nothing approved yet, the section is only that request —
  it says the memory is empty and how to propose — and the composer says _nothing is approved
  yet_. This is how a memory gets its first entries: an agent that is never told about
  `ys memory propose` never runs it.
- **Handoffs** carry the same section in their packet, where you see and edit it with the rest.
- **What the workspace was asked stays your own words.** The memory goes to the agent, not into
  the conversation's record, so Assist and the next handoff read your task, not the memory.

Revoking an entry takes it out of every launch after that. Launches already running keep what
they were given.

## What agents can run

Every agent Yardsort starts can run [`ys`](cli.md#ys-memory), which ships with the app:

```sh
ys memory list                     # the approved entries, newest first
ys memory search timezone tests    # approved entries containing every word
ys memory propose "The tests need TZ=UTC."
```

`list` and `search` answer only while the project shares its memory — the same switch that adds
it to first messages — and otherwise say that it is not shared. `propose` works either way, since a
proposal reaches no agent until you approve it; it adds the proposal to **Waiting for you** and
says so; the same text twice is recognised,
not queued again. Yardsort knows which agent proposed it from the launch environment it gave that
agent. At most 50 proposals wait in a project at once; past that, `propose` refuses and says why.

There is no `ys memory approve`, on purpose. An agent can run anything `ys` offers, so approving,
editing, rejecting and revoking happen only in the app, by you.

## With Assist

With [Assist](assist.md) and its **Check memory proposals for repeats and contradictions**
switch on, each proposal in **Waiting for you** is marked when Jev reads it as saying what an
approved entry already says — **repeats an entry** — or the opposite of one — **may contradict
an entry**. That sends the proposals and the project's approved entries to TypeSafe, and nothing
else. Jev never writes or approves an entry; the marks are there to help you decide.

## Where it is kept

In Yardsort's local database, per project, with every change to every entry: who proposed or
wrote it, who approved, edited, rejected or revoked it, and the text an edit replaced. Removing a
project from Yardsort without keeping its history removes its memory too.

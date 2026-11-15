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

In a project that [syncs](#on-your-other-computers), a fourth list, **Conflicts**, comes first
when there is something in it.

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
- **Agents a [workflow](workflows.md) starts** get the approved entries too, but are not asked
  to propose, and get nothing while the memory is empty: a workflow that starts several agents
  would otherwise leave a proposal from each. They can still run `ys memory propose`.
- **Handoffs** carry the same section in their packet, where you see and edit it with the rest.
  So do **forks**: the packet that goes after your message carries it, and nothing else is added.
- **What the workspace was asked stays your own words.** The memory goes to the agent, not into
  the conversation's record, so Assist and the next handoff read your task, not the memory.

Revoking an entry takes it out of every launch after that. Launches already running keep what
they were given.

## Sharing conversations

The Memory view has a second switch, **Share conversations between this project's agents**, on
until you untick it. What an agent reads goes to that agent's provider, so untick it in a project
whose conversations should stay with the agent they were said to. It is not memory: nothing is written and nothing outlives the workspace. While
it is on, an agent in one of the project's workspaces can run `ys workspace context` to read what
was said in the workspace's other conversations, and a new agent started beside one is told so.
See [what agents can read of each other](terminals-and-sessions.md#what-agents-can-read-of-each-other).

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
editing, rejecting and revoking happen only in the app, by you, or on a computer with no screen
with [`yardsort-host memory`](headless-host.md#memory-there), at a terminal of your own.

## On your other computers

In a project you [sync](remote.md#syncing-projects) between computers, the notes you approve on
one reach the others, **approved**, since it was you who approved them, and their agents get them
like their own. A note from another computer says which, as in _from the user, on Build box_, in
the view and in the citation an agent reads: `(memory ab12cd34, from the user, on Build box)`. It
names that computer's workspace when an agent proposed it there, but its workspace and run stay on
that computer. Each note carries its history: who wrote, approved, edited or revoked it, and the
text each edit replaced. What it sends is in **Project settings → Sync** before you turn it on, and
the server keeps it in plain text, readable by whoever runs it.

- **Only approved and revoked notes sync.** A proposal waits on the computer whose agent made it,
  until you approve it there; a note you rejected stays there too.
- **Turning sync on** sends the project's approved notes. **Linking** a project on another computer
  sends its own approved notes too, after taking in the ones already synced; a note there with the
  same text, whatever its case, is taken as the same note rather than kept twice.
- **Revoking one** revokes it on every computer, and that wins over an edit another computer made
  meanwhile: a revoked note never comes back from an older copy. The edit is kept in its history.
  **Restore** is yours to do, on any of them.
- **Two computers that edit the same note apart**, offline say, both keep their text. Once both
  have synced, the text that reached the server first stands, and is what agents get; the other
  waits beside it, under **Conflicts**, on both computers, saying which computer it is from. Press
  **Keep this one**, **Keep Build box's** or **Write another**, or **Revoke** it. Whatever you
  choose goes to every computer; if you settle it on two computers at once, the first to reach the
  server stands, and the second yields to it. A third edit while one waits replaces the one
  waiting, which is kept in the history.

Notes sync while Yardsort runs with Remote on, right after you change one, and when another
computer did. **Stop syncing** keeps every note here as it is. The switches stay each computer's
own: whether a project's agents get its memory, and whether they share conversations.

## With Assist

With [Assist](assist.md) and its **Check memory proposals for repeats and contradictions**
switch on, each proposal in **Waiting for you** is marked when Jev reads it as saying what an
approved entry already says — **repeats an entry** — or the opposite of one — **may contradict
an entry**. That sends the proposals and the project's approved entries to TypeSafe, and nothing
else. Jev never writes or approves an entry; the marks are there to help you decide.

## Where it is kept

In Yardsort's local database, per project, with every change to every entry: who proposed or
wrote it, who approved, edited, rejected or revoked it, and the text an edit replaced. Removing a
project from Yardsort without keeping its history removes its memory too. A synced project's
approved and revoked notes are also on the server, until you remove the project from it.

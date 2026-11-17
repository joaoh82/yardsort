# Remote

> **Remote is still under development, and will be until the whole plan for it is finished.** Expect it
> to change. It stays off, and connects to nothing, unless you turn it on. [The phone app](phone.md)
> is invite only for now, and will be in the app stores in a few weeks.

Remote lets your phone reach the agents on this computer through a Yardsort server you sign in to. Look
at a workspace, watch a terminal, or take control of one and type, away from your desk. A computer with no
screen, such as a server you reach over SSH, can be a host too: see
[A computer with no screen](headless-host.md). A phone that may
type can also start a new workspace or another agent, resume a conversation that ended, and give an
agent a picture. The agents keep
running on this computer whatever happens to the connection. This computer can also reach another one
the same way, watch its terminals, type into them and start work there: see
[Reaching another computer](#reaching-another-computer). Your computers can sync a project with each
other too: see [Syncing projects](#syncing-projects).

Neither end listens for connections: this computer and your phone each reach out to the server.

## What the server can see

**Whoever runs the server can see what passes through it**, your terminals, your agents'
conversations as Chat shows them, and what you type into them included. The connection is encrypted on the way, but not end to end. Use a server you run yourself, or
one run by someone you trust with that. A project you [sync](#syncing-projects) is kept on the
server too, in plain text.

Neither the server nor this computer writes any of it in a log: not what a terminal shows or is sent,
not what a conversation says or its title, and not a memory note. Their logs say which device and which computer, and
when, and the errors.

## Turning it on

Open **Settings**, then **Remote**. Give the server's address and a name for this computer, then press
**Turn on and sign in**, which opens your browser. Its sign-in is kept in this computer's credential
store, and the section says where. While it is on, Yardsort keeps a link to the server from the moment it
starts.

**Turn off** ends that link.

## Letting a device in

Press **Pair a device**. It shows a QR code and a code to type, good for five minutes. Enter it on the
device. The device then asks, and it gets in only if you say yes here, under **Waiting for you**. You choose
whether it can only watch or can also type, and which projects it can reach. The status bar tells you when
one is waiting. Nothing but this window can say yes: not `ys`, and not an agent. On a computer with no
screen, it is [`yardsort-host pair`](headless-host.md#only-a-person-can-pair-a-device), at a terminal of
your own.

**Devices let in** lists them, and you can take any one back there at any time.

## Who has control

Any number of devices can watch a terminal, but only one can type into it at a time. The phone asks for
control, and the person at the computer can take it back at any moment just by typing.

## A keyboard with the phone

A Bluetooth keyboard paired with the phone, or one plugged into it, works once the phone has control.
What you type goes into the field, and Return sends it, as with the phone's own keyboard. These keys
go straight to the terminal instead, wherever the field is:

- **Esc**, and **Shift-Tab**.
- **Ctrl** with a letter, such as Ctrl-C to interrupt, and with `[`, `\`, `]` or space.
- **Alt** (Option on an Apple keyboard) with a letter, a digit or a symbol, as Alt-B and Alt-F move by
  word in a shell.
- **The arrows** with Shift, Alt or Ctrl, and **Backspace** with Alt or Ctrl.

**The arrows, Tab and Backspace on their own** go to the terminal while the field is empty, so they
move through an agent's menu or a shell's history. While something is in the field, they edit it.
Shortcuts with **Cmd** are left to the phone, and so are Ctrl-Tab and Alt-Tab.

## A computer's workspaces

On the phone, a computer's screen lists its workspaces under their projects, each with its branch
and how many of its conversations are running. Press a project's name to fold it: its workspaces
are put away, and the line keeps the count and, when agents are at work inside, how many are
running — _api-gateway 3 · 2 running_. Press it again to open it. The phone remembers which
projects you folded, for each computer, as this window remembers its own sidebar; nothing about
it is sent anywhere.

## Pull requests on the phone

When this computer has `gh` installed and signed in, the phone shows what became of a workspace's
branch, as the sidebar here does. It is read-only: nothing on the phone changes a pull request.

- **On a workspace's row**, a badge with its pull request's number, coloured as here: green for
  one open with nothing pending or wrong, yellow for a draft or one still waiting on checks or a
  review, red for a failed check, changes asked for or conflicts, purple for merged, and red
  struck through for one closed without merging. _#88 +2_ is a workspace with two more, and
  _lib#7_ is one it follows in another repository. Press the badge to see that pull request.
- **In a workspace**, a card above its conversations: the number and state, the title, the branch
  and what it would merge into, lines added and removed, the checks, the review, and whether it
  conflicts. **Show** opens the checks to each by name, the failed ones first; they are open
  already when one failed. **Open on GitHub** opens its page in the phone's browser. Others the
  workspace opened or follows are listed under the card.
- **Pull requests**, on a computer's screen with the count of open ones, lists them over every
  project the phone may see: **Open**, **Merged** and **Closed**. Merged and closed are those
  among the newest fifty of each project.

The phone asks this computer, never GitHub: it shows what this computer last read, and holds no
GitHub sign-in of its own. A screen showing pull requests asks again each minute while it is in
front, and this computer reads them again for it, so they stay current with this window in the
background. Nothing of them is kept on the phone, so they show only while this computer can be
reached.

Without `gh`, or signed out of it, the phone shows no badges and no card, and says nothing about
it, as here. The same goes for a computer on a version before this one.
[A computer with no screen](headless-host.md) reads them too, with its own `gh`, when the phone
asks, so the first look at one starts the reading and the list fills in a moment later. It does
not yet read the pull requests a workspace follows in another repository. A notification when a
check fails or a review arrives is not built yet.

## Starting work from the phone

A device you let type can start work here too. It is no more than it could do by typing into a terminal,
so it needs nothing else from you.

The same goes for what the desktop app on another computer can ask of this one, once you let it
type ([Reaching another computer](#reaching-another-computer)): a shell in a workspace, a workspace
renamed, archived or deleted, and a repository cloned and added as a project. None of them names a
folder here. A clone goes into the folder under **Projects cloned for a device** in Settings →
Remote, `~/Projects` unless you set another, and is fetched with this computer's own Git sign-in;
a device let into only some of your projects cannot clone. Archiving and deleting keep the branch,
and a workspace with uncommitted work is refused until the person at the other computer has been
asked a second time. A device you let only watch can do none of this.

Such a computer can also read a workspace's changes and files here: which files changed, the file
tree without what Git ignores, and the text of a file inside the workspace, up to 1 MB. It is a
path inside the workspace that is asked for, checked here, links included, and nothing outside a
workspace is answered. It is what the same device could read by typing `git diff` or `cat` into a
terminal, so it comes with letting it type and not with a permission of its own. What is read
passes through the server as terminal output does: carried, and not kept.

It can also commit everything changed in a workspace here, push the workspace's branch to its
project's remote, and open and merge its pull request, as the person there presses for each. This
computer does them as itself: with its own Git identity, its own Git credentials and its own `gh`
sign-in. No request names a remote, a branch to push to or a credential, and a merge goes ahead
only for the workspace's own pull request, open, not a draft, at the commit the request names.

- **New workspace**, on a computer's screen, asks for the project, the agent and what it should do. This
  computer makes the workspace as its [composer](workspaces.md#starting-one-the-composer) does: a branch
  named after the message, from the project's default branch, with the project's setup run, and the
  agent given the message. The model and effort are the agent's own defaults. Its
  terminal then opens on the phone. Setup that takes a while keeps the phone waiting; it carries on here
  whatever happens to the connection.
- **New agent**, in a workspace, starts another agent there, with a first message or none.
- **Resume**, on a conversation that ended, takes it up again in a new terminal, as
  [Resume](terminals-and-sessions.md#resume) does here: the same conversation, under the same record,
  with a message to go on with or none. A conversation shows it only when this computer could resume it, and only to a device you let type.
  Tap **Show ended conversations** in a workspace to see them. One whose terminal is still here also
  offers **See its last screen**.

The phone offers only the agents this computer has turned on in **Settings → Harnesses** and can find,
the first of them chosen to begin with, and each starts as those settings say. The workspace and its
tab appear in this window at once. If the connection drops before this computer answers, the phone says
the agent may have started anyway and does not try again: look at the list before starting it again.

## Giving an agent a picture

While a phone has control of a terminal, **Image**, at the start of its row of keys, lets you choose a
picture. The phone keeps control while its photo picker is open, though leaving the app otherwise
lets go of it. The phone sends it as a JPEG, no larger than 2048 pixels on its longest side, and this computer
keeps it in the profile's `uploads` folder, outside every worktree, so no branch picks it up. Its path
is then pasted into the terminal, as dropping a file on a terminal here does, with nothing after it, so
you can go on and type the message. Claude Code, for one, reads a pasted path to a picture as the
picture. Pictures are deleted after seven days, when another one comes.

## When the phone changes network

Moving from Wi-Fi to cellular, or back, can leave the phone's connection to the server dead without a
word. When the phone's network changes, it asks each computer it has open whether it still answers,
and opens the connection again within a few seconds if not, rather than waiting until it notices by
itself. One it was already waiting to reach again is tried at once. Nothing typed is sent again, and
control is not taken back: press **Take control** again once it is back.

## When a computer cannot be reached

On the phone, a computer is **Online** while the phone holds a connection to it, and **Reconnecting**
while it tries again. It shows **Offline, last seen** and when, once the server says the computer is
not connected to it: a computer that is asleep, shut down or off the network. Offline does not say
whether what runs there stopped: losing the connection stops nothing, and a computer that sleeps or
shuts down stops its agents with it. What the phone last read from the computer, such as a
workspace's conversations, approved memory or a workflow run, stays on its screen marked **As of**
when it was read. Nothing typed or chosen while it cannot be reached is kept to be sent later.

## Diagnostics on the phone

The phone's **Settings** is the gear at the top right of its first screen, Connect or your account.
Once the phone is signed in to a server, **Settings → Servers on this phone** lists every server it
is signed in to: switch to another, add one, or remove one.
**Settings → Diagnostics** shows what happened to the phone's connections since the app started:

- the app's version, the phone's system and the protocol it speaks;
- the server's id, and this phone's id there;
- each change to the sign-in, to each computer, named by its id, and to the network, with its time
  in UTC and the error code that came with it.

It holds no names, titles, paths or addresses, nothing a terminal showed and nothing typed. Read it
there, then **Share** sends that same text through the phone's share sheet, to whoever you choose.
Nothing is sent otherwise. Only the last 200 changes are kept, and only while the app runs.

## Chat on the phone

A conversation opens as **Chat**: one column at the phone's width, scrolled up and down, never
sideways. Each turn is what you asked, the agent's steps folded to one line — _6 steps · read 2 ·
edit 3 · bash 1 · 1m 12s_, tap it for each step and its file — and what the agent wrote back,
with its code, lists and tables drawn to fit. A reply longer than a screen or two is folded behind
**Show all**. While the agent works, its steps appear as it takes them, with how long it has been
working and **Stop**, which sends Esc. Scrolled up, **Latest** takes you back to the bottom, and
**Show earlier turns** at the top reads further back.

The header says which agent it is and what it is doing — working, done, needing you, or idle when
it says it is working but its terminal has been still for a while, as after Esc — and
**Chat | Terminal** switches to the conversation's terminal. The phone remembers which of the two
you last used for each conversation and opens it there next time.

The field at the bottom types your message into the agent's terminal on this computer, then
presses Enter, as the composer here does. The phone takes control only for that, and gives it
back at once; the terminal keeps this computer's size. If someone at this computer is typing,
the message waits in the field and the phone says so. **+** gives the agent a picture, and
**Keys** opens the terminal, for Esc, the arrows and the rest.

What Chat shows is read from the agent's own record of the conversation, on this computer, each
time the phone asks: Claude Code, Codex, Cursor, OpenCode, Grok, OMP and Pi. Nothing is read from
the terminal, and nothing is kept. A conversation with an agent you added yourself opens on its
terminal, as does every conversation while this computer runs a version from before Chat. A turn
too long to send to a phone says so, and the terminal shows the rest. An ended conversation can
still be read.

## When an agent asks for a permission

When Claude Code asks to run a command or edit a file, Chat shows what it wants — the command, or
the file — with **Allow once** and **Deny**. Its terminal on this computer asks at the same time,
and whichever is answered first is the one Claude Code takes: answer here, and the card goes from
the phone; answer there, Yes, No or Esc, and it goes from the phone within a few seconds. For any other answer, such as allowing it from now on, **Other answers: open the
terminal**. The phone needs to be allowed to type for this.

The command is kept on this computer only while the question is open, and never in its records.
For other agents, Chat says the agent is waiting and opens its terminal.

## When a smoke test waits for you

A [smoke test](smoke-tests.md) from the repository runs only once you have seen what it does. When
one waits in a workspace — an agent ran `ys test run` on a file that is new or has changed, or the
project asks to see every run — Chat in any of that workspace's conversations shows
[the approval card](smoke-tests.md#the-approval-card) this window shows: who asked and with what
inputs, every step in words, the names of the project's test environment it can read, and, for a
file you approved before, the lines that changed. Values of the test environment are never sent.

**Run** approves that version of the file for the project and starts the run here; **Don't run**
ends it as refused and records nothing against the file. The card here goes once the phone has
answered, and one answered here goes from the phone within a few seconds. Several waiting in one workspace
show one at a time, oldest first. When the run was asked for by the conversation you are reading,
its header says **Needs you** until you answer.

The phone needs to be allowed to type for this: letting a test run is running programs on this
computer, which is what typing into a terminal does. A phone that may only watch sees the card
and is told so when it answers. A file too long to show whole on the phone says so and offers
only **Don't run**; let that one run from here. A computer with no screen runs no smoke tests, so
there is nothing to answer there.

## Scrolling on the phone

A terminal wider than the phone, as one is at this computer's size, is zoomed out to fit the
phone's width; pinch to read it closer. Once the phone has control the terminal is drawn to fit
the phone, at its normal size.

Drag down on a terminal to go back through what scrolled off its screen, as far as this computer keeps
it, and up to come back. A terminal taller than the phone moves first; the history follows on in the same
drag. A program on its alternate screen, as a full-screen program uses, has no history to scroll.

## Reaching another computer

The desktop app can reach another computer running Yardsort too, as your phone does: one with the app
and Remote on, or one with no screen running [`yardsort-host`](headless-host.md). Its projects and
workspaces are listed in the sidebar under that computer, and a workspace there opens like one of your
own: its terminals as tabs, to watch and, once this computer has control, to type into, and that
computer's agents to start.

1. In **Settings → Remote**, under **Other computers**, give the server the other computer is signed in
   to and a name for this one, then press **Sign in to reach other computers**, which opens your
   browser. The server is filled in for you when this computer is a host on one.
2. On the other computer, show a code: **Pair a device** in its **Settings → Remote**, or
   `yardsort-host pair` on one with no screen.
3. Type the code under **Pairing code** here and press **Pair**. This computer asks to watch and to
   type, as a phone does. Watching includes reading workflow runs when this computer asked to see
   them. The other computer's person is asked there, and lets it watch, watch and type, or refuses.
   When this computer asked to approve or answer workflows, they can allow that separately. Until
   then the pairing says _waiting for its answer_.

**In the sidebar.** Once let in, the other computer is listed under **Other computers**, below your
projects, by its name and how it stands: _online_, _connecting_, _offline_ and for how long, _waiting_
while its person has not answered, or _watch only_. Under it are its projects, and under each project
its workspaces, drawn as your own are: the dot for what its terminals are doing, `local` with its
branch, how many agents are open, and the task it was started from, which opens the task's page.
Click a computer or a project to fold it; what you folded is remembered. A computer that is offline
folds by itself, and opened, it shows what it last showed, dimmed. The search above your projects
filters this list too, by a computer's name or a project's. The **+** beside the heading opens
**Settings → Remote**, where pairing is. Nothing of another computer is mixed into your own projects.

**A workspace there.** Click one to open it in the middle of the window, as a workspace of your own
opens. The bar across the top names the computer and says how it stands; the tabs are that
workspace's agents and other terminals; the bottom line says the computer, the project, the
workspace and its branch. While the other computer is away, it keeps what it last showed and says
when that was.

**Its changes and files.** The right panel shows them as it shows a local workspace's, read from
that computer when you look: **Changes** lists what is uncommitted and what is on its branch, and
a file there opens as a diff, inline or side by side; **Files** is its file tree, a folder listed
when you open it, and a file opens as text. Files are read-only: there is no editor, and nothing
read is kept on this computer. The panel reads again
whenever this computer reads that one again, which is when something happens there and every 20
seconds otherwise.

What it does not show: files Git ignores there, and `.git`, are not in the tree; a picture or any
other file that is not text is said to be binary; a file over 1 MB is said to be too large. The
panel needs that computer to let this one type, and Yardsort 0.35.0 there. Otherwise it keeps its
place and says why it is empty.

**Commit, push and the pull request.** Under **Changes**, for a computer that lets you type and
has Yardsort 0.35.0:

- **Commit** commits everything changed in the workspace there, with your message. It is that
  computer's Git that commits, so the commit is by the name and email Git has there, which the
  button's tooltip shows. With none set there, the bar says so and nothing can be committed.
- **Push _n_ commits** pushes the workspace's branch from that computer to its project's remote,
  with that computer's Git credentials. A push the remote refuses says why in a sentence: it has
  commits this branch lacks, a rule there protects the branch, or that computer's sign-in was not
  accepted. Git's own output is not sent here, since it names that computer's folders and the
  remote's address; open a shell in the workspace to read it.
- **Open pull request** asks for a title, a description and whether it is a draft. That computer
  pushes the branch if the remote lacks any of it, then opens the pull request with its own `gh`
  sign-in. Without `gh` there, or signed out of it, the forge's own form opens in this computer's
  browser with both branches filled in.
- **The pull request**, once there is one, is a line with its number and title, whether it is
  open, a draft, merged or closed, and what its checks say. The title opens its page. **Merge…**
  is offered for an open one that is not a draft and has no conflicts, with squash, merge commit
  or rebase beside it. You are asked first, and that computer merges only if the pull request is
  still at the commit you were shown. Its branch is not deleted.

Each of these is one press of yours. Nothing commits, pushes or merges by itself, nothing is
offered to an agent, and nothing is tried again when an answer is lost: the bar then says it may
have been done, and reads how the workspace stands. The forge is asked about the pull request when
the bar opens and after a push, an open or a merge, whether it was done or refused, not at every
reading. So a merge refused because the pull request was pushed to meanwhile shows the pull
request as it is now, and **Merge…** then asks about that commit. If the forge does not answer,
the bar says so and keeps what it last said.

**Who has control.** A terminal there shows what it shows on that computer, at its size there, and
the bar says who has control: nobody, the person at that computer, or a device by its name. Watching
takes nothing, and what you type goes nowhere. Press **Take control** to type into it: the terminal
then fits this window, and the person at that computer sees who has it. **Release** hands it back.
For a terminal that was already running, control is taken only by pressing. If the person at that
computer types, or a phone takes control, the bar says who took it and offers **Take control** again.
A computer that lets this one only watch offers no **Take control**, and says so; to change that,
**Forget** it under **Pairings**, pair again, and let it type there. If the connection drops, the
terminal is watched again by itself once the other computer is back, from where it was, and control
has to be taken again. Something typed as it dropped may not have arrived, and the window says so:
nothing is sent twice.

**Starting work there.** When that computer lets this one type, you start work there as you do here,
each by that computer's own projects, workspaces and agents:

- **A new workspace:** choose the computer in the composer. Its title ends in where the workspace
  will run, **This computer** until you change it; press that and pick a computer. The **+** on one
  of a computer's projects in the sidebar, or **New workspace** in that project's menu, opens the
  composer with that computer already chosen. Say what the agent should work on, pick one of that
  computer's agents, and press Enter. That computer makes the workspace. See [Run on](#run-on) below.
- **Another agent:** the buttons at the end of the tab bar start one of that computer's agents in the
  workspace, waiting to be told what to do. A workspace's menu in the sidebar offers the same.
- **A shell:** the **+** after the tabs, or **New shell** in the workspace's menu, opens that
  computer's shell in the workspace. It is yours to type into at once, like an agent you start.
- **Rename, archive, delete:** in the menu of a workspace there, for every workspace but the
  project's own checkout. **Rename…** changes the name shown; the folder and the branch keep
  theirs. **Archive…** and **Delete workspace…** ask first, remove the folder on that computer and
  keep its branch. If the workspace has uncommitted work there, you are asked a second time before
  it is lost, and a no leaves everything as it was. Restoring an archived workspace is done at that
  computer.
- **Resume:** an agent that ended there offers **Resume** over its terminal, and under **Previous
  sessions** when nothing runs in the workspace. It shows only for one that computer could resume.

What you start from here is yours to type into at once: this computer asks for control of its
terminal as soon as it opens, without a press. If that is refused, or the person there takes control
back, it is **Take control** from then on. It runs on that computer, with that computer's settings,
and is listed under it, never among your own projects. If the connection drops before that computer
answers, the window says it may have started anyway, and nothing is tried again by itself. A computer
that lets this one only watch offers none of these. One whose Yardsort is older than 0.35.0 starts
agents and resumes them, and does not offer the shell, the rename, archive and delete, or the fuller
composer described below. Nothing can be started on a computer that is away, and nothing is queued
for it.

### Run on

Once this computer is signed in to reach other computers, the title of the composer for a new
workspace reads _New workspace in project on **This computer**_. The last part is a menu:

- **This computer**, then every computer you are paired with and how it stands.
- A computer that is **online** can be chosen. One that is offline, still connecting, or that lets
  this computer only watch is listed with the reason and cannot be chosen. Nothing is queued for a
  computer that is away.
- **Pair another computer…** opens Settings → Remote.

Choosing a computer keeps what you have typed and changes the rest of the composer to that
computer's:

- **Harness**, **Model** and **Effort** list that computer's agents, the models its settings
  suggest and the efforts each agent takes there.
- **Branch** lists that project's branches on that computer: start a new branch from one, or open
  one that no workspace there has out.
- **Name** is the new workspace's folder there and its branch after that computer's prefix. Left
  empty, that computer names it after your message.

That computer adds its project memory and runs its setup by its own settings, and the triggers of
the new workspace are set there. A computer whose Yardsort is older than 0.35.0 shows only its
agents, and says that it names the branch and picks where to start itself. Choosing **This
computer** brings the local composer back, again with what you typed; the model, effort and branch
you picked for one computer do not carry to another.

**Project on _computer_** says which of that computer's projects the workspace will be made in. A
project here and one there are separate, so Yardsort offers one and says why: the one you chose
last time for this project, or else the one that is the same repository (both fetched from the
same place), or else the one with the same name. You can pick another. The choice is remembered on
this computer once something starts.

When that computer has no such project, the composer says so and offers **Clone it there**. Start
then has that computer clone the repository into its own projects folder and add it as a project,
and makes the workspace in it. Only the repository's address goes over, with any user name and
password taken out; that computer clones with its own Git sign-in, so a private repository it
cannot read fails there and the composer says so. Where it goes is that computer's setting
(**Projects cloned for a device** in its Settings → Remote, or `projects_dir` under `[remote]` in
a headless host's `settings.toml`), `~/Projects` by default. **Clone it there** is not offered
when this project has no remote another computer could reach, or when that computer's Yardsort is
older; and that computer refuses it when you are let into only some of its projects. Then add the
project there yourself (**Add project** in its window, or `ys project add` in its terminal).

The menu is offered for a plain new workspace only. A fork, a task from the Tasks view, a pull
request's branch and a run in an existing workspace all start from something on this computer, so
they stay here.

**The keyboard.** The next and previous workspace shortcuts go on from your own workspaces to the
other computers', in the sidebar's order, and the next and previous tab shortcuts switch a workspace's
tabs there. With a workspace there in view, the new workspace shortcut composes one on that computer,
in the same project, and the new shell shortcut opens that computer's shell in it. Closing a tab
there is not offered: its terminals are ended at that computer, or by the program in them.

**This is a device of its own.** Signing in here makes a second device at the server, apart from this
computer as a host: turning Remote on or off above leaves it signed in, and **Sign out** under Other
computers leaves this computer's host as it is. Its sign-in is kept in the same credential store. The
other computer lists it among its devices let in, where it can be revoked. A pairing it revoked or
refused stays under **Pairings**, saying so, until you press **Forget**. **Forget** on one still in
force takes it back from this side. Either way it is no longer listed here; pairing with that computer
again makes a new pairing, which shows.

## Syncing projects

Your computers can share a project through the server: two computers with Remote on, signed in to
the same account on the same server. That is the project itself, its name and its remote; its
[approved notes](memory.md#on-your-other-computers), which reach every computer's agents; and its
workspaces, which every other computer shows and never makes. Nothing syncs until you turn it on for
a project, and **never its files, its path or its terminals**.

Open **Project settings…** from the project's menu. Its **Sync** section shows what turning sync on
sends: the project's name, the address of its Git remote (`origin`) without any user or password in
it, or none for a remote that is a folder, and how many approved notes and workspaces go with it. **The server
keeps synced records in plain text, readable by whoever runs it.** Press **Turn on sync**. The section says _Waiting to reach the
server…_ until the server has it, then _Syncing with your other computers_, and when it last
synced. With Remote off, the section says sync needs it and links to **Settings → Remote**.

**On your other computer**, add the same repository as a project, a clone of it say, open its
**Project settings…** and press **Link to a synced project…**. It lists the projects your other
computers sync that this one does not, each with its remote; one whose remote is this project's
is marked **Same remote** and listed first. Nothing is chosen for you: pick the one this project
is and press **Link**. Link only to the same repository, since what syncs of it will be shared
with this one: the notes synced from there arrive here, and this project's own approved notes go
there.

Yardsort syncs while it runs with Remote on: right after a change here, when the server says
another computer changed something, and every five minutes otherwise. When the server cannot be
reached, it tries again by itself, waiting longer each time, up to five minutes, and the section
says why the last attempt failed. When the server refuses something, the section says why, and it
is not sent again.

**Your other computers' workspaces** of the project are listed under it in the sidebar, folded into
**on other computers (n)**: each with its name, the computer it is on, its branch and the task it was
started from, and _archived_ when it is. They are only a list. Nothing of them is on this computer:
no worktree, no branch, no folder and no terminal is made for one, and there is nothing here to open
or delete, except **Check out here**, which fetches its branch from the project's remote and makes a
[workspace of this computer's own](workspaces.md#workspaces-on-your-other-computers) on it. Each
changes when it changes on its computer, which alone writes it: made, renamed,
archived, restored or deleted there. A workspace you make here, in the app or with `ys`, shows on
your other computers within seconds. The project's own checkout is not listed, since each computer
has its own.

- **Stop syncing** unlinks this computer. The project stays here as it is, its notes included, and
  your other computers keep syncing it.
- **Remove from the server…** asks first, then removes the project's records from the server for
  every computer. Each keeps its own copy of the project, and stops syncing it.
- **Turning Remote off**, or moving this computer to another server, stops every project syncing
  here. What is on the server stays for your other computers; turn sync on or link again
  afterwards.

On a computer with no screen, `yardsort-host sync` does the same: see
[Syncing projects there](headless-host.md#syncing-projects). [`ys sync status`](cli.md#ys-sync-status)
says what syncs, and nothing more: no agent can turn sync on, link, stop or remove a project.

## If a phone cannot open any terminal

The phone may say that this computer's terminals run in a process from an earlier version of Yardsort.
That process, the [daemon](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window), is
what keeps your agents running when Yardsort closes, so updating Yardsort leaves it running, and new
terminals start in it too. One from before 0.21.0 cannot share terminals, however recently they started.

**Settings → Remote** says when this is so. **Stop everything and restart…** names what it stops and asks
again, then stops every terminal and restarts Yardsort, and a current daemon takes over. Files stay on
disk, and conversations can be [resumed](terminals-and-sessions.md#resume). If you would rather finish
what is running first, Yardsort does the same by itself when it next starts with nothing running. It only
has to happen once: as long as a daemon can share terminals, later updates keep that working.

## If this computer is signed out

If the server signs this computer out, **Settings → Remote** says so and offers **Sign in again**. It
stays the same computer on your phone, and your devices stay paired. **Turn off** is for when you want
Remote off: your devices have to pair again afterwards, though they will still find one computer.

The usual reason is the server seeing this computer's sign-in used twice, which it treats as stolen. A
second copy of Yardsort used to cause that. Now only one copy runs on a profile, and starting it again
[brings up the window it already has](troubleshooting.md#yardsort-is-already-open).

## The phone app

[The phone app](phone.md) says what it does and what it needs. It is invite only for now, and will
be in the app stores in a few weeks. Its code is in
[`apps/mobile`](https://github.com/joaoh82/yardsort/tree/main/apps/mobile). To run a server of your own, see the
[self-host recipe](https://github.com/joaoh82/yardsort/tree/main/crates/server/self-host).

## Notifications

With Remote on, this computer can tell your phone when something needs a look.

- A task moved forward, or finished.
- A review needs attention: checks failed, it conflicts, changes were requested, or it has no review yet.
- A workflow run failed.
- A newer desktop version is available. The same version can be mentioned again after a week.
  Installing it stays on this computer.
- A workflow is waiting for a question or an approval. Open it on the phone to answer or
  approve, while this computer is online. The action is not sent, and is not sent later, when
  this computer is offline.
- An agent needs you: it asked for a permission, such as Claude Code wanting to run a command.
  Open it on the phone and it opens that conversation, where Claude Code's question can be
  answered in Chat ([When an agent asks for a permission](#when-an-agent-asks-for-a-permission)),
  and any agent's in its terminal.
  If the agent asks again before anyone answers, that is the same notification, not a new one.
  Claude Code, OpenCode and Pi say when they ask; the notification says nothing of what was
  asked. A computer with no screen, running `yardsort-host`, sends it too.

The notification opens that item in the phone app. A workflow question or approval opens the
decision. Installing a desktop update stays on this computer. A one-time code can still appear
in a notification here when Settings → General includes it. It is never in the one the phone
receives. The phone asks this computer for the code when you decide, over the connection you
signed in on, and it does not show the code.

A phone paired before this needs to be unpaired and paired again. Letting it watch includes
reading workflow runs when the phone asked to see them. Approving or answering is the checkbox
**and approve or answer workflows**, shown only when the phone asked for that, or a separate yes
on a computer with no screen, asked only then. Being allowed to type in a terminal does not
include it. If the step changed after the phone showed it, the phone says to refresh and reads
the run again. That tap does not use up a wrong code. An older desktop is not offered those
workflow permissions until it states that it speaks them.

On the phone, Settings → Notifications asks once for permission to send notifications. After
that, the token is registered for every account this phone is signed in to, when the app starts
and after a sign-in. The screen can
mute everything for this account, or turn one of those categories off. **Agents that need
you** shows only once the server knows that kind of notification. A tap opens the item,
including when the app was not running. A notice that was skipped while a category was off is not
sent later if you turn it back on. A new event after that is.

A task or review notice is sent when this computer loads that list. It is not sent on a timer of
its own. Opening a list of tasks that were already finished, or a pull request this computer has
not shown before, does not notify. A change after that does.

If this computer cannot reach the server, or the phone cannot be reached, the notice waits up to a
day and is delivered once when the path is open again. The same event from two computers is one
notification. A long wait does not arrive as a burst: only the newest few are sent.

The server can do this without Apple or Google push credentials. Notices wait until those are set.
What to set is in [Mobile notifications](../design/40-mobile-notifications.md#what-to-configure).

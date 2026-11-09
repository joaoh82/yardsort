# Remote

> **Remote is still under development, and will be until the whole plan for it is finished.** Expect it
> to change. It stays off, and connects to nothing, unless you turn it on. The phone app is not in the
> app stores yet.

Remote lets your phone reach the agents on this computer through a Yardsort server you sign in to. Look
at a workspace, watch a terminal, or take control of one and type, away from your desk. A computer with no
screen, such as a server you reach over SSH, can be a host too: see
[A computer with no screen](headless-host.md). A phone that may
type can also start a new workspace or another agent, and give an agent a picture. The agents keep
running on this computer whatever happens to the connection. This computer can also reach another one
the same way, watch its terminals and type into them: see
[Reaching another computer](#reaching-another-computer).

Neither end listens for connections: this computer and your phone each reach out to the server.

## What the server can see

**Whoever runs the server can see what passes through it**, your terminals and what you type into them
included. The connection is encrypted on the way, but not end to end. Use a server you run yourself, or
one run by someone you trust with that.

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

## Starting work from the phone

A device you let type can start work here too. It is no more than it could do by typing into a terminal,
so it needs nothing else from you.

- **New workspace**, on a computer's screen, asks for the project, the agent and what it should do. This
  computer makes the workspace as its [composer](workspaces.md#starting-one-the-composer) does: a branch
  named after the message, from the project's default branch, with the project's setup run, and the
  agent given the message. The model and effort are the agent's own defaults. Its
  terminal then opens on the phone. Setup that takes a while keeps the phone waiting; it carries on here
  whatever happens to the connection.
- **New agent**, in a workspace, starts another agent there, with a first message or none.

The phone offers only the agents this computer has turned on in **Settings → Harnesses** and can find,
the first of them chosen to begin with, and each starts as those settings say. The workspace and its
tab appear in this window at once.

## Giving an agent a picture

While a phone has control of a terminal, **Image**, at the start of its row of keys, lets you choose a
picture. The phone keeps control while its photo picker is open, though leaving the app otherwise
lets go of it. The phone sends it as a JPEG, no larger than 2048 pixels on its longest side, and this computer
keeps it in the profile's `uploads` folder, outside every worktree, so no branch picks it up. Its path
is then pasted into the terminal, as dropping a file on a terminal here does, with nothing after it, so
you can go on and type the message. Claude Code, for one, reads a pasted path to a picture as the
picture. Pictures are deleted after seven days, when another one comes.

## Scrolling on the phone

Drag down on a terminal to go back through what scrolled off its screen, as far as this computer keeps
it, and up to come back. A terminal taller than the phone moves first; the history follows on in the same
drag. A program on its alternate screen, as a full-screen program uses, has no history to scroll.

## Reaching another computer

The desktop app can reach another computer running Yardsort too, as your phone does: one with the app
and Remote on, or one with no screen running [`yardsort-host`](headless-host.md). It shows that
computer's workspaces, agents and activity, opens its terminals to watch, and types into one once you
take control of it. It does not start work there yet.

1. In **Settings → Remote**, under **Other computers**, give the server the other computer is signed in
   to and a name for this one, then press **Sign in to reach other computers**, which opens your
   browser. The server is filled in for you when this computer is a host on one.
2. On the other computer, show a code: **Pair a device** in its **Settings → Remote**, or
   `yardsort-host pair` on one with no screen.
3. Type the code under **Pairing code** here and press **Pair**. This computer asks to watch and to
   type, as a phone does. The other computer's person is asked there, and lets it watch, watch and
   type, or refuses. Until then the pairing says _waiting for its answer_.

Once let in, the other computer is listed under **Other computers** in the sidebar, below your projects,
by its name: _online_, _connecting_, or _offline_ since when. Click it to open its page in the middle of
the window, with its name across the top. It lists each of its projects' workspaces with their branches,
the agents in each with what they are doing, the workspace's other terminals, and its recent activity,
and updates as it changes. There is no Changes panel, no Git action and no file for another computer's
workspace, and nothing of it is mixed into your own projects. While the other computer is away, the page
keeps what it last showed, and says when that was.

**Its terminals.** Press **Watch** beside an agent or a terminal to open its terminal there, with **Back** to the computer's
page. It shows what that terminal shows, at its size on that computer, and says who has control: nobody,
the person at that computer, or a device by its name. Watching takes nothing, and what you type goes
nowhere. Press **Take control** to type into it: the terminal then fits this window, and the person at
that computer sees who has it. **Release control** hands it back, and so does going **Back**. Control is
taken only by pressing; if the person at that computer types, or a phone takes control, this window says
who took it and offers **Take control** again. A computer that lets this one only watch offers no
**Take control**, and says so; to change that, **Forget** it under **Pairings**, pair again, and let it
type there. If the connection drops, the terminal is watched again by itself once the other computer
is back, from where it was, and control has to be taken again. Something typed as it dropped may not
have arrived, and the window says so: nothing is sent twice.

**This is a device of its own.** Signing in here makes a second device at the server, apart from this
computer as a host: turning Remote on or off above leaves it signed in, and **Sign out** under Other
computers leaves this computer's host as it is. Its sign-in is kept in the same credential store. The
other computer lists it among its devices let in, where it can be revoked. A pairing it revoked or
refused stays under **Pairings**, saying so, until you press **Forget**. **Forget** on one still in
force takes it back from this side. Either way it is no longer listed here; pairing with that computer
again makes a new pairing, which shows.

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

It is in [`apps/mobile`](https://github.com/joaoh82/yardsort/tree/main/apps/mobile), and is not in the
app stores yet. To run a server of your own, see the
[self-host recipe](https://github.com/joaoh82/yardsort/tree/main/crates/server/self-host).

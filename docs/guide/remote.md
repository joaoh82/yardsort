# Remote

> **Remote is still under development, and will be until the whole plan for it is finished.** Expect it
> to change. It stays off, and connects to nothing, unless you turn it on. The phone app is not in the
> app stores yet.

Remote lets your phone reach the agents on this computer through a Yardsort server you sign in to. Look
at a workspace, watch a terminal, or take control of one and type, away from your desk. The agents keep
running on this computer whatever happens to the connection, and the phone never starts them.

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
one is waiting. Nothing but this window can say yes: not `ys`, and not an agent.

**Devices let in** lists them, and you can take any one back there at any time.

## Who has control

Any number of devices can watch a terminal, but only one can type into it at a time. The phone asks for
control, and the person at the computer can take it back at any moment just by typing.

## The phone app

It is in [`apps/mobile`](https://github.com/joaoh82/yardsort/tree/main/apps/mobile), and is not in the
app stores yet. To run a server of your own, see the
[self-host recipe](https://github.com/joaoh82/yardsort/tree/main/crates/server/self-host).

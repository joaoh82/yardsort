# Other computers, like local projects

_Proposed in Figma on 2026-11-14 and approved by the owner the same day, with the decisions in
section 6. Slice 1 was built the same day (section 8); slices 2 to 5 are not built. The frames are on the page "Other computers like local" of the
desktop file (<https://www.figma.com/design/HXSYFYmKvlzr1IXp0hBEh3?node-id=15-841>): A1 a workspace
on another computer, A2 and A3 the composer, A4 the states, and a Notes frame, which was written
before the decisions and is superseded by this document where they differ._

This is the plan for the desktop as a client of other computers, after its first version
([35](35-remote-r4-r5.md) §13 to §16): what it has to show, what it starts from, the decisions, the
slices in the order they are built, and what each slice's smoke test needs. Like 35 and
[42](42-remote-r6.md), it becomes the record of the slices as they are built.

## 1 · Why

The desktop reaches other computers since 0.30.0, and it works: it watches their terminals, takes
control, starts workspaces and agents there and resumes conversations. But it is a different app
inside the app. Another computer is one row at the bottom of the sidebar. It opens a page of its own
with an amber banner, where workspaces are cards, a terminal is opened with **Watch** and left with
**Back**, and work is started from a plain form. The composer, where every local task starts, knows
nothing of it.

The owner's feedback on 2026-11-14: work on another computer looks, and is, completely different
from local work. It should look like the local projects. The account's computers should be listed
under their own heading and fold, in case there are many, with their projects, `local` and
workspaces under each. And the composer should offer a connected computer to run the task on.

## 2 · What it has to show

A person can:

- see every computer this one is paired with in the sidebar, fold and unfold each, and find its
  projects and their workspaces under it, drawn as local ones are;
- open a workspace there and get the window a local workspace gets: its terminals as tabs, that
  computer's agents to start, its ended conversations with **Resume**;
- type into a terminal there once this computer has control, and always see who has it;
- start a task from the composer on **This computer** or on a computer that is online and lets this
  one start work, and land in the new workspace there;
- tell at a glance, everywhere, which computer a workspace is on.

- review what an agent there changed, commit it, push it and open its pull request, from the same
  right panel a local workspace has.

**Not in it:** handing a running task to another computer and managed sandboxes (35 §9); workflows,
smoke tests, schedules and triggers on another computer; editing a file there in the built-in
editor, and the conflict helpers.

## 3 · Where it starts

Checked against `main` at `1b20247` on 2026-11-14.

**What the core already gives the window.** The desktop's client (`remote_host::computer`) keeps one
link per approved host and reads what each shows when its activity moves and every 20 s
(35 §13). The window gets it as `OtherHostView` (`src-tauri/src/computers.rs`): projects, their
workspaces with name, branch and archived, each workspace's sessions (harness, title, state,
terminal, resumable) and other terminals, and `mayType`, `mayStart`, `mayResume`. It asks for one
host at a time (`other_host`), only for a host the window has open. The protocol's workspace
summary also carries the kind (`local` or a worktree) and the task links; the window's
`OtherWorkspace` drops both.

**What a start can say.** `workspace.start` takes a project, a message and a harness by id;
`session.start` a workspace, a harness and a message; `session.resume` a conversation and a
message (`remote-proto/src/start.rs`). The host names the branch from the message and starts from
the project's default branch. `harness.list` gives ids and names. Nothing carries a model, an
effort, a name, a branch to start from or to open, or a list of branches. Nothing starts a shell,
or renames, archives or deletes a workspace. Protocol minor 9.

**What does not cross the relay at all.** Paths, changes, files, Git, pull requests, a project's Run
command, workflows and smoke tests (35 §3, "A remote path is never treated as local").

**The window.** `useProjectsStore.remoteHostId` gives the center panel to `OtherHostView`
(`src/features/remote/`), which holds its own state for the terminal it watches and the form it
shows. A remote terminal is a `TerminalSurface` behind a `TerminalSource`, as a local one is
(35 §14); `RemoteTerminal` adds **Back** and the control buttons. `TerminalTabs` and
`useTerminalStore` are the local daemon's only, keyed by its session ids. `ProjectTree` and its rows
take the local `Project` and `Workspace`, and their menus are local actions. `Composer` takes a
local project. The sidebar keeps which projects are folded in the core's UI store
(`sidebar.collapsedProjects`), and the composer keeps its last picks there too.

**Its smoke tests.** `desktop-client.yaml`, `same-session.yaml` and `remote-start-and-resume.yaml`
drive the host page by its labels and its **Watch** and **Close** buttons
(`.yardsort/tests/support/window.ts`). They change with it.

## 4 · Decisions

**A tree under its own heading.** **Other computers** stays a section of the sidebar, below
Projects, and becomes a tree: a computer, its projects, their workspaces. A computer's row has its
name and how it stands: online, connecting, offline and for how long, waiting for approval there,
or watch only. It folds like a project, and which are folded is kept where the projects' is. An
offline computer is folded by itself; opened, it shows what it last showed, dimmed. The filter
above Projects filters this tree too. The **+** on the heading opens Settings → Remote → Other
computers, where pairing is.

**The same rows.** A project and a workspace there are drawn by the rows the local tree uses: the
status dot, the busy clock and the agents' icons from that computer's terminals, `local` with its
branch, the task badge. What a row cannot have yet is left out, not greyed: the pull request badge,
and the menu entries that act on this computer's disk. A row's menu holds what works there: **New
workspace** on a project, **New agent** on a workspace.

**The same panel.** A workspace there is a selection like a local one, and the center panel is the
one a local workspace gets: tabs for that workspace's agents and terminals, the buttons that start
one of that computer's agents, the terminal, the bar that offers **Resume** when a conversation
ended, the history of ended ones when nothing runs. The host's own page, its banner, **Watch**,
**Back** and the start form are removed.

**The computer is always named, in three places and no banner.** Its row in the tree; the bar above
the tabs, which says the computer, how it stands and who has control; and the first part of the
bottom line, where a local workspace shows its path. 35 §3's rules hold: nothing of another
computer is mixed into Projects, and a path there is never treated as local.

**Control is said, not hidden.** One device types into a terminal there at a time, and the person at
that computer must be able to see who (35 §3, §14). So the bar says who has control and offers
**Take control** or **Release**; typing alone still never takes it. One change, decided by the owner: **an
agent started from this computer starts with this computer in control.** Starting it was already
this person's act under the same grant, and it is the case that made the first version feel most
foreign: start an agent, then press a button to talk to it. An agent that was already running there
takes one press, or the shortcut. Watching draws the terminal at that computer's size; in control,
it takes this window's.

**What only that computer can do is not shown, rather than shown and failing.** Reveal in the file
manager, open in an editor, Run, a new shell, hand-off, rename, archive and delete, workflows,
smoke tests, schedules and triggers do not appear on a workspace there. Slice 3 brings the ones the
protocol can carry by ids.

**The composer says where.** Its title becomes "New workspace in _project_ on **This computer**",
and the last part is a menu: this computer, then every paired computer with how it stands. One that
is offline, or lets this one only watch, is listed and cannot be chosen. Nothing is ever queued for
a computer that is away. Choosing a computer swaps the agent list for that computer's, and with
slice 3 the model, effort and branch lists too. Start makes the workspace there, selects it in that
computer's tree and opens it as any workspace there. The **+** on a project in a computer's tree
opens the same composer with that computer chosen.

**Which project there: the same one, shown before the start, and cloned there when it is missing.**
A local project and one on another computer are separate records (35 §3). The composer shows
"Project on _computer_" with that computer's projects, and offers the one that is the same
repository: by its fetch remote once both sides can compare it (slice 3), by its name before that.
The person sees it, can change it, and the choice is remembered on this computer. This is the
narrower mapping 35 §9 left open. Once sync links projects across computers (42, slice 2), a link
decides.

When that computer has no such project, the composer offers **Clone it there** (decided by the owner
on 2026-11-14): Start first has that computer clone the repository into its projects folder and add
it as a project, then makes the workspace in it. What goes over is this project's fetch remote
with any user and password removed, as sync's hint is (42 §3), carried and never kept by the
server. That computer clones with its own Git credentials and into a folder of its own choosing:
a setting there, never a path from here. A clone that fails says why in the composer and leaves
nothing behind, as a local clone does. It needs the protocol step of slice 3; until a computer has
it, a missing project there is said to be missing, with the way to add it.

**What a start there does not take from here.** The new workspace's triggers, the project's memory
and its setup are that computer's own, by its settings. The composer says so in place of those
controls.

**The right panel keeps its place.** For a workspace there it says that changes and files stay on
that computer, until slice 4 fills it (decided by the owner: it stays, it does not fold away).

**Changes, files and Git there come under the control a pairing already has.** Proposed, following
35 §16: a device that may type into a terminal there can already read any file and run any Git
command through it, so reading changes and committing are offered to a pairing with
`terminal:control`, with no scope of their own and no pairing again. A pairing that only watches
gets none of it. Everything names a workspace by its id and a file by its path inside that
workspace, checked there; nothing outside a workspace is read or written. This is the one decision
here the owner has not been asked directly; it is theirs to overturn before slice 4.

**Released together.** Decided by the owner: nothing of this is released until all five slices are
built. Proposed: each slice is a pull request stacked on the one before, and they merge together,
so `main` keeps releasing meanwhile.

**The frontend still holds no truth.** The tree is drawn from what the core read. The window asks for
all paired computers at once instead of one, and the core's reading does not change.

## 5 · Slices

Each slice ends with its tests, its guide and changelog lines, and a local smoke test against a
`yardsort-host` on the stand-in server, as 35's did. None changes the server, so staging needs no
deploy. Slices 1 and 2 change no protocol and no host. Slices 3 to 5 each add to the protocol, so
they need the other computer on the release that carries them; an older one keeps what it had, and
the window says what it lacks rather than offering it.

| #   | Slice                                              | What it adds                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Its smoke test                                                                                                                                                                                                                                                                |
| --- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The tree and the workspace there                   | The window reads every paired computer; `OtherWorkspace` gains its kind and tasks. The sidebar's tree with the local rows. A workspace there as a selection, in the workspace panel: the computer bar with control, tabs from that computer's sessions and terminals, its agents to start, Resume, the bottom line. Control taken at once for an agent started here. The host page removed. The filter, the keyboard and the shortcuts cover the tree                                                                                   | `desktop-client.yaml`, `same-session.yaml` and `remote-start-and-resume.yaml` rewritten for the tree: the host under Other computers, unfolded to its workspace, its shell in a tab, control taken and lost, an agent started from the tab bar and resumed                    |
| 2   | Run on, in the composer                            | The menu in the composer's title, each computer's state, the project there offered by name and remembered, that computer's agents. Start through `workspace.start`, landing in the new workspace. The model, effort, name and branch controls give way to a line saying that computer decides them                                                                                                                                                                                                                                      | `run-on.yaml`: from a local project's composer, choose the host, see its project offered, start; the workspace appears under the host and its agent hears the message. A host with no project of that name: said to be missing. An offline host: listed, not offered          |
| 3   | The rest of a start, clone there, and the row menu | Protocol minor 10. A name, a model, an effort and a branch to start from or to open, on `workspace.start`; models and efforts in `harness.list`; a project's branches, and its fetch remote without credentials in its summary. `project.clone`: a repository by its address, into that computer's projects folder (a new setting there, in the window and in `settings.toml`), added as a project. A shell in a workspace. Rename, archive and delete, with the second confirmation for uncommitted work asked there and answered here | `run-on-full.yaml`: each new field reaches a stand-in agent on the host; a project the host lacks is cloned there from a local bare repository and the workspace made in it; a clone that fails leaves nothing; a delete with uncommitted work refused without the second yes |
| 4   | Changes and files there, read-only                 | Protocol minor 11, under `terminal:control`: a workspace's changed files with their counts (`workspaceChanges` there), a file's diff, the file tree and a file's text, each bounded in size and refused outside the workspace. The right panel's Changes and Files tabs filled from them, refreshed when that computer's activity moves, with no commit box and no editor                                                                                                                                                               | `changes-there.yaml`: an agent stand-in on the host changes a file; the window lists it and shows its diff; a path outside the workspace is refused; a pairing that only watches is offered none of it                                                                        |
| 5   | Commit, push and pull requests there               | Protocol minor 12, under `terminal:control`: commit with a message, push, the workspace's publish state, open a pull request and merge it, each run by that computer with its own Git and forge sign-ins. The commit box, the publish bar and the pull request toolbar for a workspace there, and the pull request badge on its row                                                                                                                                                                                                     | `publish-there.yaml`: commit and push from the window to a local bare remote of the host's repository; the stand-in `gh` there answers for the pull request; a push the remote refuses says why                                                                               |

Slice 2 depends on 1, 3 on 2, 4 on 1, 5 on 4. The guides that change: `docs/guide/remote.md`,
`workspaces.md`, `projects.md`, `changes-and-files.md` and `commits-and-pull-requests.md`, and
`headless-host.md` for the projects folder and for what a paired desktop can do.

## 6 · Decided

By the owner on 2026-11-14, after the frames:

1. **Control of an agent started from this computer** is taken at once. An agent already running
   there takes one press.
2. **Which project there:** the same one, offered and shown; when that computer does not have it,
   it is cloned there into its projects folder and the workspace made in it (section 4).
3. **The right panel** stays for a workspace there, with its line, until slice 4 fills it.
4. **The heading** is **Other computers**.
5. **Released** only when all five slices are built.

Proposed by this plan and not yet put to the owner: changes, files and Git there under
`terminal:control` with no new scope (section 4); the projects folder's default on a computer that
has not set one, `~/Projects`, made if it is missing; and stacked pull requests merged together.

## 7 · Risks

- **Taking control at a start surprises the person at that computer.** The bar there and here says
  who has it, and they take it back by typing, as now.
- **The same name is not the same repository.** Until slice 3 compares the fetch remote, the project
  there is offered by name, shown before Start and never picked silently.
- **A clone there can be large, slow, or refused.** It runs with that computer's credentials, so a
  private repository it cannot read fails there, and the composer says so in that computer's words.
  The composer shows that it is cloning, and the start is not tried again by itself.
- **File content crosses the relay from slice 4.** It is carried and never kept, as terminal output
  is; diffs and files are bounded in size, and the leak audit of 35 §19 extends to them.
- **A commit or a push from here acts on that computer's repository.** Each is the person's press,
  none is offered to an agent, and nothing is discarded or deleted without the second confirmation
  the local panel asks.
- **A long tree.** Many computers with many projects make the sidebar long. Computers fold, offline
  ones by themselves, and the filter covers the tree.
- **Three smoke tests are rewritten in slice 1.** They are the record of R5's gate. Their checks
  stay, driven through the tree.

## 8 · Slice 1: the tree and the workspace there

**Built on 2026-11-14**, on Linux. Nothing changed in the protocol, the server, the host or the
phone, so it works with every host from 0.30.0 and staging needs no deploy.

**What it adds.**

- **What the window is told.** `OtherWorkspace` gains its kind, whether its folder is missing, and
  its tasks, all of which the protocol already carried (`src-tauri/src/computers.rs`). The window
  reads every computer it is let into, not only one that is open, on the same change event
  (`src/stores/otherComputers.ts`). The core's reading is unchanged.
- **The tree** (`src/features/sidebar/OtherComputersTree.tsx`): a computer, its projects, their
  workspaces. A computer's row says how it stands; it and its projects fold, kept in the core's UI
  store under `sidebar.computers` and `sidebar.computerProjects`; an offline one is folded until
  opened, and dimmed when open. A workspace's row has the status dot, `local`'s branch, the count
  of open agents and the task's key, which opens the task's page when its address is `https`. The
  **+** on a project composes a workspace there; a workspace's menu starts one of that computer's
  agents. The sidebar's search filters it by a computer's name or a project's.
- **A workspace there as a selection** (`useProjectsStore.remote`), kept above the local selection
  as every other view of the center panel is, and `composingThere` for a new one.
- **The workspace panel for it** (`src/features/remote/RemoteWorkspaceView.tsx`): the bar with the
  computer, how it stands and who has control; tabs for its agents' terminals and its other
  terminals; that computer's agents at the end of the tab bar; **Resume** over an agent's terminal
  that ended, and under Previous sessions when nothing runs; the bottom line with the computer
  first and no path. `RemoteTerminal` lost **Back** and its own header.
- **Control at a start.** What `otherHostStartWorkspace`, `otherHostStartSession` and
  `otherHostResume` start is put in front at once, before that computer's next reading lists it,
  and its control is asked for as soon as its terminal says who has it. Once: refused or taken
  back, it is **Take control** from then on. Its tab is held as "starting…" only until a reading
  lists it, or until one taken more than 30 seconds after the start does not; from then on it is
  what that computer lists, and gone when it is no longer listed.
- **Composing there** (`src/features/remote/RemoteComposer.tsx`): the composer's look for a new
  workspace on another computer, with that computer's agents and the message. Slice 2 puts the
  choice of computer in its title and in the local composer's.
- **The keyboard.** Next and previous workspace go on from this computer's to the others', next and
  previous tab switch a workspace's tabs there, and new workspace composes there from a workspace
  there. New shell and close tab are not offered there, so they cannot act on the local selection
  hidden beneath.
- **Removed:** `OtherHostView`, its banner, **Watch**, **Back**, **Close** and its start form, and
  the list of that computer's recent activity, which nothing else in the window shows for a
  computer as a whole.

**What building it settled, beyond section 4.**

- **No busy clock on a row there.** The protocol says how long a terminal has been quiet, not since
  when it has been busy, so the row has the dot and not the clock the frames draw. Slice 3 can
  carry it.
- **No shortcut takes control.** The frames' note mentions one; a press is the only way, as before.
- **A phone has to take control over** from a desktop that started an agent, as from any device
  holding it. `remote-start-and-resume.yaml`'s pretend phone now does.
- **The right panel** says "Changes and files stay on _computer_", and nothing more.

**Its tests.** The tree, the workspace panel, control with and without a press, starting and
resuming, and composing there (`src/features/remote/OtherComputers.test.tsx`); the selection in the
center panel (`projects.test.ts`); the shortcuts (`Keyboard.test.tsx`).

**Its smoke tests.** `desktop-client.yaml`, `same-session.yaml` and `remote-start-and-resume.yaml`,
with their window steps rewritten for the tree (`.yardsort/tests/support/window.ts`).

**Run on 2026-11-15**, on Linux, each against a `yardsort-host` on the stand-in server with the
real window on a profile of its own.

| Test                      | Result                                | What it showed                                                                                                                                                                                      |
| ------------------------- | ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `desktop-client`          | passed, 16 of 16 steps (run b5d7cbf6) | The host under Other computers, its row saying `watch only`, its project and workspace under it; the workspace in the center panel with nothing that acts; the right panel's note.                  |
| `same-session`            | passed, 21 of 21 steps (run 0d31a5ed) | The workspace opened from the tree with its shell watched; control only after **Take control**; the phone taking it over, and the window saying so.                                                 |
| `remote-start-and-resume` | passed, 25 of 25 steps (run c452d086) | A workspace composed from the **+** on the host's project, this computer in control without a press; the phone taking over, resuming and ending it; **Resume** from the window, in control at once. |

What the runs found before they passed:

- **A watch-only row says `watch only`, not `online`.** The first `desktop-client` run (8ff423c3)
  failed in its own helper, which waited for `online`. The helper now takes either, and that test
  checks the row says `watch only`.
- **A tab held as "starting…" for good.** The first `remote-start-and-resume` run (39ccb73c)
  failed at the window's **Resume**: the agent it had started ended, the phone resumed it in a new
  terminal and ended that, and the window still held the first terminal as a "starting…" tab,
  which hid Previous sessions and its **Resume**. A terminal started here was remembered for as
  long as the window stayed open. It is now held only until that computer's readings settle it
  (above), with two tests in `OtherComputers.test.tsx` that fail without the change.

Not run: macOS and Windows, since these three tests are Linux only (the window stands in for the
browser with `xdg-open`), and none of it on a second real computer.

## 9 · Slice 2: Run on, in the composer

**Built on 2026-11-15**, on Linux. No change to the protocol, the server, the host or the phone.

**What it adds.**

- **Run on** (`src/features/remote/RunOn.tsx`): the last part of the composer's title, a menu of
  this computer and every paired one with how it stands. Online and able to start is offered;
  offline, connecting, not read yet, watch only and too old to start are listed with the reason and
  disabled (`whyNotThere`). **Pair another computer…** opens Settings → Remote.
- **In the local composer** (`src/features/composer/Composer.tsx`): shown once this computer is
  signed in to reach others, and only for a plain new workspace. Choosing a computer goes to the
  composer for it with the message typed so far.
- **In the composer for another computer** (`RemoteComposer`): the same menu, with that computer
  chosen. **This computer** goes back to the project it was opened in, or to one here of the same
  name when it was opened from the tree; with neither, it is listed and disabled. Another computer
  is chosen the same way. The message goes along each time.
- **Project on _computer_**: that computer's projects, with the one offered for the project here
  chosen (`offeredThere` in `there.ts`): the one chosen last time for it, if that computer still
  has it, or else the one of the same name, exactly and then without regard to case. The line beside
  it says which: "same name as here", "as you chose last time", or only that it will be remembered.
  The choice is kept on this computer under `composer.projectThere`, by that computer's id and
  this project's, and only once a start succeeded.
- **A project that computer lacks** is said to be missing, with how to add it there; Start waits
  until one is chosen. Slice 3 puts **Clone it there** in that place.
- `useProjectsStore.composingThere` gained `from` (this computer's project) and `message`, and
  its `projectId` can be `null`.

**What building it settled.**

- **The menu is not there without Remote.** A computer that is not signed in to reach others shows
  the composer's title as before, so nothing changes for someone who does not use Remote.
- **Only a plain new workspace can move.** A fork, a task, a pull request's branch and a run in a
  workspace start from records and files of this computer, which a start there cannot carry. Their
  composers have no menu.
- **What does not go along.** The model, effort, branch, name and ticked presets picked here are
  left behind when another computer is chosen, since a start there cannot say them until slice 3.
  They come back from the last picks when this computer is chosen again.

**Its tests.** The menu, the disabled rows and the hand-over in `Composer.test.tsx`; the project
offered, changed, remembered and missing, and the way back, in `OtherComputers.test.tsx`.

**Its smoke test.** `run-on.yaml`, new, with the window steps `runs-on`, `missing-there` and
`not-offered-away` (`.yardsort/tests/support/window.ts`).

**Run on 2026-11-15**, on Linux, against a `yardsort-host` on the stand-in server with the real
window on a profile of its own: passed, 24 of 24 steps (run 52804dbb).

- From the composer of this computer's own `repo`, the host was chosen in the title's menu; what
  was typed came along; the host's `repo` was offered as the same name; Start made the workspace
  there, selected under the host in the sidebar, with this computer in control. A pretend phone saw
  the host's agent begin with the message. This computer's repository still had its one checkout,
  and the host's had the new worktree.
- From `notes`, which the host has no project for, the composer said so and Start waited.
- With the host stopped, its row in the menu said offline and was disabled.

`remote-start-and-resume.yaml` was run again on this build, since the composer for another computer
now has two lists and its step picked the agent by the first: passed, 25 of 25 (run 1e4abbc2).

What the runs found: nothing in the app. The first run of `run-on` (9427a522) failed in the test's
own setup: a step loaded the window again and then opened a second inspector connection beside
the first, which never answers. Loading again and looking are now two steps.

Not run: macOS and Windows, since the test is Linux only like the three before it, and none of it
on a second real computer.

## 10 · Slice 3: the rest of a start, clone there, and the row menu

**Built on 2026-11-15**, on Linux. Protocol minor 10 (`crates/remote-proto`): nothing changes on
the server, which carries it unread. Both ends need the release that has it; an older host keeps
answering what it answered, and the window offers the rest only to a host that lists
`project.branches` among what it answers (`OtherHostView.mayManage`).

**The protocol.**

- `workspace.start` may say `name`, `model`, `effort`, and `baseBranch` or `existingBranch`. A
  client sends them only to a host that reads them: an older one would start something other than
  what was chosen. An effort must be one the host lists for that agent; a branch must exist there.
- `harness.list` gives each agent's `models` and `efforts`. A project's summary gives `remote`.
- `project.branches`, `project.clone`, `shell.start`, `workspace.rename`, `workspace.archive` and
  `workspace.delete` (`remote_proto::manage`). All but the first need `terminal:control`.
- A new error, `uncommitted_work`, for archiving or deleting a workspace whose folder has work in
  no commit: the request is sent again with `force` only after the person was asked a second time.

**What the host does** (`crates/core/src/remote/start.rs`, answered in `serve/start.rs`, by the app
and by `yardsort-host` alike): each is the call its own window makes. `Workspaces::create_named`,
`open_branch`, `rename`, `archive`, `delete`; `Projects::clone_from`; the launcher's shell.

- **A clone goes where the host says.** `remote.projects_dir` in its settings, `~/Projects` when
  unset, made when first needed. In the window it is under Settings → Remote, **Projects cloned for
  a device**. A request names only an address, checked as one typed into the host's own Clone
  dialog is: HTTPS, `ssh://` or `user@host:path`, no credentials in it, nothing that reads as an
  option. The name is the last part of the address.
- **A grant to some projects cannot clone.** The new project would be none of those it names.
- **A project's remote is told as it is configured**, not as this computer's own `insteadOf`
  rewrites it, with any user and password of an HTTPS address removed, and only when another
  computer could clone it. A folder, `file://` and `git://` are not told.
- **Messages name no folder.** The core's own for a dirty worktree and a failed clone do, so a
  device is given the host's wording for each instead (`start_error`).

**What the window does.**

- **The composer there** (`RemoteComposer`) has Name, Model, Effort and Branch for a host that
  reads them, from that host's lists, and says so for one that does not.
- **The same repository before the same name.** `project_repository` reads this computer's
  project's `origin` and reduces it to host and path; `OtherProject.repository` is the same for
  what the host told. `offeredThere` offers the remembered choice, then the same repository, then
  the same name, and the composer says which.
- **Clone it there** is in the project list and in the line that says the project is missing, for
  a project here whose `origin` another computer could clone. Never chosen by itself. Start clones,
  then starts in the clone; a start that fails after the clone is tried again in it, and does not
  clone twice. Only the project's id goes from the window to the core, which reads the address.
- **A shell**: the **+** after a workspace's tabs there, **New shell** in its row's menu, and the
  new shell shortcut. It is this computer's to type into at once, as an agent started here is.
- **Rename, archive, delete** in a workspace's row menu, for a worktree. The questions are the
  local ones with the computer named; a refusal for uncommitted work asks the second one.
- **A host that is the app** tells its own window to read its projects again when a device
  changed them (`RemoteChangedProjects`).

**What building it settled.**

- **Start a shell is its own operation**, `shell.start`, answering as a start does with no harness,
  and not `session.start` with a special name.
- **The window cannot offer a clone it knows will be refused for the grant.** It does not know
  which projects a pairing reaches. The host refuses, in words, and the composer shows them.
- **Restoring an archived workspace is not offered from here.** The tree lists how many are
  archived; bringing one back is done at that computer. A later slice can carry it.
- **Closing a terminal there is not offered.** Its program ends there, or by what is typed into it.
- **The busy clock is still not on a row there.** It needs a field in a session's summary and was
  left out of this minor.
- **The phone app is unchanged.** Its client reads the new messages (`packages/remote-client`),
  and nothing in its screens uses them yet.

**Its tests.** The wire and its bounds in `remote-proto` (fixtures for every new message); the host
against real repositories, real terminals and a real clone in `crates/core/src/remote/serve/tests/
start.rs`; a desktop reaching a host through the real relay in `crates/server/tests/computer.rs`;
the composer, the clone, the menu and the questions in `OtherComputers.test.tsx`; the folder
setting in `RemoteSettings.test.tsx`.

**Its smoke test.** `run-on-full.yaml`, new, with the window steps `runs-on-saying`,
`clones-there`, `clone-fails-there`, `shell-there` and `remove-there`.

**Run on 2026-11-15**, on Linux, against a `yardsort-host` on the stand-in server with the real
window on a profile of its own: passed, 35 of 35 steps (run ce4adcb0).

- The composer said a name, a model, an effort and the branch `develop`, each chosen from the
  host's lists. A pretend phone saw the host's agent begin with the model and the effort, and the
  new branch was at the commit `develop` is at and the default branch is not.
- From `notes`, which the host had no project for, **Clone it there** had the host clone the
  repository into its own projects folder, from an address its git rewrites to a repository on
  this disk, and the workspace was made in the clone. This computer's repository kept its one
  checkout.
- From `gone`, whose repository is not there, the composer said the host could not clone it, kept
  the message, and nothing was left in the host's folder.
- The **+** in a workspace there opened the host's shell, with this computer in control.
- A workspace with a file in no commit was refused deleting without force (`remote_uncommitted`),
  its file still there; with force its folder went and its branch stayed. The window's two
  questions are the system's dialogs, which the test cannot press, so it asked the core as the
  window does once they are answered; the questions themselves are in the unit tests.
- The host's projects folder was still in its `settings.toml` after it signed in.

The four earlier tests were run again on this build and passed: `desktop-client` (16 of 16, run
4c11a662), `same-session` (21 of 21, 0ffb8986), `remote-start-and-resume` (25 of 25, d401366e) and
`run-on` (24 of 24, fdf73af0).

What the runs found: nothing in the app. The first run (312d4bf4) failed at the delete step in the
test itself, which waited on a promise the inspector does not wait for.

Not run: macOS and Windows, a second real computer, and a clone over a network with real
credentials.

## 11 · Slice 4: changes and files there, read-only

**Built on 2026-11-15**, on Linux. Protocol minor 11 (`remote_proto::look`), carried by the server
unread. The window offers it to a host that lists `file.read` (`OtherHostView.mayLook`).

**The protocol.** Three operations, all under `terminal:control`, as section 4 proposed and the
owner did not overturn:

- `workspace.changes`: the files changed and not committed, and those committed on the
  workspace's branch since its base, each with its kind and its counts. Cut to what a frame
  carries, the uncommitted first, with `more` when it was cut.
- `workspace.files`: one folder of the file tree, folders first, without `.git` and what Git
  ignores. A device has no way to ask for the ignored ones.
- `file.read`: a file's contents on disk, or on one side of a change (`working`, `uncommittedOld`,
  `committedOld`, `committedNew`), in pieces of at most 32 KB as base64, each asked for by where it
  starts. A reply says the whole file's size, so a client that sees it change starts again. Only
  text is sent: a file over 1 MB, a picture, any other binary and anything that is not a plain
  file is said to be so, with nothing carried.

**A path is inside the workspace or it is refused.** The protocol refuses one that is absolute,
climbs with `..`, starts with a drive or a `-`, or holds a control character: this computer's
client refuses it before it is sent, and a host refuses it before it looks. The host then resolves it as its own window does (`changes::resolve_inside`), which refuses
a link that leads out. Its refusals name no folder.

**What the host does.** The calls its own right panel makes: `Changes::list` and `Changes::diff`,
`read_working_file`, and `list_dir`, which moved from the app into the core
(`crates/core/src/file_tree.rs`) so `yardsort-host` answers it too.

**What the window does.**

- **`RemoteChangesPanel`** takes the right panel for a workspace there: the two tabs, the list
  drawn as the local one, the tree listed a folder at a time, and a viewer that is the local
  diff and code views without what writes. It reads when it opens and again whenever this
  computer reads that one again (`readAt`), what is open included.
- **The core puts a file together** from its pieces (`Channel::whole_file`) and hands the window
  the same `Content`, `FileDiff` and `ChangeSet` a local workspace gives, so the viewer does not
  know where a file came from.
- **Where it is not offered** (watch only, an older Yardsort there, away, not read yet) the panel
  keeps its place and says which.

**What building it settled.**

- **Pictures are not sent.** The local viewer shows them; over the relay a picture is said to be
  binary. They can come later as another state.
- **Assist's review, who wrote each file, and the file menu are not there.** They are this
  computer's own, about its own files.
- **The list stays mounted when a file opens.** A tree mounted again would fold and ask that
  computer for its folders again, so the viewer's panel folds away instead of being removed.
- **Nothing is cached.** A file is read when it is opened, and again at the next reading.

**Its tests.** The wire and its bounds in `remote-proto`; the host against a real repository, with
a file larger than a piece, a binary, an ignored file, a link that leads out and a device that only
watches, in `crates/core/src/remote/serve/tests/start.rs`; a desktop reading through the real relay
a file whose pieces end inside a character, in `crates/server/tests/computer.rs`; the panel in
`RemoteChangesPanel.test.tsx`.

**Its smoke test.** `changes-there.yaml`, new, with the window steps `reads-changes-there`,
`reads-files-there` and `outside-there`, and the pretend phone's `cannot-read`.

**Run on 2026-11-15**, on Linux, against a `yardsort-host` on the stand-in server with the real
window on a profile of its own: passed, 25 of 25 steps (run cab1e421).

- With the workspace open, the right panel listed the changed and the new file, opened the change
  and showed its new line, and offered nothing that writes. When the file changed again on the
  host, the open change followed at the next reading.
- The Files tab listed the tree and a folder when it was opened, without `.git` and the ignored
  file.
- A file inside the workspace was read. A path that climbs out was refused by this computer's own
  client before it was sent; a link in the workspace that leads out was refused by the host.
- A pretend phone let only watch was refused the changes, the tree and a file.

The five earlier tests were run again on this build and passed: `desktop-client` (16 of 16, run
62f45446), `same-session` (21 of 21, fe13ddbf), `remote-start-and-resume` (25 of 25, 803dd978),
`run-on` (24 of 24, 61141c6b) and `run-on-full` (35 of 35, 2810d6e2).

What the runs found: nothing in the app. The first run (9289927a) expected the host to refuse the
climbing path; this computer refuses it first, which is the stricter of the two, and the test and
the relay test now say so.

Not run: macOS and Windows, a second real computer, and a repository of real size.

## 12 · Slice 5: commit, push and pull requests there

**Built on 2026-11-15**, on Linux. Protocol minor 12 (`remote_proto::publish`), carried by the
server unread. The window offers it to a host that lists `workspace.publish`
(`OtherHostView.mayPublish`).

**The protocol.** Five operations, all under `terminal:control`:

- `workspace.publish`: where the workspace stands with its remote: its branch and base, who a
  commit would be by, whether there is a remote and whether the branch was pushed, how far ahead
  and behind, the subjects of what is unpushed, and whether a pull request can be opened. With
  `forge`, the host asks the forge and says the branch's pull request: its number, title, address,
  state, checks, the commit at its head and whether it conflicts. Without, only Git is asked.
- `workspace.commit`: everything changed, with a message. `workspace.push`: the workspace's own
  branch to the project's own remote. Both answer with the state after.
- `pull_request.open`: a title, a description, draft or not. The host pushes first when the remote
  lacks any of the branch. The answer is the pull request, or the forge's own form when the host
  has no `gh` or is signed out of it.
- `pull_request.merge`: a number, a method, and the commit the person saw at its head. The host
  merges only the workspace's own pull request, open, not a draft, still at that commit, and asks
  `gh` to match that commit too.

No request names a remote, a branch to push to, a path or a credential.

**What the host does** (`crates/core/src/remote/publish.rs`): `Git::commit_all`, `Git::push`,
`Gh::create_pull_request` and `Gh::merge_pull_request`, with its own identity, credentials and
sign-in. The app's own publish state lives in the app, with its cache of the forge's answers, so
the host's is a smaller one written for the core, which `yardsort-host` has too.

- **Its words, never Git's.** What Git and `gh` print when they refuse names folders and a
  remote's address. A device gets a sentence the host wrote for each kind it can tell apart: the
  remote has commits the branch lacks, a rule there refuses the push, the host's sign-in was not
  accepted, nothing to commit, a hook refused the commit, the pull request exists, it changed.
- **The forge is asked only when told to.** A reading in between asks Git alone.
- **A remote is read as its configuration names it**, so a host that rewrites addresses still
  knows which forge it is on.

**What the window does.**

- **`RemotePublishBar`**, under the changes of a workspace there: the commit message and
  **Commit**, **Push _n_ commits**, **Open pull request** with its small form, and the pull
  request's line with its address and **Merge…**, which asks first and names the commit shown.
  Each is one press. The bar is outside what is laid out again when a file opens, so a half-typed
  message stays.
- **A lost answer** is said to be one that may have been done, and how the workspace stands is
  read again. Nothing is sent a second time by itself.
- **An address from that computer is opened only when it is `https`.**

**What building it settled.**

- **Simpler than the local bar.** No model writes the commit message or the pull request, there
  is no dialog of commits to pick a title from beyond the first, and no closing or reopening.
- **No pull request badge on the row in the tree yet.** The section's plan had one. It needs the
  pull request in what a host lists for every workspace, which would have it ask the forge at
  each reading; it is left for when the host caches the forge's answers as the app does.
- **`gh` only.** As locally, another forge gets its own form in the browser.
- **The tree's state moved up** in `RemoteChangesPanel`: its open folders and what each listed are
  kept by the panel, since the list is laid out again when a file opens. The earlier way, a
  viewer panel folded to nothing, made the panel library take presses meant for the commit box.

**Its tests.** The wire and its bounds in `remote-proto`; the host against a real repository with
a remote on disk behind a forge's address and a stand-in `gh`, including a push a hook refuses and
a merge at another commit, in `crates/core/src/remote/serve/tests/start.rs`; a desktop through the
real relay in `crates/server/tests/computer.rs`; the bar in `RemotePublishBar.test.tsx` and in the
panel's tests.

**Its smoke test.** `publish-there.yaml`, new, with the window steps `commits-there`,
`pushes-there`, `opens-pull-request-there` and `merges-there`, the pretend phone's
`cannot-publish`, and a stand-in for `gh` (`.yardsort/tests/support/standin-gh.sh`).

**Run on 2026-11-15**, on Linux, against a `yardsort-host` on the stand-in server with the real
window on a profile of its own: passed, 32 of 32 steps (run f10fecf0).

- The window's commit was made on the host, by the host's own Git identity, with nothing left
  uncommitted. Its push reached the remote.
- With a hook on the remote refusing pushes, a second commit was not pushed, the remote stayed
  where it was, and the window said why in a sentence with no folder in it.
- Opening a pull request pushed first, asked the host's `gh` to create it, and its line showed in
  the window.
- A merge at another commit than the one at its head was refused, and `gh` was not asked. At its
  head, `gh` was asked to merge exactly that commit. The window's question before a merge is the
  system's dialog, which the test cannot press, so it asked the core as the window does once it
  is answered; the question itself is in the unit tests.
- A pretend phone let only watch was refused the state, a commit, a push and a pull request.

The six earlier tests were run again on this build and passed: `desktop-client` (16 of 16, run
ef5c9e10), `same-session` (21 of 21, 40f0a159), `remote-start-and-resume` (25 of 25, 4aee1a89),
`run-on` (24 of 24, 209878c3), `run-on-full` (35 of 35, 52621f6a) and `changes-there` (25 of 25,
e6057af7). `changes-there` checked that the panel offers nothing that writes; with this slice it
offers the commit bar, so it now checks that nothing in it edits a file.

**What the first run found, which was in the test and matters beyond it.** The first run
(79091064) reached the pull request step and failed there: the host had run the owner's real
`gh`, signed in, and not the stand-in. A host reads its environment from the login shell, whose
startup files had put the owner's own programs ahead of the stand-in on `PATH`, and the test's
remote was named as a repository on github.com. That repository does not exist, so nothing was
created, but a test had asked a real forge, as the owner, to list and to create. Three changes
keep it from happening again: the remote is under a name reserved for tests, which nothing can
reach; the host in the test is started with a shell that reads no startup files; and a step stops
the run unless the stand-in was the one asked, before anything is asked to be created. Any later
smoke test that has a host ask a forge needs the same three.

Not run: macOS and Windows, a second real computer, and a real forge on purpose.

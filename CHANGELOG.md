# Changelog

Notable changes in each release. The [releases page](https://github.com/joaoh82/yardsort/releases)
has the downloads and the full commit lists.

## 0.33.0

- **A workflow's page has two views: Runs and Definition.** **Runs** puts the run list, the
  picked run's steps and the step's detail side by side, so notes, answers and artifacts are read
  at the width of the window instead of in a narrow column. **Continue from the failed step** sits
  right under the run's error, a run opens on the step that needs you or the one that failed, and
  a long run folds its succeeded steps into one row. **Definition** puts the chart beside the file
  instead of above it. See [Workflows](docs/guide/workflows.md#the-workflows-list).
- **Start a workflow from a project, in a workspace made for the run.** A run no longer needs a
  workspace to be there first. In the Run dialog, choose **new workspace** under a project, or
  open it from **Run workflow…** in the project's menu; from a terminal,
  `ys workflow run <id> --project <name>`. The branch and worktree are made once everything else
  has been checked, so a refused run leaves nothing behind. See
  [In a new workspace](docs/guide/workflows.md#in-a-new-workspace).
- **A schedule can make a new workspace for each run.** In the schedule dialog, choose
  **new workspace each run** under a project, or use `ys schedule add --project <name>`. Yardsort
  never removes these workspaces: the schedule keeps up to a number you set (5 unless you say),
  and at that many a time is refused, and you are told, until you archive or delete one. See
  [A new workspace for each run](docs/guide/schedules.md#a-new-workspace-for-each-run).
- **A workflow run lists the pull requests it opened.** The run's summary shows each one with its
  state and a link, in the project's own repository and in the repositories its workspace tracks,
  and the run's row says how many. `ys workflow runs` has a **PRS** column, and
  `ys workflow runs --run` lists them. See
  [What a run opened](docs/guide/workflows.md#what-a-run-opened).
- **A `report` step says what a workflow run came to.** Its body, in Markdown, is the run's
  result: first on the run's summary, and printed by `ys workflow runs --run`. It can list
  `{{ run.pull_requests }}`, and the built-in software factory now ends with one. See
  [A run's result](docs/guide/workflows.md#a-runs-result).

## 0.32.0

- **Continue a failed workflow run from the step that failed.** A run that failed part of the
  way through no longer has to start over: **Continue from the failed step** on the run, or
  `ys workflow continue <run>`, runs the failed step again and then the steps that were skipped
  after it. The steps that had succeeded are kept, with what they left and the agents they
  started. See [Continuing a failed run](docs/guide/workflows.md#continuing-a-failed-run).

## 0.31.0

- **Answer a workflow's questions one at a time.** When an agent in a workflow asks several
  questions, **Answer** in the run view opens a wide dialog with one question at a time, then all
  of them with your answers to look over before you send. Closing it keeps what you typed. See
  [Asking a question](docs/guide/workflows.md#asking-a-question).
- **Read a plan before approving it at the size of the window.** **Review and decide** on a
  waiting approval opens a dialog with the plan, or any document the step reads, formatted and
  filling most of the window. The note and **Approve**, **Request changes** and **Reject** are
  beside it. See [An approval](docs/guide/workflows.md#an-approval).

## 0.30.0

- **Container images for a computer with no screen.** Each release now publishes
  `ghcr.io/joaoh82/yardsort-host-full` and `ghcr.io/joaoh82/yardsort-host`, for amd64 and arm64,
  signed. The full one has Node.js, Bun, Go, Rust, Python, the GitHub CLI and the build tools, and
  the agents that may be handed on: Codex CLI, OpenCode, Pi and Oh My Pi. The compose example now
  runs it. See [In a container](docs/guide/headless-host.md#in-a-container).
- **`yardsort-host install-agent`** installs Claude Code, Cursor's CLI or Grok with the vendor's own
  installer, naming whose terms you accept first, or brings Codex, OpenCode, Pi or Oh My Pi up to
  date from npm, into `~/.local`. See [Installing an agent](docs/guide/headless-host.md#installing-an-agent).

## 0.29.0

- **An agent that needs you reaches your phone.** When Claude Code, OpenCode or Pi asks for a
  permission, on this computer or on one with no screen running `yardsort-host`, a phone with
  notifications on gets **An agent needs you**, and opening it opens that agent's terminal. One
  notification per wait: asking again before anyone answers sends nothing more. It can be turned
  off in **Settings → Notifications** on the phone. See
  [Notifications](docs/guide/remote.md#notifications).

## 0.28.1

- **Describe a change to a workflow.** **Customize** on a built-in now opens a box on your copy:
  say what should be different, and the workflow writer changes the file. Your own workflows
  have a **Describe a change** button that does the same. The answer keeps the workflow's id and
  stays unsaved until you save it. See [Describe a change](docs/guide/workflows.md#the-workflows-list).
- **The Floor Manager can change a workflow.** Ask it to, and it saves the changed file with
  the new `ys workflow save`, which checks the file first and refuses one with problems. Saving
  follows the Floor Manager permission, and at ask first the confirmation shows the whole file.
  See [Changing a workflow](docs/guide/floor-manager.md#changing-a-workflow).
- An open window now shows a workflow file changed outside it, such as by `ys`, within a
  second or two.

## 0.28.0

- **Software factory.** A built-in workflow, `software-factory`, takes a task from a brief the
  agent clarifies with you, through context, a plan you approve, implementation, a smoke test
  that must pass, and a review, and stops when you say it is ready to release. The run starts
  without an open pull request, because every step that names `{{ pr.* }}` comes after the
  review wait. A workflow that names the pull request before any `wait_pr_activity` — code
  review, address review, fix checks — still needs one open before it can start. Implementation
  pushes and, when the branch has none, opens a draft with `gh pr create --draft`; otherwise
  it updates the one already open. The review wait follows that pull request once it exists,
  and a review from before this wait began does not count. Requesting changes on the plan runs
  planning again. A failed smoke test, or
  changes after review, runs implementation again, up to three times including the first. You
  answer and decide in Yardsort, from a terminal, or on the phone. It does not merge, deploy,
  or roll back. See [The software factory](docs/guide/workflows.md#the-software-factory).
- **Diagnostics on the phone.** Settings → Diagnostics shows the phone app's version, the server's
  id, and each change to the sign-in, each computer and the network since the app started, with its
  time and error code, and shares that same text from the share sheet. It holds no names, titles or
  paths. See [Diagnostics on the phone](docs/guide/remote.md#diagnostics-on-the-phone).
- **The phone says when a computer was last seen on every screen.** Inside a computer that is
  offline, each screen now says when it was last seen, and what was read from it before is marked
  with when. A terminal no longer says its program keeps running while the computer is offline,
  which an asleep computer does not. See
  [When a computer cannot be reached](docs/guide/remote.md#when-a-computer-cannot-be-reached).
- **The server's log no longer quotes a database row.** An error from PostgreSQL is logged by its
  code and the table, column or constraint, not its message, which could include a device's name.
- **Workflows open as a list in the center panel.** The sidebar's Workflows section is now one
  **Workflows** row, under Tasks, like Tests and Triggers. Clicking it lists every workflow in
  the center panel, each marked **Built in**, **Custom**, or **Custom, replaces the built-in**,
  with its description, its problems, its runs going now, and the runs waiting on you at the top.
  A custom workflow can be deleted from its row, and a copy that replaces a built-in reset, after
  a confirmation. A workflow's view has **←** back to the list. See
  [The Workflows list](docs/guide/workflows.md#the-workflows-list).
- **A trigger that could never fire says so.** A trigger on a pull request's events whose
  workflow is about the workspace's own pull request, such as the **Review the pull request when
  it opens** preset, never hears the pull requests the workspace tracks in other repositories.
  In such a workspace the trigger dialog now says how many it tracks and offers **Use “Request
  code review elsewhere”**, and the Triggers view says the same of a saved one. See
  [Pull requests elsewhere](docs/guide/triggers.md#pull-requests-elsewhere).

## 0.27.0

- **The phone can be told about activity on this computer.** When Remote is on, a task that
  moved or finished, a review that needs attention, a failed workflow, an available desktop
  update, and a workflow question or approval each send one notification. It opens that item
  in the phone app. You answer, approve, and install on the computer. The phone notification
  never includes a one-time code. On the phone, Settings → Notifications asks once for
  permission. After that the token is registered for every account this phone is signed in to,
  when the app starts and after a sign-in. The screen can mute all of them for this account or
  turn a category off. A tap opens
  the item, including when the app was not running. The same desktop version can be mentioned
  again after a week. Task and review notices are sent when
  this computer loads those lists. A notice waits up to a day when the phone or the server cannot be
  reached, and is delivered once when it can. See
  [Notifications](docs/guide/remote.md#notifications).
- **A workflow can be answered from the phone.** While this computer is online, a phone signed
  in to the same account can see a run, including why it is waiting, and can approve, reject,
  request changes, answer a question, or resume a run that is ready to continue. The one-time
  code is not in the notification. The phone asks this computer for it when you decide, and
  does not show it. Watching a pairing includes reading workflow runs when the phone asked to
  see them. Approving or answering is a separate yes, and that checkbox is shown only when the
  phone asked for it. If the step changed before the tap, the phone says to refresh and reads
  the run again, and that tap does not use up a wrong code. A host token and a computer cannot
  decide. The action is not sent when this computer is offline, and it is not sent later. A
  phone paired before this needs to be unpaired and paired again. An older desktop is not shown
  workflow scopes, and they are not granted for it, until that desktop states that it speaks
  them. See [Notifications](docs/guide/remote.md#notifications).
- **A keyboard with the phone.** A Bluetooth or plugged-in keyboard now sends Esc, Ctrl and Alt
  combinations, Shift-Tab and the arrows to the terminal while the phone has control; letters still
  go into the field. The arrows, Tab and Backspace on their own reach the terminal while the field is
  empty. See [A keyboard with the phone](docs/guide/remote.md#a-keyboard-with-the-phone).

## 0.26.0

- **A workflow on a workspace's pull requests elsewhere, from the workspace.** When **Request code
  review** or **Address reviews** is started on a workspace with no open pull request of its own,
  the dialog now lists the open pull requests the workspace tracks in other repositories, and
  picking one runs **Request code review elsewhere** or **Address reviews elsewhere** on it.
  Before, a parent project's workspace whose own pull request was merged could only be told it
  had none, even with its agent's new pull requests in the clones tracked beside it. See
  [A pull request in another repository](docs/guide/workflows.md#a-pull-request-in-another-repository).

- **An agent in a workflow can ask you a question and continue in the same session.**
  On a `wait_session` with `clarify: true`, the agent asks with `ys workflow ask`
  (one `--question` each) and the run waits. You answer in the run view, or with
  `ys workflow answer` from a terminal Yardsort did not start. The terminal needs the
  one-time code shown in the window and, unless the run is set not to notify, in the
  notification. The code is issued either way. The window does not ask for it.
  The answer is typed back into that same session. A question only printed in the
  terminal is not asked. The step finishes when the agent records `ys workflow summary`.
  A summary is refused while a question is still waiting for an answer. The step also
  finishes when `until: exited` and the session has ended with no question still open.
  Going quiet does not finish it. Without a summary, a session that is still running
  waits until the step's timeout and then fails. If the session ends first, the step
  fails and says the session ended without a summary. Rounds are
  limited by `max_rounds` (3 when left out, 10 at most). One more than that fails the
  step. `from: brief` hands the questions, the answers and that summary to the next
  phase. This is not an approval. The run says `Blocked: waiting for your answer.`
  and `ys workflow runs` shows `your answer`. An answer that has not been claimed yet
  is typed once when Yardsort opens again. If the process stopped after the claim and
  before the typing was confirmed, the step fails and says the answer may not have
  reached the session, and it is not typed again. A busy session does not keep the
  claim, so the next pass tries again. See
  [Asking a question](docs/guide/workflows.md#asking-a-question).
- **Two workflow steps that loop back over the same steps in one pass both keep their notes.**
  A failed check and a request for changes that send the same earlier steps back are
  stored together, so neither `failure-<step>` nor `changes-<step>` is dropped. Each
  note is stored at that step's own attempt count.
- **A workflow can branch on a step's outcome, and send work back a limited number of times.**
  An `approve` or `run_test` step may set `on`. `approved` can go on to implementation,
  `changes_requested` can return the note to planning, a failed test can return to
  implementation, and `rejected` can stop the run or follow a path you name. The branch
  reads the outcome, not the text of a note. A route back needs `max_attempts`, from 2 to
  10, including the first try. At the limit the run stops, says why, and says to start a
  new run. You see that in `ys workflow runs` and on the run in the app. Each pass through
  an approval waits again and needs a new code and a new decision. The note or the failed
  report is an artifact the step being run again can read, and each attempt is kept. A
  failed route that goes forward leaves that report for the step it goes to as well. An
  earlier loop's note is not handed out again when a later loop runs those steps. A step
  cannot be named `stop`. A pass that goes to `stop` still ends the run after the step
  saves what it `produces`. A rewind is not applied when the run is already stopping, even
  when the looping step comes first in the file, and a smoke test still running on a step
  that is sent back is cancelled with that rewind. The count survives a
  restart and is not increased twice. A file with no `on` still loads.
  See [Conditional paths](docs/guide/workflows.md#conditional-paths).
- **A workflow that is waiting on you stays waiting across a restart.** The run view
  says `Blocked: waiting for your approval.` and what to do. The sidebar lists every
  run waiting on you. `ys workflow runs` has a WAITING ON column, and `--run` and
  `--json` say the same. A decision recorded while Yardsort was closed is applied
  once when it opens again. A restart does not approve a gate, start a step twice,
  send the waiting notification again, or write an artifact twice. The one-time code
  lives only in memory, so opening the app replaces it and sends a new notification.
  Listing a run does not. **Show a new code** in the window does, and sends one too.
  Leave `--code` out and `ys` asks at the terminal without echoing, when you have a
  real console. On Windows that is cmd, PowerShell, or Windows Terminal. Git Bash
  and mintty use `--code -`. With no terminal, `ys` fails at once and tells you to
  pass `--code -` and pipe the code in. `--code -` reads one line from standard
  input. Passing the code on the command line warns, because it stays in shell
  history and in the process list. Five wrong codes stay locked across a restart
  until **Show a new code**. Settings → General can leave the code out of the
  notification. That is the default on Linux, where other programs can read a
  notification; the notice then says to open Yardsort for the code. A file written
  before this still loads.
- **A workflow can wait for your approval.** An `approve` step shows the artifacts it
  reads and waits for you to approve, reject, or request changes. The steps that need
  it do not start until you approve. A rejection stops the run, and it is not an
  approval. Requesting changes needs a note, stops the run for now, and keeps that
  note as an artifact named `changes-<step>` for a later revision. You decide in the
  app, or from a terminal Yardsort did not start, with `ys workflow approve`,
  `reject` and `request-changes`. Those need the one-time code shown on the run and
  in the notification. `ys workflow runs` does not print it. Five wrong codes stop
  `ys` until the app shows a new one. A session Yardsort started cannot decide. The
  Floor Manager cannot either. This is not a sandbox: a process running as you that
  writes the database directly is not stopped. A decision is recorded once, with who,
  when and the note, and one on a finished run is refused. Yardsort tells you when a
  run starts waiting, unless the schedule or trigger says `never`. A run that is
  waiting when you quit is still waiting when you open it again.
  Waiting for a GitHub review is not this. A file written before this still loads.
  See [An approval](docs/guide/workflows.md#an-approval).
- **A workflow step can hand a named document to the next step.** `produces` names what a step
  leaves when it succeeds, and a later step `consumes` it by name. The next agent receives that
  text, and the run keeps it so you can read what was passed on, in the app and in
  `ys workflow runs --run <id>`. A file, a short text, or a field the step already leaves is
  copied. A conversation is copied only when the step asks, and then as a digest unless it asks
  for the turns themselves. Secret-looking paths are refused, including a link that points at
  one, and a long document is cut short. A `start_session` or `send_to_session` cannot hand on
  a file or a transcript. A step that succeeds again replaces what it left. A file written
  before this still loads. See [Artifacts](docs/guide/workflows.md#artifacts).
- **Floor Manager permission is a setting, and `ys` enforces it.** Settings → General chooses
  read only, ask first (the default) or trusted. The next `ys` command the Floor Manager runs
  follows it. Ask first waits for **Allow** in the window, even when the panel is closed, and
  auto mode does not skip that. Read only refuses a start or a write. Trusted may start agents,
  workspaces, workflows, tests and checkouts without asking. Deleting a workspace, closing a
  task and attaching stay refused at every level. This is not a sandbox: a shell command that
  is not `ys` is outside the gate. On Linux the Floor Manager reads the settings file in the
  config directory, the same one Settings writes. A settings file that cannot be read is
  treated as read only, and the refusal says why. Ask first shows the text a task comment
  would post, and posts that same text. A confirmation nobody is still waiting on says so,
  and does not cover the next command. A `YARDSORT_CONFIG_DIR` set for Yardsort is not
  passed on to the agents it launches, or to the terminal daemon. See
  [What it may do](docs/guide/floor-manager.md#what-it-may-do).
- **A smoke test step can stop the workflow when it fails.** A `run_test` step with `gate: true`
  fails the run when the test's verdict is a failure, and skips every step that has not
  finished, so a long wait does not keep the run open. A smoke test another step was waiting
  on is cancelled. The run names the step and the first line of the report. The step keeps
  the full report. You see both in the app and in `ys workflow runs --run <id>`. A schedule
  or trigger that tells you when a run fails sends that line. When there is more to read, it
  says to open Yardsort for the report. Without `gate`, a failed test still lets the workflow
  continue, as it does today. A file written before this still loads. See
  [Workflows](docs/guide/workflows.md#a-test-that-must-pass).
- **The phone notices a network change.** Moving between Wi-Fi and cellular could leave a computer's
  connection dead until the phone noticed by itself. Now a change of network has each open connection
  asked whether it still answers, and opened again within a few seconds if not; one already waiting to
  be reached again is tried at once. See
  [When the phone changes network](docs/guide/remote.md#when-the-phone-changes-network).
- **A larger interface.** Everything in the window is one step bigger, and the smallest text grows
  the most: text is 14, 13, 12 and 11 px where it was 13, 12, 11 and 10, and nothing is smaller
  than 11. Rows, headers, tabs, the bottom bars, buttons, icons and status dots are each a few
  pixels larger, and the faintest text is lighter in dark mode and darker in light, so it is
  easier to read. Diffs and the file viewer are 13 px, like the terminal, which is unchanged. In
  a new profile the side panels open a little wider; yours keep the widths you set. Nothing is
  added, moved or renamed. See [The desktop app's sizes](docs/design/39-desktop-sizes.md).

## 0.25.0

- **Triggers on pull requests in other repositories.** A trigger whose workflow is about a pull
  request a workspace tracks elsewhere — **Request code review elsewhere**, the new **Address
  reviews elsewhere**, or one of your own with `context: tracked_pull_request` — now hears the
  `pr.*` events of every pull request the workspace tracks with `ys pr track`, each on its own:
  reviews, comments, checks, conflicts, and `pr.opened` when one is tracked. The run is about the
  pull request that set it off. Before, no `pr.*` event fired for a tracked pull request, and such
  a workflow could not be a trigger's. A trigger about the workspace's own pull request still hears
  only that one. In the trigger dialog they are under **On a pull request it tracks elsewhere**.
  See [Pull requests elsewhere](docs/guide/triggers.md#pull-requests-elsewhere).
- **OpenCode 2 reports to the activity timeline.** With **Capture what OpenCode reports** on,
  OpenCode 2 now reports what it does, as OpenCode 1 does: before, its launches reported nothing,
  as it no longer loaded Yardsort's plugin. Yardsort asks `opencode --version` which one it is
  starting, and starts OpenCode 2 with `--standalone`, so that each launch's sessions run in a
  server of its own and are reported on the right run, not in the background service every
  OpenCode on the computer shares. OpenCode 1 needs 1.18.29 or later.
- **A workspace `ys` makes shows in the open window at once, with its agent in a tab.**
  `ys workspace new`, `ys task start` and the other `ys` commands that start an agent now put it
  in the window the moment it prints its first line; before, the workspace waited for the window
  to be focused again and the agent got no tab until the app was restarted.

## 0.24.1

- **Start work on another computer, and resume its conversations, from the phone or the desktop.**
  On another computer's page in the desktop app, **New workspace** beside a project, **New agent**
  beside a workspace and **Resume** beside an agent that ended start them there, by that computer's
  own projects and agents, and open the new terminal to watch. On the phone, an ended conversation
  the computer could resume shows **Resume**, with a message to go on with or none. Both need the
  computer to let the device type, as starting already does, and the computer to run this version.
  A start whose answer is lost on the way is never tried again by itself. See
  [Starting work from the phone](docs/guide/remote.md#starting-work-from-the-phone) and
  [Reaching another computer](docs/guide/remote.md#reaching-another-computer).
- **Refresh reads a merged or closed pull request elsewhere again.** **Refresh** in the Pull
  requests view and **Refresh pull request** in the session toolbar now read a tracked pull
  request that was merged or closed at once; before, it stayed as it was for up to ten minutes,
  which is still how long the window's own look leaves one.

## 0.24.0

- **`ys session resume` and `ys session fork`.** Continue an agent's conversation from the command
  line, as **Resume** and **Fork** do in the app, naming it by the start of its id from
  `ys session list --all`. On a computer with no screen, this is how agents a reboot or a container
  restart ended are brought back; restarting the host's service never ends them. A session that
  still says it is running, though its process went with the terminal daemon, is settled first.
  See [After a restart](docs/guide/headless-host.md#after-a-restart).
- **Follow pull requests an agent opens in another repository.** A project whose folder holds
  other repositories as nested clones — a parent repository — has agents that open pull requests
  in those clones, and Yardsort could not see them. Now a workspace can **track** them. With
  an agent's reporting on — **Capture what Claude Code reports**, or the matching switch for
  Codex, OpenCode, Grok, OMP, pi or Cursor — one the agent opens with `gh pr create` or a GitHub
  MCP server is tracked as soon as Yardsort hears of it, keeping only its link. (OpenCode 2 does
  not load Yardsort's plugin yet, so it reports nothing.) Otherwise an
  agent runs `ys pr track <url>` (agents in such a project are told where each clone is, to
  work in a worktree of it rather than its shared checkout, and to run it; `ys pr tracked` lists
  them), or you choose **Track pull request…** in the workspace's menu. A tracked pull request shows on the workspace's row as
  `lib#7`, in its toolbar's menu, and in the Pull requests view with its Summary — checks,
  reviewers and conversation — read by its link. It can be merged, closed, reopened and replied to from Yardsort, by its link, each
  confirmation naming its repository, and its **Code** is read in the project's nested clone of
  that repository. **Request code review…** has a second agent review it, running its checks in
  a worktree of that clone, and tells the agent that opened it when the review is on GitHub.
  **Stop tracking** takes it off again. See
  [Commits & pull requests](docs/guide/commits-and-pull-requests.md#pull-requests-in-other-repositories).
- **Reach another computer from the desktop app, read-only for now.** Under **Settings → Remote →
  Other computers**, sign this computer in as a client, a device of its own apart from it being a
  host, and pair it by the code another computer shows, from its own Settings → Remote or
  `yardsort-host pair`. Once let in there, the other computer shows under **Other computers** in the
  sidebar, online or offline since when, and opens to its workspaces, agents and recent activity,
  named across the top, with no Changes panel or Git action. See
  [Reaching another computer](docs/guide/remote.md#reaching-another-computer).
- **Watch another computer's terminals from the desktop app, and type into them.** On that
  computer's page, **Watch** opens an agent's terminal, or a shell's, at its size there, showing who has control.
  **Take control** lets you type into it, fitted to your window, until you release it or the person
  there or a phone takes it; the window says who did. Pairing from the desktop now asks to watch and
  to type, as a phone does, and the other computer's person can still let it only watch.
- **The Floor Manager: one agent that sees every project.** **Floor Manager** in the bottom
  bar's lower-right corner, or **Mod+J**, opens a panel with an agent you pick, started in a
  folder of its own and told how to use `ys`. Ask it what projects there are and what is
  running, which pull requests are not reviewed or need attention, or to start a task. It
  suggests a workflow and proposes exactly what it will run, then waits for your yes before it
  starts an agent, a workspace or a review, or writes to GitHub. Closing the panel leaves it
  running. _Floor Manager_ is a working name. See [Floor Manager](docs/guide/floor-manager.md).
- **`ys pr list`**: every project's open pull requests from the command line, with the Pull
  requests view's review filters (`--reviews none`, `not-by-me`, `awaiting-me`…) and
  `--attention` for failing checks, conflicts, changes requested, a review asked of you, or no
  review at all. **`ys pr checkout <n>`** opens a workspace on a pull request's branch, so
  `ys workflow run code-review` can review someone else's pull request. See
  [`ys pr`](docs/guide/cli.md#ys-pr).
- **Much less CPU while agents work.** The dot that pulsed while an agent printed kept the whole
  window redrawing at the screen's refresh rate — on Linux, about a third of a CPU core whenever
  any agent was busy. The dot is now still, and a count beside it says how long the agent has been
  at it (**12s**, **4m**, **1h 5m**), changing once a second at most. See
  [Status dots](docs/guide/terminals-and-sessions.md#status-dots).
- **Linux crashes can be traced.** The Linux builds keep their symbols, so a crash's backtrace
  names the functions it went through instead of bare addresses, for under a megabyte more
  download. Each release also carries `yardsort-<version>-linux-x86_64.debug`, which adds file
  names and line numbers. See
  [When Yardsort crashes on Linux](docs/guide/troubleshooting.md#when-yardsort-crashes-on-linux).

## 0.23.0

- **A guide for moving local Conductor worktrees to Yardsort.**
  [Migrate from Conductor](docs/guide/migrate-from-conductor.md) covers importing existing
  worktrees, what you reconfigure by hand, and what is not imported: Conductor scripts, chats, port
  allocation and Cloud.
- **Triggers: start a workflow or a smoke test when something happens.** When an agent says its
  work is done (`ys workspace done`), finishes a turn or exits, or a smoke test or a workflow run
  ends, a trigger starts what you gave it in that workspace, with what set it off as
  `{{ event.* }}`. A burst becomes one run, an event waits for a run in its way, and a cap, an
  hourly limit and a limit on chains stop loops. See [Triggers](docs/guide/triggers.md).
- **Triggers on pull requests.** A workspace's pull request opening, a review or a comment from
  someone else, its checks failing or passing, a conflict, a merge: Yardsort looks once a minute
  while a trigger watches, and what was there when the trigger was added is not news. A reply on
  a review thread counts once, as a comment, and your own posts — the agents' answers — do not
  count unless you say so. Ready-made triggers, `ys trigger add --preset`: **Address reviews**,
  **Fix failing checks**, **Review the pull request when it opens**, **Review when the work is
  done** and **Smoke test when the work is done**, with three new built-in workflows,
  `address-review`, `fix-checks` and `review-branch`. A fire with no agent in the workspace to
  tell is skipped, and you are told.
- **Triggers in the window.** **Triggers** in the sidebar lists every trigger with its event in
  words, where it is, what it runs and how its last fire went, and opens each to its history,
  **Edit…**, **Reset** when it is capped, and **Remove…**; the number beside it counts those that
  need you. The trigger dialog asks where, when, what to run and how often, and says in a sentence
  what will happen. The composer offers the ready-made triggers for each new workspace, ticked as
  they were last time in the project. **Triggers…** in a workspace's menu and an agent tab's menu
  opens the view on that workspace or agent, and runs a trigger started are marked _triggered_.
  An agent already running when a trigger on `ys workspace done` is added can be told about it
  with **Tell the agent**.
- **Custom events.** `ys event emit migration-written` raises an event of your own in a
  workspace — from an agent told to, a smoke test's `run` step, or a script — and a trigger on
  `custom.migration-written` starts what it starts, with the note as `{{ event.note }}`. Make one
  with `ys trigger add --on custom.<name>` or **A custom event** in the trigger dialog. `emit`
  says when nothing listens for the name, and which names are listened for.
- **Remote on a computer with no screen: `yardsort-host`.** A separate program, for a Linux server
  you reach over SSH: sign it in from a browser on any other computer by pasting back the address the
  browser lands on, run `yardsort-host serve`, and pair a phone with `yardsort-host pair` at a
  terminal of your own, which an agent cannot do. Start agents there with `ys`, or from the phone;
  stopping the host leaves them running. Build it from source for now. See
  [A computer with no screen](docs/guide/headless-host.md).
- **`yardsort-host` comes as a download**: a tarball for x86_64 and aarch64 Linux on each release,
  with `ys` and a systemd user service that leaves the agents running when it stops or restarts. Or
  in a container: `crates/host/Dockerfile` and a compose example, running as an ordinary user and
  listening nowhere. The guide covers installing, the service, upgrading and backing up.
- A shell with no `XDG_RUNTIME_DIR`, such as one from `sudo -iu`, now finds the terminal daemon the
  user's own services started, instead of starting a second one and taking the first one's agents
  for ended. A daemon an earlier release started in such a shell is still found where it is.
- **`ys project add <path>`** adds a repository as a project, as the app's Open project does.
- `ys workflow run`, `ys test run` and `ys schedule run` say so and queue nothing on a profile that
  `yardsort-host` runs, which does not carry them out yet. The app, started on such a profile, says
  what has it instead of waiting for a window that never comes.
- Fixed: started from a terminal, Yardsort could leave that terminal deaf to `Ctrl-C`. Reading
  your shell's environment, twice at once, handed the terminal to a shell that had already gone.

## 0.22.2

- **Remote: start work from your phone.** A phone you let type can start a **New workspace**, with
  the project, the agent and what it should do, or a **New agent** in a workspace, and opens its
  terminal. This computer makes it as its composer would, and shows it in the window at once. Needs
  this version on the computer and a new build of the phone app. See
  [Remote](docs/guide/remote.md#starting-work-from-the-phone).
- **Remote: give an agent a picture from your phone.** With control of a terminal, **Image** sends
  one from the phone's library; this computer keeps it outside every worktree and pastes its path,
  as dropping a file on a terminal does. See
  [Remote](docs/guide/remote.md#giving-an-agent-a-picture).
- **Remote: a terminal's history can be scrolled on the phone again.** A terminal taller than the
  phone stopped at the top of its screen; a drag now goes on into what scrolled off, and the phone
  keeps as much of it as this computer does.

## 0.22.1

- Project settings → Smoke tests: a variable's value field is wide enough to read again; the
  name field had taken the whole row.

## 0.22.0

- **Schedules: run a workflow or a smoke test at a set time, or on a routine.** **Schedules**, in
  the sidebar, lists them; **Schedule…** sits beside **Run…** on a workflow and on a test, and in
  a workspace's menu. Once at a date and time, every so many minutes or hours, daily, on chosen
  weekdays, monthly, or a cron expression, in this computer's local time, with the next times
  shown as you fill it in. Schedules fire while Yardsort is open: each says whether a time missed
  while it was closed runs once on opening or is skipped. A run that is still going is never
  joined by a second. A notification tells you when a scheduled run fails or could not start,
  or how every run ends. A smoke test file is approved when you schedule it in the app, and
  waits for approval again if it changes; a workflow that runs a smoke test is scheduled only
  when that test can run with nobody there. `ys schedule` does the same from a terminal. See
  [Schedules](docs/guide/schedules.md).
- **Only one copy of Yardsort runs on a profile.** Starting it again brings up the window it already
  has. Two copies could get this computer signed out of Remote, since the server takes one sign-in
  used twice for a stolen one. Development builds on profiles of their own are not affected.
- **Sign in again, instead of turning Remote off and on.** A computer signed out at the server now
  offers **Sign in again** in Settings → Remote, and stays the same computer, with its devices still
  paired. Turning Remote off while signed out no longer leaves a second entry on your phone either.

## 0.21.1

- **Settings → Remote says when a phone cannot open your terminals, and fixes it.** After updating
  from before 0.21.0, every terminal, new ones too, stays in the earlier background process until
  nothing runs in it, and that process cannot share them. Nothing on this computer said so. Settings →
  Remote now does, and can stop what is running and restart Yardsort, once it has named what stops and
  you have said yes twice. See [Remote](docs/guide/remote.md#if-a-phone-cannot-open-any-terminal).
- **The phone no longer says it is connecting when the computer has refused.** The message says that
  new terminals are affected too and where to fix it, and the status line and its dot agree.

## 0.21.0

- **Remote, still under development.** Settings → Remote lets a phone reach this computer's
  workspaces and terminals through a Yardsort server you sign in to. It stays off, and connects to
  nothing, until you turn it on, and a device gets in only if you say yes at this computer. Whoever runs
  the server can see what passes through it. The phone app is not in the app stores yet. See
  [Remote](docs/guide/remote.md).

- **A Tests view for every project's smoke tests.** **Tests**, in the sidebar, lists every
  project's tests with the verdict of each one's last run, and a **Runs** tab of every run across
  workspaces; filter by project, workspace, verdict or what the machine lacks, and on Runs by who
  asked. A test opens to its file, its history and **Run in…** a workspace of its project; a run
  opens its tab in that workspace. Its sidebar number is the runs waiting for approval. Press
  **Describe it** and a model writes a test from a description — the same Drafting writers the
  workflow editor uses — for you to check and save into a worktree's `.yardsort/tests`. See
  [Smoke tests](docs/guide/smoke-tests.md#in-the-app).

- **Workflows can run a smoke test.** A new `run_test` step queues one of the workspace's smoke
  tests and waits for its verdict, leaving the verdict and summary for later steps; it succeeds
  whether the test passed or failed, so a later step can act on either, and fails only when the
  run reaches no verdict or does not end in time. The built-in **Smoke test, then tell the agent**
  does exactly that and then tells the workspace's own agent how it went, pass or fail. Agents are told smoke tests exist in their first message,
  when the project has a `.yardsort/tests` folder or a test environment set. See
  [Workflows](docs/guide/workflows.md#steps) and [Smoke tests](docs/guide/smoke-tests.md#agents).

- **Smoke tests can drive a browser.** A `browser` step opens a page in a Chromium found on the
  machine — Chrome, Chromium or Edge, headless unless `headed: true` — and goes through its
  actions: `goto`, `click`, `fill`, `select`, `press`, `wait_for`, and the `expect_*` checks,
  each waiting for its element up to the step's timeout. It saves a screenshot when an action
  fails, after every action with `screenshots: every_step`, or wherever the file says
  `screenshot`. Nothing is installed and no Node is needed. A test that needs a browser none was
  found for is listed _needs a browser_ and refused, naming what was looked for; a project can
  name the browser to use under its smoke test settings. See
  [Smoke tests](docs/guide/smoke-tests.md#browser-actions).

- **Smoke tests in the app: the approval card, the run tab and Run smoke test….** A run of a
  file you have not approved — new, or changed since — now shows at the top of its workspace as
  a card: every step in words (a script fed to a program and a request body shown whole), the
  test environment it can read, what changed since the version you approved, and **Run** or
  **Don't run**; the workspace's row counts the
  runs waiting. A run opens as a tab beside the terminals, with its steps by phase, each step's
  note, facts and output tailed as it writes, **Pass** / **Fail** with a note for `manual` steps,
  **Cancel**, **Run again** and **Send result to agent**, which types the verdict into the
  workspace's agent once it is quiet. **Run smoke test…** in a workspace's menu starts one from
  the app, and pressing Run there is the approval. Project settings gain the test environment,
  with values hidden, **Ask every time** and how long to keep runs. See
  [Smoke tests](docs/guide/smoke-tests.md#in-the-app).

- **A terminal open in the app and in `ys attach` at once answers a program's questions once.**
  When a program asked where the cursor was, or which colours the terminal used, both answered,
  and the program could read the second answer as typing — `^[[24;1R` in a prompt. Now only the
  terminal attached last is asked.

- **Updating with agents running keeps new terminals in the daemon.** This version changes how
  the app and its terminal daemon talk, but only by adding to it, so it goes on using the daemon
  your running agents are in instead of starting new terminals inside the app, where they would
  stop when it quits. The daemon is replaced once nothing runs in it.

- **An agent can read what was said in the workspace's other conversations.**
  `ys workspace context` prints every message you typed and each turn's reply from the Claude Code
  and Codex conversations beside it, read from their own transcripts, without tool output, within
  about 8,000 tokens. Nothing is stored. On unless you untick **Share conversations between this
  project's agents** in a project's Memory view — what an agent reads goes to its provider.

- **A new agent is told when other conversations in its workspace can be read.** Claude Code and
  Codex get a short note as background context, so one started from the tab bar knows without a
  first message; other agents get it after the first message, and their buttons ask whether to
  start with the handoff. The tab says so, with **Show**; **Alt-click** starts an agent without
  it. Settings → Harnesses has a new **Context args** template.

- **Hand off… says what was said, and Cursor and OpenCode conversations can be read.** In a
  project that shares its agents' conversations, the handoff message has a _What was said_ part:
  your messages and each turn's final reply, read from the agents' own transcripts when you press
  it, within about 4,000 tokens. `ys workspace context` and that part now read Grok, Cursor,
  OpenCode, OMP and pi conversations too, beside Claude Code's and Codex's, and Grok, OMP and pi
  are told about them as background context, as Claude Code and Codex are. A resumed or forked Claude
  Code, OMP or pi conversation is told again, about the conversations there are then — Claude
  Code with `--system-prompt-snapshot off` on that resume, so it reads the new note. Settings →
  Harnesses has a **Resume context args** template for it.

- **`ys attach` and `ys logs` take the id `ys session list` prints.** They took only a workspace
  name or a terminal's own id, so two agents in one workspace could not be told apart. When a name
  matches several, each choice is listed with an id to name it by.

- **A conversation nothing has been said in yet is described as one.** Agents write no transcript
  until they are asked something, so `ys workspace context` listed a conversation started
  without a message as one whose transcript was not found, and the agent reading it could only
  guess. It now says that one was started without a message, so most likely nothing has been
  said in it yet.

- **`ys` stops quietly when what it is printing to stops reading.** `ys session list | head`
  panicked with _failed printing to stdout: Broken pipe_ once `head` had its lines, and
  `ys logs --raw` failed with _cannot write the screen_; both now exit without a word.

- **A terminal you come back to while it is printing no longer shows a stray fragment.**
  Switching to a workspace, or running `ys attach`, at the moment a program had printed half of
  a colour code or of a character such as `é` could show the rest as text (`1m`, `38;5;208m`) or
  leave a character or the space after it missing from the repainted screen. The repaint now
  carries the unfinished half, so the program's next output completes it as it would have.

- **Fork a workspace into a new one.** **Fork into new workspace…** in a workspace's menu starts
  a new workspace on a new branch made from that workspace's branch, in its own worktree — for
  building the next feature on the one in progress, where a handoff would share the worktree.
  Your message is the new workspace's task; what Yardsort recorded about the workspace you forked
  goes to the agent after it. Uncommitted files stay behind, and the composer says how many. See
  [Fork into new workspace…](docs/guide/workspaces.md#fork-into-new-workspace).
  `ys workspace fork <workspace> "<prompt>"` does the same from a terminal; `--print` shows the
  message the agent would get.

- **Smoke tests: the file format, the runner and `ys test`.** A smoke test is a YAML file in
  your repository, `.yardsort/tests/<id>.yaml`: setup, steps and teardown in order, with typed
  steps (`run`, `shell`, `start`, `wait_for`, `http`, `browser`, `manual`), expectations, and
  `{{ variables }}` checked when the file is read. `ys test validate` names every mistake at its
  line and column, `ys test list` and `show` read a workspace's tests, `ys test example` prints
  one to start from and `ys test guide` tells an agent how to write one. `ys test run <id>`
  queues a run in a workspace and Yardsort carries it out: every step in order on a free port
  of its own, output to files, a cancel that stops the step and still runs the teardown; and
  `ys test wait` prints the verdict for the agent that asked, with `runs`, `log` and `cancel`
  beside it. A file from a repository never runs unseen: a run of a new or changed file waits
  until you approve it in Yardsort. See [Smoke tests](docs/guide/smoke-tests.md).

## 0.20.2

- **Homepage and README lead with what Yardsort is for.** Orchestrate agents on Linux, macOS and
  Windows; share a worktree when you want; hand work to the next agent with recorded context so you
  do not re-explain the branch.

- **An AppImage updated from inside the app now starts the new version clean.** The old
  version used to start the new one with its own libraries still on the search path, so a
  library the new version no longer bundles was still loaded from the old copy until Yardsort was
  started afresh. That is how 0.20.1 kept drawing on the wrong GPU after an in-app update. The new
  version now starts with the environment you launched the old one with. Updating _from_ 0.20.1
  or earlier is still done by the old version, so after that one update, quit Yardsort and start
  it again.

## 0.20.1

- **The Linux AppImage draws on the GPU that drives your screen.** AppImages up to 0.20.0 carried
  a copy of `libwayland-client` too old for the Mesa drivers of current distributions (Arch,
  Fedora 42 and later, Ubuntu 25.10 and later), so Mesa could not load inside the app. On a laptop
  with an NVIDIA GPU beside the one driving the screen, every frame was drawn on the NVIDIA card
  and copied across, which made typing and selecting in the terminal feel late; without NVIDIA,
  the app could not start at all. The AppImage is now built with Tauri CLI 2.12, whose bundler
  leaves that library to the system (tauri-apps/tauri#16062), and it starts as a native Wayland
  window without Yardsort having to override its launcher. See
  [Troubleshooting](docs/guide/troubleshooting.md#linux).

## 0.20.0

- **Clone a repository by picking it from a list.** **Clone a repository** now lists the
  repositories your `gh` account owns or collaborates on, most recently pushed first; type to
  narrow it, pick one, and **Clone project** — the location is remembered from last time and
  shown as a line, so the usual path asks nothing about folders. **Search GitHub for “…”** finds
  the rest, including your organisations', when you press it. A repository that is already a
  project says so and **Go to project** selects it instead. A fork gets its parent as
  `upstream`. Pasting a URL still works, with or without `gh`. See
  [Projects](docs/guide/projects.md#clone-a-repository).

- **Start from a link.** **From a link…**, in the Tasks and Pull requests headers and in the
  command palette, takes an issue's or a pull request's address — `owner/repository#12` works
  too — and opens it in its view, selected, with **Delegate** or **Start workspace** the next
  press. A repository that is not a project yet goes to the clone dialog with the URL filled
  in, and the item opens when the clone is done. See
  [Tasks](docs/guide/tasks.md#starting-from-a-link).

- **Clone from any git host.** The same dialog takes an HTTPS or SSH clone URL from GitLab,
  Bitbucket, Gitea, Forgejo or a host of your own — subgroups included — where it used to refuse
  anything but GitHub. `owner/repository` on its own still means GitHub. Commits, pushes and
  **Open merge request** already worked on those hosts; their issues and pull requests are still
  not listed, and the views say so. See
  [Projects](docs/guide/projects.md#clone-a-repository).

## 0.19.1

- **A merged pull request now counts as _kept_ without you opening Outcomes.** Attempts are
  matched to pull requests whenever Yardsort reads a project's pull requests, not only when the
  Outcomes view is open. And the match no longer misses: an attempt nobody had looked at was
  recorded as beginning when its workspace was deleted, so a pull request opened more than ten
  minutes before that was never taken as its own — which was nearly every one. Attempts deleted
  before this update are matched too. See [Outcomes](docs/guide/outcomes.md#what-counts).

- **A project gives its agents its memory unless you turn that off.** _Give this project's agents
  its memory_ used to start unticked, and a project nobody had ticked it in never asked an agent
  for anything. It now starts ticked — in the projects you already have, too, unless you ticked
  and unticked it there. Untick it in **Memory…** and it stays off. Only entries you approved are
  ever given.

- **An empty project memory asks agents to fill it.** A project that gives its agents its memory
  but has nothing approved yet used to add nothing to a first message, so no agent ever heard of
  `ys memory propose` and the memory stayed empty. The first message now says the memory is empty
  and asks the agent to propose what the next one should know; with entries, it asks the same
  before the agent finishes. Agents a workflow starts are not asked. See
  [Memory](docs/guide/memory.md#how-it-fills-up).

- **Usage: the plan-limits card has its own heading.** It is labelled **Plan limits**, with a
  note that only agents that record their limits appear. Today that is Codex alone, and the card
  no longer reads as a header for the whole tab.

## 0.19.0

- **A pull request's checks going green, or its merge, now shows without you doing anything.**
  The `#42 · checks running` link in the Changes panel only caught up when the workspace's files
  changed or the window came back into focus; it now follows the once-a-minute poll like the
  sidebar does. And a workspace whose open pull request has fifty newer ones in front of it no
  longer shows it as it was when Yardsort started while the Pull requests view is closed: that
  pull request is asked about by itself each minute.
- **Tasks: manage them from Yardsort.** **New task** opens an issue on a project's repository,
  with a title, a description and labels. A box under a task's conversation posts a comment.
  **Close ▾** closes one as completed or as not planned, and **Reopen** reopens one — each
  asked about first, naming who opened it. **Labels ▾** and **Assignees ▾** tick and untick.
  Everything goes through `gh`, as you; a refusal from GitHub is shown in its own words.
- **`ys task create`, `comment`, `close`, `reopen` and `edit`.** An agent in a workspace can
  now file an issue, answer one, label and assign it. They write to GitHub at once; `close` and
  `reopen` do nothing without `--yes`. See [the `ys` guide](docs/guide/cli.md#ys-task).
- **Tasks: hand one to an agent.** **Delegate** on an open task opens the composer with the
  first message written from the issue — title, description, the latest comments — between two
  lines that mark it as text other people wrote, with what the agent should do if that text
  asks for more than the work. Nothing starts until you press Start. The workspace is named
  after the task and remembers it: the task names its workspaces and goes to them, **Delegate
  again** starts another attempt, and the workspace's row shows the task's key. Nothing is
  written to GitHub. `ys task start <task>` does the same from a terminal, and
  `ys workspace list --json` says which task each workspace came from. See
  [Tasks](docs/guide/tasks.md#handing-a-task-to-an-agent).
- **Tasks: every project's GitHub issues, in one place.** **Tasks**, under Pull requests in the
  sidebar or in the command palette, lists the open issues of every project on GitHub — up to
  the 200 most recently updated — with who opened each, its labels, assignees, comments and any
  pull request that will close it. The number on the sidebar row is how many **need an
  answer**: open, and the last person to speak was not an owner, member or collaborator; bots
  are nobody's word. Filter by state, project, label, assignee, author and _Needs an answer_,
  and search; the filters are remembered. Open one for its description and conversation, as
  Markdown that loads no images. Reading only, for now: handing a task to an agent and managing
  tasks from the app come next. See [the guide](docs/guide/tasks.md).
- **`ys task list` and `ys task show`.** The same tasks from a terminal, with `--needs-answer`,
  `--state`, `--label`, `--assignee`, `--author`, `--search` and `--json`, so an agent in a
  workspace can be asked which issues are open and which are waiting. See
  [the `ys` guide](docs/guide/cli.md#ys-task).
- **Pull requests: comments, and a note on lines for an agent.** A box under a pull request's
  conversation posts a comment on it. Under **Code**, each comment made on lines of the diff sits
  beside those lines, with its replies, and each file says how many threads it has. Select lines
  and press **Note on lines…** to send what you want done about them to the agent in the pull
  request's workspace — typed in if it is running and quiet, resumed with it if it has ended,
  never into a busy one — or, with no workspace yet, to start one with the note as the agent's
  first message. The same note can also go on GitHub as a comment on those lines, or only there.
  See [Pull requests](docs/guide/pull-requests.md#a-note-on-lines-for-an-agent).
- **Pull requests: the Code tab.** A pull request's details now have two tabs, **Summary** and
  **Code**. Code lists the files it changes with their kinds and line counts, and opens each in
  the changes panel's own viewer, inline or side by side, highlighted, with unchanged stretches
  folded. Nothing is checked out for it: the pull request's commits are fetched into refs of
  Yardsort's own, no branch or remote-tracking branch moves, and what was fetched is let go when
  the pull request leaves the list. The diff is measured from where the pull request left its
  base, so one merged long ago still shows its files. See
  [Pull requests](docs/guide/pull-requests.md#code).

## 0.18.0

- **Install ys no longer, now and then, says the copy it just made is "not Yardsort's".** Right
  after the copy, the file could still be held open for a moment, and running it to read its
  version failed. The check now waits that moment out.

- **The standalone `ys` for macOS is signed and notarized.** The `ys-…-macos-universal.tar.gz` on
  the releases page used to hold an unsigned binary, which macOS refused with _"ys" Not Opened_
  when it was downloaded with a browser. It is now the same signed, notarized file the app
  carries. See [the `ys` guide](docs/guide/cli.md#without-the-app).

- **`brew install --cask joaoh82/yardsort/yardsort` works again.** Current Homebrew refused the
  cask over a minimum-macOS line it no longer accepts (#90). The cask now says `depends_on :macos`
  with no minimum version — the one form today's `brew style` takes, which also insists on the
  stanza for every app cask and on its exact placement — and releases test the cask against an
  up-to-date Homebrew before publishing it, so a rule change like this is caught there rather
  than by users.

- **Usage is easier to find in the docs and website.** The README and homepage now explain both
  tabs, supported token sources and API cost estimates. The quick start introduces Usage, and
  the guide adds a first-look walkthrough, refresh guidance and the distinction from activity
  capture. New screenshots show Token usage and Machine resources with demo data.
- **Pull requests: every project's, in one place.** **Pull requests**, at the top of the sidebar
  or in the command palette, lists the pull requests of every project on GitHub — every open one
  up to the 200 most recently updated, and the newest fifty of any state — with who opened each,
  its checks as passed out of total, its age, its size and whether it conflicts. Filter by state,
  project, author and review status, including _Awaiting review from you_ and _Reviewed by you_;
  the filters are remembered. Open one for its **Summary** — the description as Markdown, every
  check with a link to its run, the reviewers, and the conversation of comments and reviews —
  and to act on it: **Start workspace**
  fetches its branch and opens the composer on it, **Merge** (squash, merge commit or rebase),
  **Close** and **Reopen** each ask first and say whose pull request it is. No branch is ever
  deleted, and a branch you already have is never moved. A pull request from a fork becomes a
  `pr/<number>` branch that follows it; Yardsort does not push to forks, and the changes panel
  says so instead of offering **Push**. Other people's words are shown with care: no image in
  a description is loaded, links open in your browser, and HTML is not rendered. It needs `gh`,
  and says plainly when `gh` is missing, logged out, or the project is on another forge. See
  [Pull requests](docs/guide/pull-requests.md).
- A workspace whose open pull request has more than fifty newer ones in front of it now shows it
  on its row: every open pull request is read when Yardsort starts, not only the newest fifty.

## 0.17.0

- **Usage: the tokens your agents spent, and what they use of the machine.** **Usage**, at the
  foot of the sidebar or in the command palette, has two tabs. **Token usage** reads the logs
  Claude Code, Codex and Grok keep on this machine and shows the last 7, 30 or 90 days by day,
  agent, model and workspace — with cache reads, cache writes, output, and what the cache saved —
  priced at each vendor's API rates as an estimate (a subscription is not billed that way), and
  Codex's plan limits as it last heard them. **Machine resources** samples every two seconds while
  it is open: Yardsort's CPU and memory over the last five minutes, the machine's memory and load,
  and a breakdown by project, workspace and terminal, each terminal counted with every process it
  started. Nothing is sent anywhere. A switch in its header takes it off the sidebar. See
  [Usage](docs/guide/usage.md).

## 0.16.0

- **Every pull request a workspace opened.** A workspace that opened more than one — a second PR
  on its branch after the first merged, or one from another branch its agent created — shows the
  rest beside its badge (**#42 +2**), lists them when you hover the row, and lets you switch the
  toolbar between them. Which PRs belong to a workspace comes from its worktree's own git history,
  so another workspace's PR on a reused branch name is not counted.

- **Ask the agent to resolve merge conflicts.** When GitHub reports that an open PR conflicts
  with its base, the badge turns red with a **⚠**, and the toolbar menu offers **Ask its agent to
  resolve conflicts…**. It asks the agent that opened the PR — typed into it if it is running,
  resumed with the request if it has ended — to merge the base in, resolve the conflicts, run the
  checks and push, never rebasing or force-pushing. A busy agent is never interrupted.

- **Copy and paste in the terminal work the way more desktops expect.** Right-click for Copy,
  Paste and Select all. Plain `Ctrl+C` copies while text is selected and interrupts otherwise,
  so Omarchy's `Super+C` copies. `Ctrl+Insert` and `Shift+Insert` copy and paste too.

- **Always start an agent in auto mode.** Settings → Harnesses has a switch per agent that starts
  every new, resumed and forked session in that agent's own auto mode — `--permission-mode auto`
  for Claude Code and Grok, `--approve-for-me` for Codex, `--auto` for OpenCode,
  `--approval-mode=write` for OMP and `--auto-review` for Cursor. Off until you turn it on; custom
  harnesses can give their own flag.

- **A nudge to compact a full context.** When a Claude Code or Codex conversation has used 80 %
  of its context window, a bar above its terminal says so and offers **Compact**, which types the
  agent's `/compact` for you; **Not now** hides it until the next step up. It reads the agents' own
  usage records, so it needs their activity capture switched on in Settings → General. The
  Activity timeline shows the same numbers on each turn.

- **Blog and tutorials on the website.** Read the story behind Yardsort, a practical parallel-agent
  tutorial, a worktree explainer, and dated comparisons with Superset and Conductor.

- **Answers and navigation on the website.** The homepage and About Yardsort guide explain
  supported agents, costs, worktrees, background sessions and data handling. Documentation pages
  now have breadcrumb navigation; product and breadcrumb structured data describe the same content.

## 0.15.0

- **Keyboard navigation and configurable shortcuts.** Search commands and workspaces with
  Mod+K, focus the three panels, cycle workspaces and terminal tabs, and navigate trees with
  arrow keys. Settings → Keyboard is the searchable cheat sheet and binding editor, with
  conflict checks, clear/reset controls and saved bindings. Dialogs keep keyboard focus inside.
  Modified arrows retain text-editing behavior in text fields, and disabled commands leave keys
  unconsumed. Saved custom bindings take precedence over new defaults; recovery notices explain
  conflicts or invalid saved data without resetting unrelated shortcuts.
- **An optional welcome tour.** First launch asks whether to take a short guided tour of the
  main panels. Skip or finish it once; replay it any time from Help / Tour in the bottom bar.
- **Clone projects from GitHub.** Add a project from an HTTPS or SSH repository URL, or
  `owner/repository`, choosing its local name and location. Common GitHub web links are accepted,
  the folder name is suggested from the repository, and cloning can continue in the background
  while you use your terminals.
- **Show files in your file explorer.** Right-click entries in Files or Changes to reveal
  their location; deleted files open the nearest existing folder.
- **Clearer website search and link previews.** Every guide now has its own description and
  share title, with a branded preview image, consistent canonical URLs and a sitemap.

## 0.14.0

- **Images and editing in the file panel.** Selecting an image opens a preview; SVG files can
  switch between preview and source. Text and code files can be edited and saved inside
  Yardsort, with syntax highlighting, undo, retained drafts, and protection against saving over
  a file changed by an agent or another editor. Disk reloads reset undo history; symlinks are
  explained before editing, and read-only files are refused without changing their permissions.
  Image changes show before and after previews, retaining text when a file changes type.

## 0.13.1

- **A Yardsort that cannot start now says why.** Opening your data with an older version than
  the one that last used it — after going back to an old download, say — used to close the app
  a moment after launch, every time, without a word. It now shows a dialog saying so and asking
  you to install the latest version; your data is left as it was. Any other failure while
  starting is shown the same way. On Linux such a failure no longer counts as Wayland not
  working, which kept that version of the AppImage under XWayland from then on. See
  [Troubleshooting](docs/guide/troubleshooting.md#yardsort-cannot-start).
- **A way back after an upgrade.** Before a new version upgrades your database, Yardsort keeps a
  copy of it as it was, `yardsort.db.before-upgrade`, which the version before can still open.
  If you go back to that version, its dialog points at the copy. See
  [Going back to the older version](docs/guide/troubleshooting.md#going-back-to-the-older-version-instead).

## 0.13.0

- **Workflows: named agent work, in the sidebar and from `ys`.** A **Workflows** section above
  Projects lists them; open one to see its steps as a chart, edit its file with every mistake
  marked where it is, follow its runs, and **Run…** it. A new one can be written for you: say
  what it should do and press **Write it**, and the agent you already have, or your Anthropic
  key, writes the file, checked like any other and never saved until you say so. A workflow is a
  short YAML file in your profile's `workflows` folder: the inputs to ask for, and steps that
  start an agent, wait for it to settle, type to it once it is quiet, and notify you. Steps run
  as soon as the ones they need succeed, so they can run side by side. `ys workflow validate`
  checks a file and names the line and column of every mistake, including every
  `{{ variable }}`; `list`, `show` and `copy` manage them. With Yardsort open, `ys workflow run`
  queues a run and the app carries it out in the background, with the agents it starts as tabs;
  `ys workflow runs` follows it and `cancel` stops it, never the agents. Steps can wait for a
  review or comment on the workspace's pull request, use its number and link, and talk to the
  workspace's own agent. The built-in **Request code review** puts it together, from a
  workspace's menu or as `ys workflow run code-review --input reviewer=codex`: a second agent
  reviews the pull request and posts on GitHub, then you are told, and the agent that wrote it
  is told to address the review. See [Workflows](docs/guide/workflows.md).
- Workflow generation has a **Workflow writer** choice in Settings → Assist, including Codex and
  custom harnesses with Write args. Automatic selection skips harnesses that are not installed.
  Select a chart node to highlight it and jump to its YAML source line.
- **Project search.** Click the search icon beside Projects to filter by name. Clear the filter
  with **×**, or press **Escape** to close it; **×** also closes an empty field. The selected
  workspace's project stays visible, including projects added while filtering.
- Workspace rows now preview pull request details on hover or keyboard focus, including branches,
  review status, line counts and individual checks. Open harnesses have a count and a preview of
  their names and activity. The workspace toolbar adds the same PR preview with actions to open,
  copy, refresh, or merge by squash, merge commit or rebase after confirmation.
- Workspace previews open on hover or keyboard focus without popping over clicks or menus.
  They only say no PR was found after the forge has answered successfully.
- PR badges and the toolbar dropdown share status colours: green for open PRs, yellow for
  drafts or pending checks/reviews, red for failures, requested changes or closed PRs, and
  purple for merged PRs. Open PRs without CI no longer fall back to grey.
- Reused branches prefer an open pull request over an older merged or closed one. Slower refresh
  responses no longer replace newer PR data in the sidebar or the core cache.

## 0.12.0

- **Project memory.** Short lessons about a project for its agents — "the tests need
  `TZ=UTC`". Write them in **Memory…** from a project's menu, where they are approved as you write
  them; agents propose them with `ys memory propose`, and those wait for you, with a count on the
  project's row. Tick **Give this project's agents its memory** and every agent you start there
  with a first message gets the approved entries after it, cited, as notes rather than
  instructions — never into what the workspace was asked — and handoffs carry them too. Agents
  can run `ys memory search` any time. Nothing an agent proposes reaches another agent until you
  approve it, and there is no command an agent could run to approve it. With Assist, a new switch
  marks proposals that repeat or contradict an approved entry. See
  [Memory](docs/guide/memory.md).
- **Outcomes: what became of each attempt.** After you archive or delete a workspace, the
  sidebar asks how it went — kept, partly or discarded, one optional click. **Outcomes…** in a
  project's menu lists every attempt, deleted ones included, with its task, agents, outcome and
  evidence, and each agent's history. A merged pull request or a branch merged into its base
  counts as kept until you say otherwise; nothing else is ever counted. The composer shows your
  history with the agent you picked — "too few to say yet" until there are five outcomes. See
  [Outcomes](docs/guide/outcomes.md).
- The composer's hint under a handoff said it runs "in the project's own checkout", which is only
  true of `local`; in a worktree it now says it runs in this workspace.

## 0.11.0

- **`ys` comes with the app, and stays up to date with it.** Every installer now carries the
  command-line client next to Yardsort. The `.deb`, `.rpm` and AUR packages put it in
  `/usr/bin`, and the Homebrew cask links it. Elsewhere, **Install ys** — in Settings → General
  and in the first-run checklist — links it into `/usr/local/bin` on macOS, or copies it to
  `~/.local/bin` (AppImage) or to a folder it adds to your `PATH` (Windows). Both screens say
  which `ys` your terminal finds and whether it is this version. A copy is replaced when Yardsort
  starts and finds it older, and a file there that is not `ys` is replaced only after you
  confirm. On macOS the bundled `ys` is signed and notarized with the app. See
  [Installing `ys`](docs/guide/cli.md#installing).
- **Activity: a local record of what ran.** Yardsort now notes when each agent, shell or run
  command starts in a workspace — fresh, resumed or forked, from the app or from `ys` — and how
  it ended, without reading anything the agent prints. An agent that finishes while the window
  is closed is no longer listed as _interrupted_ the next time Yardsort starts: the background
  process keeps its exit and the record gets the real exit code. **Show the activity timeline**
  in Settings → General (experimental) adds an **Activity** button to each workspace's footer;
  **Record when agents start and exit** switches the recording off. `ys activity list` and
  `ys activity export` (NDJSON) read it from a terminal, and `ys doctor` reports how much there
  is. Programs started in a workspace are given `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID` and
  `YARDSORT_SESSION_RECORD_ID`. Nothing leaves the machine. See [Activity](docs/guide/activity.md)
  and the [design note](docs/design/10-agent-events-stage-1.md).
- **Claude Code can report what it does.** Switch on **Capture what Claude Code reports** in
  Settings → General and every Claude Code that Yardsort starts tells the activity timeline about
  each prompt, tool call, permission prompt, turn and session end — as metadata: the tool's name
  and the file's relative path, never the prompt, the command or the output. It rides on a
  per-launch settings file, so your own Claude Code settings and hooks are never touched, and it
  keeps working while the window is closed. The timeline moves as the agent works; `ys activity
list` shows the same rows as `claude/hook`. Off by default. Other agents stay at what Yardsort
  itself sees. See [What Claude Code reports](docs/guide/activity.md#what-claude-code-reports)
  and the [design note](docs/design/11-agent-events-stage-2-claude.md).
- **Codex can report what it does.** Switch on **Capture what Codex reports** in Settings →
  General and every Codex that Yardsort starts puts each turn on the activity timeline: the shell
  commands it ran with their exit codes and durations, the files it added or changed, the tokens
  the turn cost, and how long it took — never a command line, a file's contents or a message. It
  works through Codex's `notify` program, given for that launch alone, plus Codex's own session
  file; nothing in `~/.codex` is edited and a `notify` of your own keeps running. Off by default.
  See [What Codex reports](docs/guide/activity.md#what-codex-reports) and the
  [design note](docs/design/12-agent-events-stage-2-codex.md).
- **OpenCode can report what it does.** Switch on **Capture what OpenCode reports** in Settings
  → General and every OpenCode that Yardsort starts puts its work on the activity timeline as it
  happens: each tool with its file or exit code, tools that failed, files it edited, permission
  prompts, and the tokens each reply cost — never a message, a command, an output or a file's
  contents. It works through a small plugin given to that launch alone through OpenCode's
  environment; your `opencode.json` is not edited and your own plugins keep running. Off by
  default. See [What OpenCode reports](docs/guide/activity.md#what-opencode-reports) and the
  [design note](docs/design/13-agent-events-stage-2-opencode.md).
- **Grok's own session log on the timeline.** Switch on **Read what Grok records** in Settings →
  General and every Grok that Yardsort starts has its turns read from the log Grok keeps for
  each session: which tool ran and how long it took, tools that failed, the permissions you were
  actually asked for and how long you took to answer, each turn and its tokens. That log never
  holds a command, a path or a message. Nothing is given to Grok and nothing in `~/.grok` is
  written; Yardsort chose the session id, so it knows where to look. Off by default. See
  [What Grok records](docs/guide/activity.md#what-grok-records) and the
  [design note](docs/design/14-agent-events-stage-2-grok.md).
- **Every built-in agent can now report what it does.** Three more switches in Settings →
  General, all off by default: **Capture what OMP reports** and **Capture what pi reports** give
  those agents a small extension on the command line of each launch — never in your extension
  folders — that reports each turn with its tokens and cost, each tool with its file or duration,
  and, on OMP, each permission you answer. **Capture what Cursor reports** gives the Cursor agent
  a plugin directory for that launch, whose hooks report each tool and file edit beside your own
  hooks. Never a prompt, a command, a file or a reply, as before. Both are
  recorded against the real agents. See [What OMP and pi report](docs/guide/activity.md#what-omp-and-pi-report),
  [What Cursor reports](docs/guide/activity.md#what-cursor-reports) and the design notes
  ([15](docs/design/15-agent-events-stage-2-pi-omp.md),
  [16](docs/design/16-agent-events-stage-2-cursor.md)).
- **Cursor and pi recorded.** The Cursor and pi adapters now rest on recordings of the real
  agents rather than their documentation. For Cursor that found tool durations being dropped
  (they are fractional), which is fixed, and that this Cursor build does not tell a plugin's
  hooks when a turn ends. For pi it found that a `write` is named by its start and not its end,
  which the Changes list's badges now allow for.
- **The Changes list says which files an agent reported writing.** With any _Capture what …
  reports_ switch on, a changed file the agent said it wrote carries a badge with the agent's
  name — hover it for how many times, when, and through what — and a line above the list counts
  the files with a report and the files without one: you, a script, or a run that was not
  reporting. The expanded diff's header says the same for the open file. The other way round, a
  timeline row that names a listed file has a **Show diff** button. It never claims which lines came
  from whom; git's diff is the whole change and no agent reports a line. Nothing new is
  stored. See [Who wrote it](docs/guide/changes-and-files.md#who-wrote-it) and the design note
  ([17](docs/design/17-agent-events-stage-3-review.md)).
- **A file an agent made with a shell command is marked too, as seen rather than reported.**
  A command names no file, so it cannot be reported; but the file's own modification time says
  when it was last written, and if that falls inside the command's run on the timeline the file
  gets a dashed badge with the agent's name. Its tooltip says exactly that: the time on the file
  was read, not who wrote it. A report, when there is one, is shown instead. Only a tool call
  counts as a window, never the agent merely being open, so with no capture switched on nothing
  changes. See [Who wrote it](docs/guide/changes-and-files.md#who-wrote-it).
- **Assist can be told who wrote each file.** A new switch under Settings → Assist, off by
  default and offered once the review is on, sends with each diff one sentence in the Changes
  list's own words: _reported written by claude_, _last written while claude ran a command_,
  or _not reported written by any agent_. Never a tool, a time or a command. A substantive
  change no agent accounted for then earns an **unaccounted** badge, and every Assist badge's
  tooltip ends with the exact sentence Assist was told. See
  [Telling Assist who wrote each file](docs/guide/assist.md#telling-assist-who-wrote-each-file).
- **Hand off a workspace to another agent.** A new **Hand off…** button in the tab bar writes
  the next agent's first message from what Yardsort recorded: what the workspace was asked,
  where the branch stands and who wrote each changed file, what each agent run did, and what
  is not known — never the last agent's words, which Yardsort does not keep. It opens in the
  composer to read and edit before it is sent; the new conversation's record keeps no first
  message, so the task on record stays yours. See
  [Handing work to another agent](docs/guide/terminals-and-sessions.md#handing-work-to-another-agent)
  and the design note ([18](docs/design/18-agent-events-stage-4-handoff.md)).
- **`ys workspace handoff`, and Assist's order in the handoff.** The handoff packet can be
  printed from the command line with `ys workspace handoff <workspace>` (`--json` for the text
  with counts), the same text the app's button starts from. In the app, with Assist reviewing
  changes, the packet lists the changed files in the order Assist would look at them — on task
  first, unrelated last — with its word on each, from the review the Changes list already
  shows; nothing new is sent, and the packet says it is a judgment, not a fact.

## 0.10.0

- **OMP, Cursor and Pi are built-in harnesses.** Pick them in the composer and configure them
  in Settings → Harnesses, with install links on the welcome screen. OMP and Pi can write commit
  messages and pull request descriptions; Cursor drafting stays disabled to protect Resume. OMP and Pi expose thinking levels; Pi supports
  resuming and forking by session id, while OMP and Cursor resume the latest conversation in
  the workspace. Existing custom harnesses with these IDs keep their definitions and session
  identity. OMP and Pi accept opening messages starting with `@` as text. See [Harness settings](docs/guide/settings.md#harnesses).

- **Project setup and dev servers.** Project settings can copy files such as `.env` and run a setup script in new or restored worktrees before starting an agent. Failed preparation keeps the workspace and setup log. Logs stay outside the checkout, imported worktrees skip setup on restore, and blank argument lines are ignored. Configure a run command and use **▶ Run** to start or focus a dev-server terminal in any workspace. See [Project automation](docs/guide/projects.md#project-automation).

- **`ys workspace delete`.** Remove a workspace from the command line: the folder and the record
  go, the branch stays. Uncommitted work is refused unless `--force` is given, and a harness can
  delete the workspace it is running in — the command does not stop that process. On Windows it
  steps out of the folder first, and deletes nothing if another program still has that folder as
  its current directory. See [The `ys` command line](docs/guide/cli.md).

- **Worktrees made elsewhere no longer appear by themselves.** Yardsort used to turn every
  worktree of a repository into a workspace the moment it looked; a project with worktrees of
  its own — from a script, another tool, or your own `git worktree add` — opened full of
  workspaces nobody asked for. Now only worktrees under Yardsort's own folder are picked up (so a
  removed and re-added project still gets its workspaces back), and the rest wait for
  **Import worktrees…** in the project menu or the composer, which lists them with branch and
  path before anything is recorded. Workspaces adopted by earlier versions are still there: use
  the new **Forget…** in the workspace menu to take one out of Yardsort without touching its
  folder or branch. Its saved conversations are kept unless you tick the box to delete them. See
  [Worktrees made elsewhere](docs/guide/workspaces.md#worktrees-made-elsewhere).

- **Removing a project keeps its workspaces and conversations for when it comes back.** The
  guide always said a removed project's workspaces return when the folder is added again; in
  fact only the worktrees did, as empty workspaces, and the saved conversations were gone. Now
  the project is hidden rather than deleted, so adding the same folder brings back every
  workspace — imported worktrees included — with its history. The remove dialog has a box to
  delete Yardsort's record instead. Nothing on disk is touched either way. See
  [The project menu](docs/guide/projects.md#the-project-menu).

- **Links open in your browser again.** Opening a URL from the app — the release notes on an
  update, the agent install links on the welcome screen, and `Ctrl`/`⌘`-clicking a URL in a
  terminal — was refused with nothing shown but a line in a console nobody sees. The permission
  granted the command but carried no URL scope, which denies everything. Found by pressing a
  pull request badge that had just been made clickable.

- **Have a model write the commit message or the pull request.** A **✦** beside the commit box
  and in the pull request dialog fills them in from the diff, and from what the workspace was
  asked to do. What comes back goes in the box for you to read and edit — it is never committed
  or opened for you. It uses the coding agent you already have, in its non-interactive mode, so
  there is no new account and no new key; your own Anthropic API key is the fallback when no
  configured agent can write. Off with one switch, and absent entirely when nothing can write.
  See [Have it written for you](docs/guide/commits-and-pull-requests.md#have-it-written-for-you).

- **Commit, push and open a pull request from the panel.** Under the changed-files list: a
  message box that commits everything the workspace has changed, a **Push** button once the
  branch is ahead of its remote, and **Open pull request** — which pushes first if it needs to,
  and fills the title and description in from the commits the remote has not got yet: one
  commit's own message body becomes the description, several become a list. Committing is
  confirmed first, naming the files and the branch it lands on. With the
  [GitHub CLI](https://cli.github.com) it opens the pull request from here; without it the branch
  is pushed and your browser opens the forge's own form, which works for GitHub, GitLab,
  Bitbucket and the Gitea family alike. Yardsort holds no forge credentials of its own. See
  [Commits & pull requests](docs/guide/commits-and-pull-requests.md).

- **A workspace's pull request and its checks, on its row.** The number appears in the sidebar
  once there is one, coloured by CI — green when every check has finished and passed, amber while
  one is running, red if one failed — and says **merged** or **closed** when it is over. One
  request per project, not one per workspace. Press it to open the pull request in your browser.
  Needs `gh`; without it nothing shows and nothing complains. See [On the workspace row](docs/guide/commits-and-pull-requests.md#on-the-workspace-row).

## 0.9.2

- **A waiting update is announced beside Settings.** The **update** pill at the foot of the
  projects panel appears when the daily check finds a newer version, and opens the update dialog;
  it replaces the button that used to sit in the status bar. Coming back to the window also looks
  again when the last check is a day old, so a laptop that slept through it still finds out. See
  [Updates](docs/guide/updates.md).

- **Every agent has a mark of its own.** Claude Code, Codex, Grok and OpenCode each get a small
  icon, shown wherever an agent is named: the tab strip and its agent buttons, the composer's
  picker, the session history and Settings → Harnesses. A harness you add yourself gets its
  initial in the same frame. See [Harnesses](docs/guide/settings.md#harnesses).

- **Workspace rows say what their agents are up to.** A number at the end of the row counts the
  agents waiting for you — filled in solid if they finished while you were elsewhere — a **✓**
  means they have all ended, and a **✗** that one exited with an error. Nothing to click. See
  [The badge on a workspace row](docs/guide/terminals-and-sessions.md#the-badge-on-a-workspace-row).

## 0.9.1

- **Drop a file on the composer to point the first message at it.** The same gesture that pastes
  a path into a running terminal now inserts the path into the message you start a workspace
  with, at the cursor. Several files land as several paths. See
  [Starting one: the composer](docs/guide/workspaces.md#starting-one-the-composer).

## 0.9.0

- **`Shift+Enter` adds a line in an agent's terminal.** It used to send the message, because the
  terminal encoded it as a plain `Enter`. It is now sent the way kitty, Ghostty and foot send it,
  which Claude Code, Codex, Grok and OpenCode all read as a new line. Shells still get `Enter`.
- **Drop a file on a terminal to give the agent its path.** The terminal is outlined while you
  hover, and the path — several, if you drop several — is pasted at the cursor, quoted for the
  shell. See [Typing to an agent](docs/guide/terminals-and-sessions.md#typing-to-an-agent).

## 0.8.2

- **Windows: an agent installed while Yardsort is running is now found by "Check again".** It
  never was — a process's environment on Windows is fixed when it starts, and the installer
  writes the new `PATH` where only new processes look, so re-reading our own environment told us
  nothing and a restart was the only cure. `PATH` is now read back from the registry and merged
  with the one we were launched with, so nothing a terminal had added is lost.

## 0.8.1

- **A first message starting with `-` no longer stops the agent.** Pasting a bullet out of a list
  — `- Commit / push / open PR from the UI` — put the agent's own argument parser in charge of it,
  and Claude Code, Codex and Grok each refused to start with `unknown option` or
  `unexpected argument`. The prompt is now passed after `--`, which is how a parser is told the
  options have ended. `ys workspace new` accepted no such message either, and does now.

## 0.8.0

- **`ys attach`.** Put a running agent on your terminal from the command line: the screen is
  repainted where it got to, and what you type reaches it. `Ctrl-]` detaches and leaves it
  running. The window can have the same session open at the same time. See
  [The `ys` command line](docs/guide/cli.md).
- **`ys logs`.** Print what a session has on its screen as plain text, finished ones included —
  how an agent ended up, without opening the window. Screens are held by the background process
  and never written to disk, so they last as long as it does.

## 0.7.0

- **A command line: `ys`.** Start a workspace and put an agent in it without opening the window —
  `ys workspace new <project> "<prompt>"` — plus `project list`, `workspace list`,
  `session list` and `doctor`. It reads the same database as the app, so each sees the other's
  work, and the agent it starts belongs to the background process, so it keeps going after the
  command returns. A separate download in each release; `--json` on every command, for scripts.
  See [The `ys` command line](docs/guide/cli.md).
- **The terminal responds faster.** Output was held for up to 8 ms before being sent to the
  window, including a single keystroke echoing back into a terminal that was otherwise idle —
  where there was nothing to batch it with and the delay bought nothing. It is now held only
  while output is actually streaming, which is what the batching was for. Typing and
  quick-responding prompts feel noticeably more immediate; agents flooding the screen are
  batched exactly as before.
- Workspace and branch names look for the task after introductory context in the first message,
  skipping common request lead-ins and fenced code instead of always taking the opening words.
- Internally, the core is now its own crate with no Tauri in it, which is what lets a
  command-line client exist without carrying a webview around.

## 0.6.0

- **`local` asks what to open.** Clicking a project's own checkout used to open a shell by itself.
  It now offers **Open Terminal** or **Open Composer**, and the composer runs the agent in that
  checkout on the branch you have out — no branch and no worktree are created. It only asks when
  there is nothing running there and nothing to resume. See
  [Projects](docs/guide/projects.md#the-local-workspace).
- **Side-by-side diffs.** A diff opens inline as before; **Side by side** in the viewer's header
  puts the old and new versions in two panes, which is easier to read on a wide change. Your
  choice is remembered. See [Changes & files](docs/guide/changes-and-files.md).
- Hovering a workspace no longer shows its path in a tooltip; the bottom bar already says it for
  the workspace you are in. Archived and missing workspaces keep theirs — they cannot be opened,
  so it is the only place their path is written.

## 0.5.0

- **Agents keep working when you close Yardsort.** Terminals moved out of the app into a small
  background process (`yardsortd`) that owns them, so closing the window — or Yardsort crashing —
  no longer stops anything. Open it again and every terminal is repainted where it got to, and
  conversations that never stopped are no longer listed as _interrupted_.
- Closing Yardsort with agents still running now asks, naming them: leave them running, stop
  them, or cancel. Leaving them running is the default, and shells are never counted as work.
- The status bar shows the background process, and `YARDSORT_NO_DAEMON=1` turns it off.
- A contact address for questions and support, `hello@yardsort.sh`, on the website and in
  the docs.

## 0.4.0

- **Assist (optional).** With a TypeSafe API key of your own, Yardsort can check a workspace's
  changed files against what the agent was asked to do — badging files that look off-task, or that
  add a secret, weaken a test or switch a check off — and suggest a harness and an effort for the
  message you are typing. Off until you enter a key and tick a feature; the key is kept in your
  system credential store, never in `settings.toml`. Agent output is still never read. See
  [Assist](docs/guide/assist.md).
- Settings → Harnesses gains **Good at**: your own description of what a harness suits, used only
  by Assist's composer suggestion.
- Assist's thresholds — how sure Jev must be before a badge or a suggestion appears — are settings,
  with **Restore defaults**. Changing one re-reads answers already given instead of asking again.
- **Worktrees removed with git are noticed.** Delete a workspace's worktree _and_ its branch
  yourself, and Yardsort marks the workspace **gone** and asks whether to delete it from the app
  too — a "keep" is remembered. See [Workspaces](docs/guide/workspaces.md#when-the-branch-goes-too).
- **Linux AppImage: nothing from the replaced version follows an update.** After updating in place,
  terminals still carried the previous AppImage's `LD_LIBRARY_PATH` and friends — the new app is
  started by the old one, so it inherits them — and git kept loading libraries out of the version
  that had just been replaced. Paths into any AppImage's mount are now dropped, not only our own.

## 0.3.2

- Install with Homebrew on macOS: `brew install --cask joaoh82/yardsort/yardsort`.
- Linux: the quick start shows how to give the AppImage a launcher entry and icon.
- Submitted to winget (`joaoh82.Yardsort`), pending Microsoft's review.
- **Linux AppImage: native Wayland.** The AppImage used to run under XWayland, where typing lagged
  and dictation (Omarchy's voice input, anything built on `wtype`) dropped or garbled characters.
  It now opens a Wayland window, and falls back to X11 by itself if that ever fails.
  `YARDSORT_GDK_BACKEND` picks one explicitly.
- **Linux AppImage: terminals get your own environment.** Shells and agents no longer inherit the
  AppImage's private variables (`LD_LIBRARY_PATH`, `PYTHONHOME`, `GDK_BACKEND`, …), which broke
  `python3` and made git print library warnings.

## 0.3.1

- A changelog (this file).
- First release delivered through the in-app updater: a 0.3.0 install offers it by itself.

## 0.3.0

- **In-app updates.** Yardsort looks for new versions shortly after starting and once a day, shows
  an **Update to x.y.z** button, and installs on request — signed and verified. The macOS app,
  Windows installers and the Linux AppImage update themselves; `.deb`, `.rpm` and AUR installs are
  told a new version exists. See [Updates](docs/guide/updates.md).
- **First-run check.** The welcome screen says whether git and at least one agent were found and,
  if not, how to install them — with **Check again**, no restart needed. The status bar's
  `env: …` indicator re-reads your environment from anywhere.
- Settings → General gains **Check for updates automatically** and **Check now**.

## 0.2.0

- **Renamed from Switchyard to Yardsort.** Projects, workspaces, session history and settings are
  carried over automatically on first launch. New branches are prefixed `ys/`; `sy/` branches
  keep working, as do the `SWITCHYARD_*` environment variables.
- An AUR package, `yardsort-bin`, is prepared and will be published by the release workflow.

## 0.1.0

First public release, as _Switchyard_.

- Projects and workspaces: every task gets its own git worktree and branch.
- Real terminals running the agent of your choice — Claude Code, Codex, Grok, OpenCode, or any
  terminal agent you configure.
- Live list of changed files with diffs, a file tree, and one click into your editor.
- Resume and fork agent conversations; status dots and desktop notifications.
- Rename, archive, restore and delete workspaces, never losing uncommitted work silently.
- Linux, macOS (signed and notarized) and Windows builds.

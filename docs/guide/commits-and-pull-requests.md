# Commits & pull requests

Once you have read what an agent did, the rest of the job is sending it somewhere. The foot of
the [changes panel](changes-and-files.md) commits it, pushes it and opens the pull request,
without leaving Yardsort.

Everything here is ordinary git and, for the pull request, the
[GitHub CLI](https://cli.github.com). Nothing is stored, no account is created, and Yardsort
never holds a credential of its own.

## Commit

When a workspace has uncommitted changes, a message box appears under the list with a
**Commit N files** button beside it. Type a message and press it — or press Enter in the box.

**Every commit is confirmed first**, naming how many files it takes, the message, and the branch
they land on. There is no exception for a branch that looks safe: the project's own `local`
checkout is usually sitting on `main`, and a rule with an exception is one you have to learn.

**It commits everything**, including files git has not seen before. There is no way to leave one
out, on purpose: a partial commit you did not ask for is a worse surprise than one more file than
you expected. If you need to split a change, use a shell tab (`Ctrl+Shift+T`) or your git client;
the workspace is an ordinary checkout at the path in the footer.

The message is passed to git as a single argument, so a message beginning with `--` is a message,
not an option.

If git has no name and email configured, the button stays disabled and says so rather than
failing halfway. Set them once:

```sh
git config --global user.name "Your Name"
git config --global user.email "you@example.com"
```

### Have it written for you

A **✦** sits beside the message box. Press it and a model writes the message from the diff you
are looking at — and from what the workspace was asked to do, which is what turns "describe this
change" into "explain it".

What comes back goes **in the box**, for you to read and edit. It is never committed for you; the
confirmation still stands between it and git.

The box takes a full message: Enter makes a new line, and `Ctrl+Enter` / `⌘Enter` commits.

Two things can do the writing, tried in that order:

1. **The agent you already have**, in its non-interactive mode — `claude --print`, `codex exec`,
   `opencode run`, `grok --single`. It is installed, logged in and billed to the account it
   already uses, and it is the agent this workspace has been working with. A harness you added
   yourself needs its **Write** arguments filled in under Settings → Harnesses before it can.
2. **Your own Anthropic API key**, when no configured agent can write. Add it under
   Settings → Assist; `ANTHROPIC_API_KEY` from your environment works too.

Switch the whole thing off under Settings → Assist if you would rather not be offered it. With no
agent able to write and no key, the ✦ does not appear at all.

## Push

**Push N commits** appears when the branch is ahead of the remote — of its upstream once it has
one, and of the base branch before the first push. It pushes to `origin`, or to the only remote
if there is no `origin`, and sets the upstream the first time so that later pushes are ordinary.

The push is never forced. Yardsort can only ever add to what the remote already has.

Credentials are git's: your SSH key, or whatever credential helper you already use. Yardsort
runs git with prompts disabled, so a push that needs an answer nobody can see fails with git's
own message instead of hanging.

## Open a pull request

**Open pull request** — **Open merge request** on GitLab — asks for a title, a description and
whether it should be a draft, then opens it.

The same **✦** is here, above the fields: it writes the title and the description together, from
everything the branch has committed. As with a commit message, it fills the fields in and leaves
them to you.

Without pressing it, the form starts filled in from the commits the remote has not got yet. The
oldest becomes the title. For the description:

- **One commit** — its own message body, everything under the subject line. That commit _is_ the
  pull request, so a well-written one needs no second write-up. It is what `gh pr create --fill`
  does too.
- **Several** — a list of their subjects, oldest first, in the order they happened.

Both are a starting point; edit either before pressing the button.

**It pushes first if it needs to.** A branch the forge has never seen cannot have a pull request,
and a button that fails and tells you to press a different one is not worth having.

### With `gh`

If the [GitHub CLI](https://cli.github.com) is installed and logged in, the pull request is opened
from here and your browser lands on the finished thing. `gh` keeps your GitHub credentials, in the
place you already manage and revoke them; Yardsort never asks for a token of its own.

```sh
gh auth login    # once, if you have not already
```

### Without it

Everything still works, one step longer: the branch is pushed and your browser opens the forge's
own "open a pull request" form with both branches already filled in. Finish it there.

This is the path for GitHub, GitLab, Bitbucket, Gitea and Forgejo alike, and for a self-hosted
host Yardsort has never heard of — it assumes GitHub's URL shapes, which the Gitea family uses
too. A remote that is a path on disk is not a forge, so the button does not appear at all.

## On the workspace row

Once a workspace has a pull request, its number appears on its row in the sidebar, coloured by
what CI made of it:

| On the row | Means                                                        |
| ---------- | ------------------------------------------------------------ |
| **#42**    | Open. Grey — no checks configured, or none has reported yet. |
| **#42**    | Green — every check finished and passed.                     |
| **#42**    | Amber — at least one check is still running.                 |
| **#42**    | Red — at least one check failed.                             |
| **merged** | Merged. The workspace is done with; archive or delete it.    |
| **closed** | Closed without merging.                                      |
| dimmed     | It is a draft.                                               |

One failure outranks everything, and anything still running outranks success — so green always
means _finished_ and passing. Hovering gives the whole story, including the title.

**Press the badge to open the pull request in your browser.** It sits beside the row rather than
on it, so pressing it opens the pull request while pressing the row still opens the workspace. The
same summary sits at the right of the panel foot for the selected workspace, and does the same.

**This needs `gh`.** Without it there is no number and no check result anywhere in the app, and
nothing complains about that: it is a supported way to use Yardsort, not a fault. Logged out of
`gh`, or a repository `gh` does not recognise, is the same — no badges, no error. If you were
expecting numbers and there are none, the pull request dialog says what `gh` actually replied.

Yardsort asks the forge once per project, not once per workspace, and reuses the answer for half
a minute. It asks again when you come back to the window, every minute while it is open, and
straight after anything you do that changes the answer.

## What it does not do

No merging, no rebasing, no review comments, no CI logs — and no staging area. Those are on the
[roadmap](../design/05-roadmap.md) or deliberately left to the tools that do them well. Yardsort
gets the work out of the workspace; the forge takes it from there.

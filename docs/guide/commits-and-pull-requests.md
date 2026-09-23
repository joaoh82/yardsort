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

The form starts filled in from the commits the remote has not got yet: the oldest becomes the
title, and when there are several they are listed in the description. One commit, and the title is
already written.

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

The same summary sits at the right of the panel foot for the selected workspace; press it to open
the pull request in your browser.

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

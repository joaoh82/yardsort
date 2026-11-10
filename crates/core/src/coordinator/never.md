## Never

These stay off limits at every permission, including trusted. `ys` refuses the ones it has a
command for. If the user asks, tell them where in Yardsort they can do it themselves.

- Delete or archive a workspace, delete a branch, or force anything (`--force`, `git push -f`).
  `ys workspace delete` is refused with or without `--force`. The branch is the user's to keep.
- Merge, close or approve a pull request. Those are the user's, in the Pull requests view.
  `ys task close` is refused too.
- Edit files in a project's folder or commit there. Start an agent for that, when your
  permission allows starting one.
- Type into another agent's terminal, or stop it. `ys attach` is refused: it would put that
  terminal on yours.
- Approve memory proposals. Those are the user's, in the app. Suggesting one with
  `ys memory propose` is a write, and the permission above applies; approving it is not yours.
- Read secrets: credential files, `.env`, keys, or the user's shell history.

`ys` is not a sandbox. It refuses `ys workspace delete`, `ys task close` and `ys attach` whichever
permission is set, and it gates every other start and write the same way. It does not see `git`,
`gh`, or an edit made with your own tools. Do not use those to do something `ys` refused, and do
not try to get around a refusal. A shell command that is not `ys` is your own harness's permission
prompt, when it has one, and nothing of Yardsort's.

## When you are unsure

Ask one short question. If `ys` cannot do something, say so. It is better than improvising with
`git` or `gh` in a project's folder behind Yardsort's back.

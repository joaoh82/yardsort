# Assist

Assist is optional help from an AI model that does not write anything: it answers small, typed
questions about your changes and about the message you are typing, and Yardsort turns the answers
into badges and suggestions. It is off until you enter an API key and tick a box.

It uses [Jev](https://docs.typesafe.ai), TypeSafe's judgment model. Jev never generates text — it
returns a probability, a choice, or a level, and Yardsort decides what that means. Requests are
billed to your own TypeSafe account; Yardsort asks for the pinned model `jev-1.13.0`.

**Yardsort never sends your terminals anywhere.** Status dots, readiness and notifications still
come from terminal activity alone, and Assist never reads what an agent printed.

![Settings → Assist](../images/assist.png)

## Getting a key

1. Get an API key from the TypeSafe console.
2. **Settings → Assist**, paste it, press **Save**. Yardsort checks the key with TypeSafe before
   keeping it.
3. Tick the features you want.

The key is kept in your system credential store — Keychain on macOS, Credential Manager on
Windows, the Secret Service (GNOME Keyring, KWallet) on Linux — and never in `settings.toml`. It is
never shown again; only the last four characters are, so you can tell keys apart. **Test** asks
TypeSafe whether it still works, **Forget** removes it.

If your Linux session has no Secret Service running, Settings says so and Yardsort falls back to
the `TYPESAFE_API_KEY` environment variable — the same variable TypeSafe's own SDKs read. A key you
save in Settings takes precedence over that variable.

## Checking changed files

_Settings → Assist → "Check changed files against what the workspace was asked to do"._

Shortly after an agent stops writing, the [Changes](changes-and-files.md) list is checked file by
file, and flagged files get a badge:

| Badge           | What it means                                                                        |
| --------------- | ------------------------------------------------------------------------------------ |
| **off-task**    | The change looks unrelated to what this workspace was asked to do.                   |
| **secret**      | The change looks like it adds a literal key, token or password.                      |
| **tests**       | The change looks like it deletes, skips or weakens a test.                           |
| **checks**      | The change looks like it switches a lint, type check or CI step off.                 |
| **credentials** | The file's _name_ says it holds credentials. Its contents were not sent (see below). |

The line above the list says when Assist last looked, and **Check now** asks again immediately.
Hovering a badge explains it.

"What was asked" is the first message of each of this workspace's conversations. A workspace
Yardsort has no message for — an adopted worktree, or an existing branch you opened — still gets
the risk checks; the line above the list says so.

**What is sent:** the diff of each changed file, that file's path, and the task. Very long diffs
are cut short. Binary files and files too large to show are skipped. Files whose name says they
hold credentials (`.env`, `*.pem`, `*.key`, `id_ed25519`, `.npmrc`…) are badged without their
contents ever being sent — `.env.example` and friends are not treated as secret.

Answers are cached per file: watching an agent work re-asks only about the file whose diff
actually changed.

## Suggestions in the composer

_Settings → Assist → "Suggest a harness and an effort in the composer"._

While you type the first message, Assist can offer a harness and an effort level. Press **Use** to
apply it; ignore it and nothing happens. Nothing is ever picked for you, and a suggestion appears
only when the model is reasonably sure.

The harness suggestion is built on **your** descriptions, not on Yardsort's opinion of any agent:
fill in **Good at** for two or more harnesses in [Settings → Harnesses](settings.md), for example
"long refactors and tricky debugging" or "quick, well-specified edits". Harnesses without a
description are never suggested.

The effort suggestion comes from how demanding the request looks, mapped onto the effort levels
that harness offers.

**What is sent:** the message you are typing (once typing pauses, and only from about 15
characters), and the "Good at" descriptions.

## Tuning what gets flagged

_Settings → Assist → "How sure Jev must be"._

Jev answers with probabilities, and these three numbers decide when a probability is worth your
attention. The defaults are a starting point, not a measurement — tune them against your own work.

| Threshold                          | Default | Meaning                                                                                    |
| ---------------------------------- | ------- | ------------------------------------------------------------------------------------------ |
| **Flag a risky change at**         | 70%     | How sure Jev must be that a change adds a secret, weakens a test or switches a check off.  |
| **Call a file off-task at**        | 60%     | How much of the answer must say "unrelated to the task" before the off-task badge appears. |
| **Offer a composer suggestion at** | 50%     | How sure Jev must be about a harness or a difficulty before the composer offers it.        |

Lower numbers catch more and cry wolf more; higher ones stay quiet and miss more. Each is a
percentage between 5 and 95, and **Restore defaults** puts all three back.

**Changing a threshold costs nothing.** Yardsort caches Jev's answers, not the badges, so the new
numbers are applied to answers it already has — no new requests, no waiting.

## When something goes wrong

Nothing here is load-bearing. Without a key, switched off, offline, or rate-limited, Yardsort works
exactly as it does otherwise — you just get no badges and no suggestions. Failures appear as a
quiet line above the change list, never as a blocked action.

| Message                                        | What to do                                                        |
| ---------------------------------------------- | ----------------------------------------------------------------- |
| "TypeSafe did not accept the API key"          | The key is wrong or revoked. Save a new one.                      |
| "TypeSafe is rate limiting…"                   | Too many requests; it retries a few times, then gives up quietly. |
| "The system credential store is not available" | Start a keyring, or set `TYPESAFE_API_KEY`.                       |

## Settings file

Only the switches are stored, in `settings.toml`; the key never is.

```toml
[assist]
review_changes = true
suggest_in_composer = true
flag_at_percent = 70
off_task_at_percent = 60
suggest_at_percent = 50
```

Only what differs from the defaults is written, so a value you never touched keeps following
Yardsort's default if that ever changes.

## Writing commit messages and pull requests

Assist does not write text. Jev answers typed questions — a probability, a choice, a score — and
that is all it does; there is no wording anywhere in this page that came from a model.

Having a model _write_ a commit message or a pull request is a separate thing with its own switch,
settled at the foot of this settings page because that is where optional AI lives. It uses the
coding agent you already have, or your own Anthropic API key. See
[Commits & pull requests](commits-and-pull-requests.md#have-it-written-for-you).

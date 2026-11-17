# Security

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private reporting
instead: [**Report a vulnerability**](https://github.com/joaoh82/yardsort/security/advisories/new).

Include what you found, how to reproduce it, and the version and OS. You will get an
acknowledgement within a few days, and credit in the fix's release notes unless you prefer
otherwise.

Only the latest release is supported with security fixes.

## What Yardsort does and does not do

Knowing the design helps judge what is a vulnerability:

- Yardsort **launches programs you configured** — coding agents and your shell — in folders you
  chose, with your login shell's environment. An agent can do whatever its own permission model
  allows; that is the agent's security boundary, not Yardsort's.
- It stores **no credentials**, has no account and sends **no telemetry**. Agents use their own
  logins. Its only network request is the **update check**: a download of `latest.json` from this
  repository's GitHub releases, shortly after start and once a day (it can be switched off).
- **Updates are signed.** An update is installed only if its signature verifies against the public
  key built into the app, and only after the user asks for it.
- Programs are started with an argument list, **never through a shell**, so prompts and settings
  cannot inject commands.
- When browsing or diffing a workspace, the webview names the workspace by id and gives a
  relative path; the core resolves it and refuses anything that leaves the workspace folder.
- **The host dashboard**, where it is served ([guide](docs/guide/host-dashboard.md)), listens on
  `127.0.0.1` only. It takes a request only when it is addressed to a loopback name or to its own
  tunnel, comes from its own page, and carries a session bought with a one-time code, which is
  handed out only to a person at that computer. A signed-in browser can run commands there as
  that user: that is its purpose, and the one place a line someone typed is handed to a shell. It
  can let no device in, and a command it runs is refused what only a person at a terminal may
  ask of the host.

Reports about breaking those guarantees — path traversal out of a workspace, command injection
through a prompt or a harness field, a webview escape, anything that would let an unsigned or
downgraded update install, a way into the host dashboard without a sign-in code, or to a code
without being a person at that computer — are exactly what we want to hear about.

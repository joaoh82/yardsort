# Yardsort vs Superset

Yardsort and Superset both organize parallel coding work around agents and git workspaces.
The useful choice is about where you work and how much coordination you need around the agent.
Yardsort offers a local desktop workflow across Linux, macOS and Windows. Superset is worth
considering when its remote access, automation and team integrations match your requirements.

This comparison concerns the coding-agent app at **superset.sh**, not Apache Superset or the
hiring platform. It is a documentation comparison by the Yardsort project, not a speed or
quality benchmark.

## The main differences

| Area                          | Yardsort                                                                | Superset                                                                         |
| ----------------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Desktop platforms             | Linux, macOS and Windows                                                | macOS supported; Linux x64 AppImage experimental and untested; Windows planned   |
| Agents                        | Installed CLI agents, including Claude Code and Codex; custom harnesses | Bring CLI agents such as Claude Code, Codex and OpenCode                         |
| Workspaces                    | Local git worktrees; existing worktrees can be imported                 | Git worktrees; imports supported                                                 |
| Sessions after closing the UI | Choose **Leave them running**; a local daemon keeps terminals alive     | Background daemon preserves terminals through app restarts                       |
| Remote and team work          | Current desktop workflow is local                                       | Pro includes remote access, automations, Linear and Slack integrations           |
| App cost                      | Free, GPL-3.0; provider charges separate                                | Free individual plan; Pro $20/user/month monthly or $15/user/month billed yearly |

Sources: [Yardsort overview](/docs/guide/questions/), [Superset FAQ](https://docs.superset.sh/faq)
and [Superset pricing](https://superset.sh/pricing). Prices are USD as listed on the verification
date. Check the linked plans before purchasing; agent access is separate from workspace pricing.

## What a review looks like

In Yardsort, choose a workspace to inspect files and diffs, commit reviewed changes, push and
open a PR. GitHub check and review status appears alongside the workspace when `gh` is available.
The current documentation leaves review comments and CI logs to the forge or other tools.
See [commits and pull requests](/docs/guide/commits-and-pull-requests/).

Superset documents a broader PR dashboard: repository and review filters, checks, comment
threads and a diff view. Selected diff lines can be sent to an agent, optionally also posted to
GitHub; a PR can be checked out into a workspace for follow-up. If moving between several PRs is
a large part of your day, that is a concrete reason to try it.
[Superset PR documentation](https://docs.superset.sh/pull-requests).

For a second opinion inside Yardsort, the [built-in review workflow](/docs/guide/workflows/)
provides a repeatable review process, and [handoffs](/docs/guide/terminals-and-sessions/#handing-work-to-another-agent)
prepare editable context for another agent. Those are useful when the unit of work is a
particular local branch rather than an organization-wide PR queue.

## Local execution and data handling

Yardsort keeps its workspace and session records locally. Agents can contact their configured
providers, and optional Assist sends the context described in its [guide](/docs/guide/assist/).
Local execution is not a promise that all code remains offline.

Superset's FAQ distinguishes local files from remote features: relay traffic, cloud workspaces
and published Pages have different data paths. Its agents also contact model providers.
Read the [privacy section of its FAQ](https://docs.superset.sh/faq#privacy) for your intended setup.

Neither app's background process can make a powered-off local computer keep executing code.
Decide whether you need persistent sessions on a running workstation or execution on another host.

## Which would we try first?

Try **Yardsort** first if you move among Windows, Linux and Mac, want to use agents in their own
terminal interfaces, and value local handoffs, memory and inspectable YAML workflows. Its open
source is also relevant if you want to inspect or modify the implementation.

Try **Superset** first if its documented remote and team features solve a daily problem for you,
or if its PR dashboard is central to how you review work. Its free plan also means a local trial
does not require immediately choosing a paid team plan.

Use the same repository and two modest tasks for either trial. Check environment setup,
permissions, review effort and recovery after a restart. Those observations are more useful than
counting agent logos. For the Yardsort path, follow the [parallel-agents tutorial](/tutorials/claude-code-codex-parallel-worktrees/).

Also considering Conductor? Read [Yardsort vs Conductor](/blog/yardsort-vs-conductor/).

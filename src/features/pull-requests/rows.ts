import type { Project, ProjectPullRequests, PullRequest, Workspace } from "@/lib/ipc";
import { pullRequestsFor } from "@/stores/publish";

/** A pull request in the list: the request, the project it belongs to, and what links them. */
export interface Row {
  /** Unique across projects: two repositories both have a #1. */
  key: string;
  project: Project;
  pr: PullRequest;
  /** Who `gh` is logged in as for this project's forge: what "you" means for this row. */
  viewer: string | null;
  /** The workspace this pull request came out of or was started in, when there is one to go to. */
  workspace: Workspace | null;
}

export const rowKey = (projectId: string, number: number) => `${projectId}#${number}`;

/** When a pull request last changed, for ordering. One with no time on it sorts last. */
function changedAt(pr: PullRequest): number {
  const updated = pr.details?.updatedAt ? Date.parse(pr.details.updatedAt) : NaN;
  return Number.isNaN(updated) ? (pr.createdAt ?? 0) : updated;
}

/**
 * Every project's pull requests as one list, most recently changed first.
 *
 * Derived, never stored: the core's answer per project is the only copy, so a row here cannot
 * say something the workspace's own badge does not.
 */
export function rowsOf(projects: Project[], byProject: Record<string, ProjectPullRequests>): Row[] {
  const rows: Row[] = [];
  for (const project of projects) {
    const found = byProject[project.id];
    if (project.missing || !found) continue;
    // Which workspace each pull request belongs to: the core's call, through the same function
    // the workspace rows use. A folder that is gone is not somewhere to go.
    const owner = new Map<number, Workspace>();
    for (const workspace of project.workspaces) {
      if (workspace.archived || workspace.missing) continue;
      for (const pr of pullRequestsFor(found, workspace)) {
        if (!owner.has(pr.number)) owner.set(pr.number, workspace);
      }
    }
    for (const pr of found.pullRequests) {
      rows.push({
        key: rowKey(project.id, pr.number),
        project,
        pr,
        viewer: found.viewer,
        workspace: owner.get(pr.number) ?? null,
      });
    }
  }
  return rows.sort((a, b) => changedAt(b.pr) - changedAt(a.pr) || b.pr.number - a.pr.number);
}

/** How many open pull requests a project's list holds. */
export const openIn = (found: ProjectPullRequests | undefined): number =>
  found?.pullRequests.filter((pr) => pr.state === "open").length ?? 0;

/** The forge has more open pull requests than the list holds: it stops at a cap. */
export const moreOpenThanListed = (found: ProjectPullRequests | undefined): boolean =>
  !!found && found.openTotal !== null && found.openTotal > openIn(found);

/** How long ago, in the shortest form that is still unambiguous: `5m`, `3h`, `2d`, `3w`, `1y`. */
export function age(at: number | null, now: number): string {
  if (at === null) return "";
  const minutes = Math.max(0, Math.floor((now - at) / 60_000));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 14) return `${days}d`;
  if (days < 60) return `${Math.floor(days / 7)}w`;
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}

/** `12/12`: checks that passed out of all that reported. Empty when none did. */
export function checksSummary(pr: PullRequest): string {
  const counts = pr.details?.checkCounts;
  if (!counts) return "";
  const total = counts.passed + counts.failed + counts.running;
  return total === 0 ? "" : `${counts.passed}/${total}`;
}

/** The same summary for a screen reader, which cannot see the colour that says which way it went. */
export function checksLabel(pr: PullRequest): string {
  const counts = pr.details?.checkCounts;
  const total = counts ? counts.passed + counts.failed + counts.running : 0;
  if (!counts || total === 0) return "no checks reported";
  const parts = [`${counts.passed} of ${total} checks passed`];
  if (counts.failed > 0) parts.push(`${counts.failed} failed`);
  if (counts.running > 0) parts.push(`${counts.running} still running`);
  return parts.join(", ");
}

/** `owner/name`, or nothing for a project whose remote is not a forge. */
export const repoName = (found: ProjectPullRequests | undefined): string =>
  found?.repo ? `${found.repo.owner}/${found.repo.name}` : "";

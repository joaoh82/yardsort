import { openUrl } from "@tauri-apps/plugin-opener";
import type { ResolvedLink } from "@/lib/ipc";
import { rowKey as pullRequestKey } from "@/features/pull-requests/rows";
import { rowKey as taskKey } from "@/features/tasks/rows";
import { usePublishStore } from "@/stores/publish";
import { useProjectsStore } from "@/stores/projects";
import { usePullRequestsStore } from "@/stores/pullRequests";
import { useTasksStore } from "@/stores/tasks";

/**
 * Open what a resolved link points at, in the project that has its repository: the Tasks view
 * on that issue, or the Pull requests view on that pull request, so Delegate or Start workspace
 * is the next press. A project on another forge has no such view, so the item opens in the
 * browser; so does one the view turns out to have no row for — closed, or past what is read —
 * and a fork's own issue in a clone whose views answer for the parent.
 *
 * `projectId` is the link's own unless the repository was just cloned, in which case it is the
 * new project's — and a fresh clone of the link's repository lists it. Returns whether a view
 * was opened.
 */
export async function followLink(
  link: ResolvedLink,
  projectId: string | null = link.projectId,
): Promise<boolean> {
  if (!projectId) return false;
  const browser = () => void openUrl(link.url).catch(console.error);
  // Another forge has no view here; and a clone of a fork lists its parent's issues, not the
  // fork's own, so a link to the fork is known but has no row — and #3 of the parent is not it.
  if (link.repo.kind !== "github" || !link.listed) {
    browser();
    return false;
  }
  // Read before opening anything: a view with nothing selected in it helps nobody, so an item
  // the list has no row for — closed, or past what is read — goes to the browser instead.
  const projects = useProjectsStore.getState();
  if (link.kind === "issue") {
    const key = `#${link.number}`;
    await useTasksStore.getState().loadProject(projectId);
    const listed = useTasksStore
      .getState()
      .byProject[projectId]?.tasks.some((task) => task.key === key);
    if (listed) {
      projects.openTasks(true);
      useTasksStore.getState().select(taskKey(projectId, key));
      return true;
    }
  } else {
    await usePublishStore.getState().loadProject(projectId, false, true);
    const listed = usePublishStore
      .getState()
      .byProject[projectId]?.pullRequests.some((pr) => pr.number === link.number);
    if (listed) {
      projects.openPullRequests(true);
      usePullRequestsStore.getState().select(pullRequestKey(projectId, link.number));
      return true;
    }
  }
  browser();
  return false;
}

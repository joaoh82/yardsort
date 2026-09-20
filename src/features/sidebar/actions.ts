import { native } from "@/lib/native";
import { errorMessage, type Project, type Workspace } from "@/lib/ipc";
import { recall, useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";

/**
 * Run a user-initiated action so that a failure is *seen*. These are fired from click handlers
 * with nobody awaiting them; without this, a rejected dialog or IPC call would vanish and the
 * click would simply appear to do nothing.
 */
async function visibly<T>(action: () => Promise<T>, fallback: T): Promise<T> {
  try {
    return await action();
  } catch (error) {
    console.error(error);
    useProjectsStore.setState({ error: errorMessage(error) });
    return fallback;
  }
}

/**
 * Select a workspace because the user asked for it. A workspace with nothing running gets a
 * shell, so that choosing one always lands somewhere useful. (Restoring the selection at startup
 * goes through the store directly and spawns nothing.)
 */
export function enterWorkspace(workspaceId: string) {
  useProjectsStore.getState().select(workspaceId);
  void visibly(async () => {
    const hasTabs = () =>
      useTerminalStore.getState().tabs.some((tab) => tab.workspaceId === workspaceId);
    if (hasTabs()) return;
    // A workspace with conversations to come back to shows them instead: opening a shell on
    // top would bury the Resume button the user most likely came for.
    const past = await useSessionsStore.getState().load(workspaceId);
    if (past.length === 0 && !hasTabs()) await useTerminalStore.getState().open(workspaceId);
  }, undefined);
}

/** Pick a folder and add it, offering to initialise git if it is not a repository yet. */
export const openProjectFromDisk = (): Promise<boolean> =>
  visibly(openProjectFromDiskUnguarded, false);

async function openProjectFromDiskUnguarded(): Promise<boolean> {
  const folder = await native.pickFolder("Open project");
  if (!folder) return false;

  const projects = useProjectsStore.getState();
  let result = await projects.openFolder(folder);
  if (result.status === "needs-git") {
    const agreed = await native.confirm(
      `${folder}\n\nis not a git repository. Yardsort needs one: every workspace is a git worktree.\n\nInitialise git here? This runs "git init" and creates an empty first commit. Your files are not changed.`,
      { title: "Initialise git?", okLabel: "Initialise git" },
    );
    if (!agreed) return false;
    result = await projects.openFolder(folder, true);
  }
  if (result.status !== "added") return false;

  const selected = useProjectsStore.getState().selectedWorkspaceId;
  if (selected) enterWorkspace(selected);
  return true;
}

export const removeProject = (project: Project): Promise<void> =>
  visibly(() => removeProjectUnguarded(project), undefined);

async function removeProjectUnguarded(project: Project) {
  const running = useTerminalStore
    .getState()
    .tabs.filter((tab) => !tab.exit && project.workspaces.some((w) => w.id === tab.workspaceId));
  const agreed = await native.confirm(
    `Remove "${project.name}" from Yardsort?\n\nNothing on disk is deleted — the folder and its git history stay exactly as they are.` +
      (running.length > 0
        ? `\n\n${running.length} running terminal session${running.length === 1 ? "" : "s"} in this project will be closed.`
        : ""),
    { title: "Remove project", okLabel: "Remove" },
  );
  if (!agreed) return;
  await useTerminalStore.getState().closeWorkspaces(project.workspaces.map((w) => w.id));
  await useProjectsStore.getState().remove(project.id);
}

/** Workspaces the user chose to keep after being told there is nothing left to restore them from. */
const KEPT_KEY = "workspaces.keptVanished";

/** One question at a time: the dialog blurs the window, and regaining focus is what asks again. */
let asking = false;

/**
 * Notice worktrees the user removed with git — folder *and* branch — and offer to forget the
 * workspaces they leave behind. Those have nothing to restore from and nothing to open; letting
 * them go is all Yardsort can still do with them, and it never does that uninvited.
 *
 * Runs after every project refresh, so the answer is remembered: a workspace the user kept is
 * not raised again, and that answer is dropped once it is no longer in question.
 */
export const reviewVanishedWorkspaces = (): Promise<void> =>
  visibly(reviewVanishedWorkspacesUnguarded, undefined);

async function reviewVanishedWorkspacesUnguarded() {
  if (asking) return;
  const store = useProjectsStore.getState();
  if (!store.loaded) return;

  const vanished = store.projects
    .flatMap((project) => project.workspaces)
    .filter((workspace) => workspace.missing && workspace.branchGone);
  const inQuestion = new Set(vanished.map((workspace) => workspace.id));

  // Forget answers about workspaces that are settled — deleted since, or checked out again with
  // their branch — so that a folder which disappears a second time is asked about a second time.
  const kept = recall<string[]>(store.ui, KEPT_KEY, []);
  const answered = kept.filter((id) => inQuestion.has(id));
  const unasked = vanished.filter((workspace) => !answered.includes(workspace.id));
  if (unasked.length === 0) {
    if (answered.length !== kept.length) store.remember(KEPT_KEY, answered);
    return;
  }

  asking = true;
  try {
    const agreed = await native.confirm(vanishedMessage(unasked), {
      title: unasked.length === 1 ? "Workspace gone" : "Workspaces gone",
      okLabel: unasked.length === 1 ? "Delete workspace" : "Delete them",
      cancelLabel: "Keep",
    });
    if (!agreed) {
      store.remember(KEPT_KEY, [...answered, ...unasked.map((workspace) => workspace.id)]);
      return;
    }
    await useTerminalStore.getState().closeWorkspaces(unasked.map((workspace) => workspace.id));
    for (const workspace of unasked) {
      await useProjectsStore.getState().deleteWorkspace(workspace.id);
    }
    if (answered.length !== kept.length) store.remember(KEPT_KEY, answered);
  } finally {
    asking = false;
  }
}

function vanishedMessage(workspaces: Workspace[]): string {
  const [only] = workspaces;
  if (workspaces.length === 1 && only) {
    return (
      `"${only.name}" is gone. Its folder was removed outside Yardsort, and there is no branch ` +
      "left to check out in its place.\n\nDelete the workspace and its session history from " +
      "Yardsort too? Nothing on disk is touched — it is already gone."
    );
  }
  return (
    `${workspaces.length} workspaces are gone. Their folders were removed outside Yardsort, and ` +
    `there are no branches left to check out in their place:\n\n` +
    `${workspaces.map((workspace) => `  \u2022 ${workspace.name}`).join("\n")}\n\n` +
    "Delete these workspaces and their session history from Yardsort too? Nothing on disk is " +
    "touched — they are already gone."
  );
}

/** Open the composer for a new workspace in the project the user is currently looking at. */
export function composeInCurrentProject() {
  const { projects, selectedWorkspaceId, composingProjectId, compose } =
    useProjectsStore.getState();
  const current =
    projects.find((p) => p.workspaces.some((w) => w.id === selectedWorkspaceId)) ??
    projects.find((p) => p.id === composingProjectId) ??
    projects[0];
  if (current && !current.missing) compose(current.id);
}

/**
 * Delete a worktree workspace: its folder goes, its branch stays. Uncommitted work is never
 * destroyed without a second, explicit confirmation that says so.
 */
export const deleteWorkspace = (workspace: Workspace): Promise<void> =>
  visibly(() => deleteWorkspaceUnguarded(workspace), undefined);

async function deleteWorkspaceUnguarded(workspace: Workspace) {
  const branch = workspace.head && !workspace.head.detached ? workspace.head.label : null;
  const agreed = await native.confirm(
    `Delete workspace "${workspace.name}"?\n\nThis removes its folder:\n${workspace.path}\n\n` +
      (branch
        ? `The branch "${branch}" and all its commits are kept.`
        : "Its commits stay in the repository.") +
      " Terminals running in this workspace will be closed.",
    { title: "Delete workspace", okLabel: "Delete" },
  );
  if (!agreed) return;

  await useTerminalStore.getState().closeWorkspaces([workspace.id]);
  const projects = useProjectsStore.getState();
  if ((await projects.deleteWorkspace(workspace.id)) !== "dirty") return;

  const force = await native.confirm(
    `"${workspace.name}" has uncommitted changes or untracked files.\n\nDeleting it now destroys that work for good — it is in no commit and cannot be recovered.`,
    { title: "Uncommitted work will be lost", okLabel: "Delete anyway" },
  );
  if (force) await projects.deleteWorkspace(workspace.id, true);
}

/**
 * Archive a workspace: the folder goes, the branch and the session history stay, and it can be
 * restored later. Uncommitted work gets the same explicit second confirmation as deleting.
 */
export const archiveWorkspace = (workspace: Workspace): Promise<void> =>
  visibly(() => archiveWorkspaceUnguarded(workspace), undefined);

async function archiveWorkspaceUnguarded(workspace: Workspace) {
  const agreed = await native.confirm(
    `Archive workspace "${workspace.name}"?\n\nIts folder is removed:\n${workspace.path}\n\nThe branch, its commits and the session history are kept, and "Restore" brings the workspace back in the same place. Terminals running in it will be closed.`,
    { title: "Archive workspace", okLabel: "Archive" },
  );
  if (!agreed) return;

  await useTerminalStore.getState().closeWorkspaces([workspace.id]);
  const projects = useProjectsStore.getState();
  if ((await projects.archiveWorkspace(workspace.id)) !== "dirty") return;

  const force = await native.confirm(
    `"${workspace.name}" has uncommitted changes or untracked files.\n\nArchiving removes the folder, and that work with it — it is in no commit and cannot be recovered. Commit it first if you want to keep it.`,
    { title: "Uncommitted work will be lost", okLabel: "Archive anyway" },
  );
  if (force) await projects.archiveWorkspace(workspace.id, true);
}

/** Bring an archived or vanished workspace back and go to it. */
export const restoreWorkspace = (workspace: Workspace): Promise<void> =>
  visibly(async () => {
    if (await useProjectsStore.getState().restoreWorkspace(workspace.id)) {
      enterWorkspace(workspace.id);
    }
  }, undefined);

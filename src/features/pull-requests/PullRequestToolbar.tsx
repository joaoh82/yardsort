import { useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { HoverCard } from "@/lib/HoverCard";
import { errorMessage, ipc, type MergeMethod, type Workspace } from "@/lib/ipc";
import { native } from "@/lib/native";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import { PullRequestBadge } from "@/features/sidebar/PullRequestBadge";
import { pullRequestsFor, usePublishStore } from "@/stores/publish";
import { useTerminalStore } from "@/stores/terminals";
import { PullRequestDetails } from "./PullRequestDetails";
import { conflicting, pullRequestColour } from "./appearance";

const methods: { method: MergeMethod; label: string }[] = [
  { method: "squash", label: "Squash and merge" },
  { method: "merge", label: "Create a merge commit" },
  { method: "rebase", label: "Rebase and merge" },
];

export function PullRequestToolbar({
  workspace,
  projectId,
}: {
  workspace: Workspace;
  projectId: string;
}) {
  const found = usePublishStore((s) => s.byProject[projectId]);
  // A workspace can have opened several. The toolbar shows one at a time — the branch's own
  // until another is chosen from the menu.
  const prs = pullRequestsFor(found, workspace);
  const [chosen, setChosen] = useState<number | null>(null);
  const pr = prs.find((candidate) => candidate.number === chosen) ?? prs[0];
  const more = prs.length - 1;
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const menuButton = useRef<HTMLButtonElement>(null);
  async function action(run: () => Promise<void>) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await run();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }
  async function merge(method: MergeMethod, label: string) {
    if (!pr?.details) return;
    const confirmed = await native.confirm(
      `${label} pull request #${pr.number}: ${pr.title}\n\n${pr.branch} → ${pr.details.base}\nCommit ${pr.details.headOid.slice(0, 12)}\n\nGitHub may queue the merge if this branch requires it. Your workspace and local branch will be kept.`,
      { title: "Merge pull request", okLabel: label },
    );
    if (!confirmed) return;
    try {
      await ipc.workspaceMergePullRequest(workspace.id, pr.number, pr.details.headOid, method);
      setNotice("Merge request sent. GitHub may queue it; refresh to check its status.");
    } finally {
      await usePublishStore.getState().loadProject(projectId, true);
      if (usePublishStore.getState().workspaceId === workspace.id)
        await usePublishStore.getState().refresh();
    }
  }
  async function resolveConflicts() {
    if (!pr) return;
    const helper = await ipc.workspaceConflictHelper(workspace.id, pr.number);
    const base = pr.details?.base ?? "its base";
    const how =
      helper.reach === "type"
        ? `It is running in “${helper.title}”, and the request is typed in there.`
        : helper.reach === "resume"
          ? `Its conversation “${helper.title}” has ended. It is resumed in a new tab, with the request.`
          : `Its conversation “${helper.title}” cannot be continued, so a new ${helper.harnessLabel} conversation starts with the request and the workspace's task.`;
    const confirmed = await native.confirm(
      `Ask ${helper.harnessLabel} to resolve the conflicts in pull request #${pr.number}: ${pr.title}\n\n${how}\n\nIt is asked to merge ${base} into ${pr.branch}, resolve the conflicts, run the project's checks and push — never to rebase or force-push.`,
      { title: "Resolve merge conflicts", okLabel: `Ask ${helper.harnessLabel}` },
    );
    if (!confirmed) return;
    try {
      const terminals = useTerminalStore.getState();
      const asked = await ipc.workspaceResolveConflicts(
        workspace.id,
        pr.number,
        terminals.lastSize,
      );
      if (asked.reach === "type") terminals.activate(asked.session.id);
      else terminals.adopt(asked.session);
      setNotice(`Asked ${helper.harnessLabel} to resolve the conflicts in #${pr.number}.`);
    } finally {
      // The command asked the forge afresh; the badge should say what it said.
      await usePublishStore.getState().loadProject(projectId, true);
    }
  }
  // Keep feedback visible even when refresh removes the badge.
  if (!pr && !error && !notice) return null;
  return (
    <div className="relative ml-auto flex shrink-0 items-center gap-0.5">
      {pr && (
        <>
          <HoverCard
            label={`Pull request #${pr.number} details`}
            content={<PullRequestDetails pr={pr} />}
          >
            <PullRequestBadge pr={pr} toolbar />
          </HoverCard>
          <button
            ref={menuButton}
            type="button"
            aria-label={`Actions for pull request #${pr.number}${more > 0 ? ` — ${more} more from this workspace` : ""}`}
            aria-haspopup="menu"
            aria-expanded={!!at}
            disabled={busy}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setAt(at ? null : { x: box.right - 210, y: box.bottom + 4 });
            }}
            className={`rounded px-1.5 py-1 hover:brightness-125 disabled:opacity-40 ${pullRequestColour(pr)}`}
          >
            {busy ? "…" : <>{more > 0 && <span className="mr-1 tabular-nums">+{more}</span>}▾</>}
          </button>
          {at && (
            <ContextMenu
              at={at}
              onClose={() => {
                setAt(null);
                menuButton.current?.focus();
              }}
              items={[
                // More than one: say which, and let another be chosen.
                ...(more > 0
                  ? prs.map((candidate) => ({
                      label: `#${candidate.number} ${candidate.state === "open" ? (candidate.draft ? "draft" : "open") : candidate.state} — ${candidate.title}`,
                      checked: candidate.number === pr.number,
                      onSelect: () => setChosen(candidate.number),
                    }))
                  : []),
                ...(conflicting(pr)
                  ? [
                      {
                        label: "Ask its agent to resolve conflicts…",
                        divider: more > 0,
                        disabled: busy || workspace.missing || workspace.archived,
                        onSelect: () => void action(resolveConflicts),
                      },
                    ]
                  : []),
                ...methods.map(({ method, label }, index) => ({
                  divider: index === 0 && more > 0 && !conflicting(pr),
                  label,
                  disabled:
                    busy ||
                    pr.state !== "open" ||
                    pr.draft ||
                    !pr.details?.headOid ||
                    workspace.missing ||
                    workspace.archived,
                  onSelect: () => void action(() => merge(method, label)),
                })),
                { label: "View on GitHub", onSelect: () => void action(() => openUrl(pr.url)) },
                { label: "Copy PR link", onSelect: () => void action(() => writeText(pr.url)) },
                {
                  label: "Refresh pull request",
                  onSelect: () =>
                    void action(async () => {
                      await usePublishStore.getState().loadProject(projectId, true);
                      const problem = usePublishStore.getState().byProject[projectId]?.problem;
                      if (problem) throw new Error(problem);
                    }),
                },
              ]}
            />
          )}
        </>
      )}
      {(error || notice) && (
        <div className="absolute top-full right-0 z-30 mt-2 w-80 rounded border border-line bg-raised p-3 shadow-xl">
          <p
            role={error ? "alert" : "status"}
            className={error ? "text-red-400" : "text-ink-muted"}
          >
            {error || notice}
          </p>
          <button
            type="button"
            onClick={() => {
              setError(null);
              setNotice(null);
            }}
            className="mt-2 text-ink-muted hover:text-ink"
          >
            Dismiss
          </button>
        </div>
      )}
    </div>
  );
}

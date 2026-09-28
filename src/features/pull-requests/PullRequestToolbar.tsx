import { useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { HoverCard } from "@/lib/HoverCard";
import { errorMessage, ipc, type MergeMethod, type Workspace } from "@/lib/ipc";
import { native } from "@/lib/native";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import { PullRequestBadge } from "@/features/sidebar/PullRequestBadge";
import { pullRequestFor, usePublishStore } from "@/stores/publish";
import { PullRequestDetails } from "./PullRequestDetails";
import { pullRequestColour } from "./appearance";

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
  const pr = pullRequestFor(
    found,
    workspace.head && !workspace.head.detached ? workspace.head.label : undefined,
  );
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
            aria-label={`Actions for pull request #${pr.number}`}
            aria-haspopup="menu"
            aria-expanded={!!at}
            disabled={busy}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setAt(at ? null : { x: box.right - 210, y: box.bottom + 4 });
            }}
            className={`rounded px-1.5 py-1 hover:brightness-125 disabled:opacity-40 ${pullRequestColour(pr)}`}
          >
            {busy ? "…" : "▾"}
          </button>
          {at && (
            <ContextMenu
              at={at}
              onClose={() => {
                setAt(null);
                menuButton.current?.focus();
              }}
              items={[
                ...methods.map(({ method, label }) => ({
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

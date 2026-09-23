import { openUrl } from "@tauri-apps/plugin-opener";
import { useState } from "react";
import type { ChangeSet } from "@/lib/ipc";
import { native } from "@/lib/native";
import { usePublishStore, type Busy } from "@/stores/publish";
import { PullRequestDialog } from "./PullRequestDialog";

/**
 * The foot of the changes panel: commit what the agent wrote, push it, open the pull request.
 *
 * It is the last third of the loop the rest of Yardsort exists for — an agent writes, you read
 * the diff, and until now you went to a terminal to send it anywhere. Three buttons in the order
 * the work goes, never more than one of them interesting at a time.
 */
export function PublishBar({ changes }: { changes: ChangeSet | null }) {
  const state = usePublishStore((s) => s.state);
  const busy = usePublishStore((s) => s.busy);
  const error = usePublishStore((s) => s.error);
  const [opening, setOpening] = useState(false);

  const uncommitted = changes?.uncommitted.length ?? 0;
  // Nothing below applies to a detached HEAD, and the panel above already says what is going on.
  if (!state || !state.branch) return null;

  const unpushed = state.ahead;
  const canPush = state.remote !== null && unpushed > 0;
  const noun = state.repo?.kind === "gitlab" ? "merge request" : "pull request";

  return (
    <div className="mt-auto shrink-0 border-t border-line p-2">
      <CommitBox
        // Remounted when the last uncommitted file goes, which clears a half-typed message the
        // agent has just committed out from under. Steady while files come and go above zero.
        key={uncommitted > 0 ? "changed" : "clean"}
        uncommitted={uncommitted}
        identity={state.identity}
        branch={state.branch}
        busy={busy}
      >
        {canPush && (
          <button
            type="button"
            disabled={busy !== null}
            title={`Push ${state.branch} to ${state.remote}`}
            onClick={() => void usePublishStore.getState().push()}
            className="rounded border border-line px-3 py-1 hover:bg-raised disabled:opacity-40"
          >
            {busy === "push"
              ? "Pushing…"
              : `Push ${unpushed} ${unpushed === 1 ? "commit" : "commits"}`}
          </button>
        )}

        {state.canOpen && (
          <button
            type="button"
            disabled={busy !== null}
            onClick={() => setOpening(true)}
            className="rounded border border-line px-3 py-1 hover:bg-raised disabled:opacity-40"
          >
            {busy === "pullRequest" ? "Opening…" : `Open ${noun}`}
          </button>
        )}

        <PullRequestLink />
      </CommitBox>

      {error && (
        <p role="alert" className="mt-2 text-red-400 select-text">
          {error}
        </p>
      )}
      {opening && <PullRequestDialog onClose={() => setOpening(false)} />}
    </div>
  );
}

/**
 * The commit message and the buttons beside it.
 *
 * It owns the message so that unmounting is what clears it — an effect watching the file count
 * would set state during a render pass, and the message has exactly one reason to disappear.
 */
function CommitBox({
  uncommitted,
  identity,
  branch,
  busy,
  children,
}: {
  uncommitted: number;
  identity: string | null;
  branch: string;
  busy: Busy | null;
  children: React.ReactNode;
}) {
  const [message, setMessage] = useState("");
  const canCommit = uncommitted > 0 && message.trim() !== "" && identity !== null;

  /**
   * Every commit is confirmed, naming what it takes and where it lands.
   *
   * It takes *everything* uncommitted, and on a project's own checkout "everything" can land on
   * the branch the rest of the work is measured against. One press is too few for that, and a
   * rule with an exception is one the user has to learn.
   */
  const commit = async () => {
    const agreed = await native.confirm(
      `Commit ${uncommitted === 1 ? "1 file" : `all ${uncommitted} files`} to "${branch}"?` +
        `\n\n${message.trim()}` +
        "\n\nEverything uncommitted goes in, including files git has not seen before.",
      {
        title: uncommitted === 1 ? "Commit 1 file" : `Commit ${uncommitted} files`,
        okLabel: "Commit",
      },
    );
    if (agreed) await usePublishStore.getState().commit(message.trim());
  };

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (canCommit) void commit();
      }}
    >
      {uncommitted > 0 && (
        <>
          <input
            aria-label="Commit message"
            value={message}
            maxLength={200}
            placeholder="Commit message"
            spellCheck={false}
            autoComplete="off"
            disabled={busy !== null}
            onChange={(event) => setMessage(event.target.value)}
            className="w-full rounded border border-line bg-canvas px-2 py-1 outline-none select-text focus:border-accent disabled:opacity-50"
          />
          {identity === null && (
            <p className="mt-1 text-[11px] text-ink-faint">
              git has no name and email to commit with yet. Set them with{" "}
              <code className="select-text">git config --global user.name</code> and{" "}
              <code className="select-text">user.email</code>.
            </p>
          )}
        </>
      )}

      <div className={`flex flex-wrap items-center gap-2 ${uncommitted > 0 ? "mt-2" : ""}`}>
        {uncommitted > 0 && (
          <button
            type="submit"
            disabled={!canCommit || busy !== null}
            className="rounded bg-accent px-3 py-1 font-medium text-canvas disabled:opacity-40"
          >
            {busy === "commit"
              ? "Committing…"
              : uncommitted === 1
                ? "Commit 1 file"
                : `Commit ${uncommitted} files`}
          </button>
        )}
        {children}
      </div>
    </form>
  );
}

/** The pull request this workspace already has, once it has one. */
function PullRequestLink() {
  const pr = usePublishStore((s) => s.state?.pullRequest ?? null);
  if (!pr) return null;
  const said =
    pr.state === "merged"
      ? "merged"
      : pr.state === "closed"
        ? "closed"
        : pr.checks === "failing"
          ? "checks failing"
          : pr.checks === "running"
            ? "checks running"
            : pr.checks === "passing"
              ? "checks passing"
              : "open";
  return (
    <button
      type="button"
      onClick={() => void openUrl(pr.url).catch(console.error)}
      title={pr.title}
      className="ml-auto truncate text-ink-faint hover:text-ink"
    >
      #{pr.number} · {said}
    </button>
  );
}

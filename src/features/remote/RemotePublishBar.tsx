import { useEffect, useId, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  errorMessage,
  ipc,
  type OtherMergeMethod,
  type OtherPublishState,
  type OtherPullRequest,
} from "@/lib/ipc";
import { native } from "@/lib/native";

type Busy = "commit" | "push" | "pullRequest" | "merge";

/**
 * Under a workspace's changes on another computer: commit them, push the branch, open its pull
 * request and merge it. That computer does each, as itself: with its own Git identity, its own
 * credentials and its own forge sign-in. This computer sends the words and the press.
 *
 * Each is one press by the person, never repeated by itself. When that computer's answer is
 * lost with the link, the bar says it may have been done and reads how the workspace stands.
 */
export function RemotePublishBar({
  hostId,
  workspaceId,
  name,
  uncommitted,
  revision,
  onChanged,
}: {
  hostId: string;
  workspaceId: string;
  /** What that computer is called. */
  name: string;
  /** How many files are uncommitted there, as the list above shows. */
  uncommitted: number;
  /** Changes when that computer was read again: how the workspace stands is read again too. */
  revision: number | null;
  /** Something was committed there: the list above reads again. */
  onChanged: () => void;
}) {
  const [state, setState] = useState<OtherPublishState | null>(null);
  const [busy, setBusy] = useState<Busy | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [opening, setOpening] = useState(false);
  const [opened, setOpened] = useState<string | null>(null);
  const [method, setMethod] = useState<OtherMergeMethod>("squash");

  // The forge is asked when the bar opens and after what moves it, done or refused; a reading in
  // between asks Git alone, and keeps what the forge last said about the pull request. So does
  // one the forge did not answer: that says nothing about the pull request, not that it is gone.
  const take = (next: OtherPublishState, forge: boolean) =>
    setState((before) =>
      forge && !next.problem ? next : { ...next, pullRequest: before?.pullRequest ?? null },
    );
  const read = (forge: boolean) =>
    ipc.otherHostPublishState(hostId, workspaceId, forge).then(
      (next) => take(next, forge),
      (reason) => setError(errorMessage(reason)),
    );
  useEffect(() => {
    let stale = false;
    ipc.otherHostPublishState(hostId, workspaceId, true).then(
      (next) => !stale && setState(next),
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [hostId, workspaceId]);
  useEffect(() => {
    if (revision === null) return;
    let stale = false;
    ipc.otherHostPublishState(hostId, workspaceId, false).then(
      (next) => !stale && take(next, false),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [hostId, workspaceId, revision]);

  /** One press: done there, or said why not. Whatever came of it, the bar reads again. */
  const press = async (doing: Busy, action: () => Promise<unknown>, forge: boolean) => {
    if (busy) return false;
    setBusy(doing);
    setError(null);
    try {
      await action();
      await read(forge);
      onChanged();
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      // It may have been done all the same, and a refusal can mean the forge has moved on: a pull
      // request pushed to since it was read is refused at the commit shown here until the forge
      // is read again. So this asks what the press would have asked, and leaves why it was
      // refused in view whether or not that reading arrives.
      await ipc.otherHostPublishState(hostId, workspaceId, forge).then(
        (next) => take(next, forge),
        () => {},
      );
      onChanged();
      return false;
    } finally {
      setBusy(null);
    }
  };

  if (!state || !state.branch) return null;
  const pr = state.pullRequest;
  const canOpen = state.canOpen && pr?.state !== "open";
  const canPush = state.remote && state.ahead > 0;
  const canCommit = uncommitted > 0 && message.trim() !== "" && state.identity !== null;

  const commit = () =>
    press("commit", () => ipc.otherHostCommit(hostId, workspaceId, message), false).then(
      (done) => done && setMessage(""),
    );

  return (
    <div
      role="group"
      aria-label={`Send on from ${name}`}
      className="shrink-0 border-t border-line p-2"
    >
      <textarea
        aria-label="Commit message"
        placeholder={
          uncommitted === 0
            ? "Nothing to commit"
            : `Commit ${uncommitted} ${uncommitted === 1 ? "file" : "files"} on ${name}`
        }
        value={message}
        rows={2}
        disabled={busy !== null || uncommitted === 0}
        onChange={(event) => setMessage(event.target.value)}
        className="w-full resize-none rounded border border-line bg-canvas p-2 outline-none select-text focus:border-accent disabled:opacity-50"
      />
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy !== null || !canCommit}
          title={
            state.identity
              ? `Commit everything changed, on ${state.branch}, as ${state.identity}`
              : `Git has no name and email to commit with on ${name}`
          }
          onClick={() => void commit()}
          className="rounded bg-accent px-3 py-1.5 font-medium text-canvas disabled:opacity-40"
        >
          {busy === "commit" ? "Committing…" : "Commit"}
        </button>
        {canPush && (
          <button
            type="button"
            disabled={busy !== null}
            title={`Push ${state.branch} from ${name} to its remote`}
            onClick={() => void press("push", () => ipc.otherHostPush(hostId, workspaceId), true)}
            className="rounded border border-line px-3 py-1.5 hover:bg-raised disabled:opacity-40"
          >
            {busy === "push"
              ? "Pushing…"
              : `Push ${state.ahead} ${state.ahead === 1 ? "commit" : "commits"}`}
          </button>
        )}
        {canOpen && !opening && (
          <button
            type="button"
            disabled={busy !== null}
            onClick={() => setOpening(true)}
            className="rounded border border-line px-3 py-1.5 hover:bg-raised disabled:opacity-40"
          >
            Open pull request
          </button>
        )}
      </div>

      {opening && (
        <OpenForm
          name={name}
          gh={state.gh}
          busy={busy === "pullRequest"}
          suggested={state.unpushed.at(-1) ?? ""}
          onCancel={() => setOpening(false)}
          onOpen={async (title, body, draft) => {
            let made: { url: string; created: boolean } | null = null;
            const done = await press(
              "pullRequest",
              async () => {
                made = await ipc.otherHostOpenPullRequest(hostId, workspaceId, title, body, draft);
              },
              true,
            );
            if (!done || !made) return;
            setOpening(false);
            const { url, created } = made as { url: string; created: boolean };
            // Not opened there: the forge's own form, to finish in this computer's browser.
            if (created) setOpened(null);
            else {
              setOpened(url);
              void openUrl(url).catch(console.error);
            }
          }}
        />
      )}
      {opened && (
        <p className="mt-2 text-small text-ink-faint">
          {name} could not open it itself, so the forge&rsquo;s form was opened in your browser to
          finish there.
        </p>
      )}

      {pr && (
        <PullRequestLine
          pr={pr}
          base={state.base}
          gh={state.gh}
          busy={busy}
          method={method}
          onMethod={setMethod}
          onMerge={async () => {
            const head = pr.headOid;
            if (!head) return;
            const agreed = await native.confirm(
              `Merge pull request #${pr.number} into ${state.base ?? "its base"}?\n\n"${pr.title}"\n\n` +
                `${name} asks the forge to ${method} it, only if it is still at the commit shown now (${head.slice(0, 7)}). Its branch is not deleted.`,
              { title: "Merge pull request", okLabel: "Merge" },
            );
            if (!agreed) return;
            await press(
              "merge",
              () => ipc.otherHostMergePullRequest(hostId, workspaceId, pr.number, head, method),
              true,
            );
          }}
        />
      )}

      {state.identity === null && (
        <p className="mt-2 text-small text-amber-300">
          Git has no name and email on {name}, so nothing can be committed there yet.
        </p>
      )}
      {state.problem && <p className="mt-2 text-small text-ink-faint">{state.problem}</p>}
      {error && (
        <p role="alert" className="mt-2 text-red-400 select-text">
          {error}
        </p>
      )}
    </div>
  );
}

/** The title and description of a pull request to open there. */
function OpenForm({
  name,
  gh,
  busy,
  suggested,
  onCancel,
  onOpen,
}: {
  name: string;
  gh: boolean;
  busy: boolean;
  suggested: string;
  onCancel: () => void;
  onOpen: (title: string, body: string, draft: boolean) => Promise<void>;
}) {
  const [title, setTitle] = useState(suggested);
  const [body, setBody] = useState("");
  const [draft, setDraft] = useState(false);
  const titleId = useId();
  return (
    <form
      aria-label="Open pull request"
      className="mt-2 grid gap-2 rounded border border-line p-2"
      onSubmit={(event) => {
        event.preventDefault();
        void onOpen(title, body, draft);
      }}
    >
      <label htmlFor={titleId} className="text-small text-ink-faint">
        Title
      </label>
      <input
        id={titleId}
        value={title}
        maxLength={256}
        disabled={busy}
        onChange={(event) => setTitle(event.target.value)}
        className="h-8 rounded border border-line bg-canvas px-2 outline-none select-text focus:border-accent"
      />
      <textarea
        aria-label="Description"
        placeholder="What it changes, and why"
        value={body}
        rows={4}
        disabled={busy}
        onChange={(event) => setBody(event.target.value)}
        className="resize-none rounded border border-line bg-canvas p-2 outline-none select-text focus:border-accent"
      />
      <label className="flex items-center gap-2 text-small text-ink-muted">
        <input
          type="checkbox"
          checked={draft}
          disabled={busy}
          onChange={(event) => setDraft(event.target.checked)}
          className="accent-(--color-accent)"
        />
        Open as a draft
      </label>
      <p className="text-small text-ink-faint">
        {gh
          ? `${name} pushes the branch if its remote lacks any of it, then opens the pull request with its own forge sign-in.`
          : `${name} has no program to open it with, so it pushes the branch and the forge's own form opens in your browser.`}
      </p>
      <div className="flex justify-end gap-2">
        <button type="button" onClick={onCancel} className="px-3 py-1.5 text-ink-muted">
          Cancel
        </button>
        <button
          type="submit"
          disabled={busy || title.trim() === ""}
          className="rounded bg-accent px-3 py-1.5 font-medium text-canvas disabled:opacity-40"
        >
          {busy ? "Opening…" : "Open"}
        </button>
      </div>
    </form>
  );
}

const CHECKS: Record<string, string> = {
  passing: "checks passing",
  failing: "checks failing",
  running: "checks running",
};

/** The workspace's pull request: what it is, a way to its page, and merging it. */
function PullRequestLine({
  pr,
  base,
  gh,
  busy,
  method,
  onMethod,
  onMerge,
}: {
  pr: OtherPullRequest;
  base: string | null;
  gh: boolean;
  busy: Busy | null;
  method: OtherMergeMethod;
  onMethod: (method: OtherMergeMethod) => void;
  onMerge: () => Promise<void>;
}) {
  const mergeable = pr.state === "open" && !pr.draft && !pr.conflicting && gh && !!pr.headOid;
  const standing = [
    pr.draft && pr.state === "open" ? "draft" : pr.state,
    CHECKS[pr.checks],
    pr.conflicting ? "conflicts with " + (base ?? "its base") : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      role="group"
      aria-label={`Pull request #${pr.number}`}
      className="mt-2 flex flex-wrap items-center gap-2 text-small"
    >
      <button
        type="button"
        disabled={!pr.url}
        title={pr.url ?? undefined}
        // Its address came from that computer: only a web page is opened.
        onClick={() => pr.url && void openUrl(pr.url).catch(console.error)}
        className="min-w-0 truncate text-left text-ink underline decoration-line underline-offset-2 hover:decoration-accent disabled:no-underline"
      >
        #{pr.number} {pr.title}
      </button>
      <span className="text-ink-faint">{standing}</span>
      {mergeable && (
        <span className="ml-auto flex items-center gap-1">
          <select
            aria-label="How to merge"
            value={method}
            disabled={busy !== null}
            onChange={(event) => onMethod(event.target.value as OtherMergeMethod)}
            className="h-7 rounded border border-line bg-canvas px-1 text-ink-muted"
          >
            <option value="squash">squash</option>
            <option value="merge">merge commit</option>
            <option value="rebase">rebase</option>
          </select>
          <button
            type="button"
            disabled={busy !== null}
            onClick={() => void onMerge()}
            className="rounded border border-line px-2 py-1 hover:bg-raised disabled:opacity-40"
          >
            {busy === "merge" ? "Merging…" : "Merge…"}
          </button>
        </span>
      )}
    </div>
  );
}

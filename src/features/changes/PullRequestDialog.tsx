import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useRef, useState } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { useDraftStore } from "@/stores/draft";
import { usePublishStore } from "@/stores/publish";
import { DraftButton } from "./PublishBar";

/**
 * What a pull request should say, out of the commits nobody has seen yet.
 *
 * One commit and it *is* the pull request, so its own message body is the description — the
 * same thing `gh pr create --fill` does, and it means a well-written commit needs no second
 * write-up. Several, and no one body speaks for them, so they are listed oldest first, in the
 * order they happened.
 */
function describe(commits: { subject: string; body: string }[]): string {
  if (commits.length === 0) return "";
  if (commits.length === 1) return commits[0]!.body;
  return commits
    .map((commit) => `- ${commit.subject}`)
    .reverse()
    .join("\n");
}

/**
 * Title, description and draft, then hand it to `gh`.
 *
 * Without `gh` — or logged out of it — nothing here is sent anywhere: the branch is pushed and
 * the browser opens the forge's own form, which is the same fields in the place that owns them.
 */
export function PullRequestDialog({
  workspaceId,
  onClose,
}: {
  workspaceId: string;
  onClose: () => void;
}) {
  const state = usePublishStore((s) => s.state);
  const busy = usePublishStore((s) => s.busy);
  const error = usePublishStore((s) => s.error);
  // Newest first, as git prints them; the oldest is the one the branch is about.
  const commits = state?.unpushed ?? [];

  const [title, setTitle] = useState(commits.at(-1)?.subject ?? state?.branch ?? "");
  const [body, setBody] = useState(() => describe(commits));
  const [draft, setDraft] = useState(false);
  const titleId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    inputRef.current?.select();
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  if (!state) return null;
  const noun = state.repo?.kind === "gitlab" ? "merge request" : "pull request";
  const ahead = state.ahead;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const opened = await usePublishStore
      .getState()
      .openPullRequest({ title: title.trim(), body, draft });
    if (!opened) return; // The store holds the reason; it is shown below.
    await openUrl(opened.url).catch(console.error);
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[16vh]"
      onPointerDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={submit}
        className="w-[34rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold first-letter:uppercase">
          Open {noun}
        </h2>
        <p className="mt-1 text-ink-faint">
          <span className="font-mono">{state.branch}</span> into{" "}
          <span className="font-mono">{state.base}</span>
          {state.repo && ` on ${state.repo.host}`}
        </p>

        <div className="mt-4 flex items-end justify-between gap-2">
          <span className="text-ink-muted">Title and description</span>
          <DraftButton
            what="pullRequest"
            label="Write the title and description"
            onWrite={() => useDraftStore.getState().pullRequest(workspaceId)}
            onWritten={(written) => {
              setTitle(written.title);
              setBody(written.body);
            }}
          />
        </div>

        <label className="mt-1 block">
          <span className="sr-only">Title</span>
          <input
            aria-label="Title"
            ref={inputRef}
            value={title}
            maxLength={200}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setTitle(event.target.value)}
            className="mt-1 w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
          />
        </label>

        <label className="mt-3 block">
          <span className="text-ink-muted">Description</span>
          <textarea
            value={body}
            rows={5}
            onChange={(event) => setBody(event.target.value)}
            className="mt-1 w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
          />
        </label>

        <label className="mt-3 flex items-center gap-2">
          <input type="checkbox" checked={draft} onChange={() => setDraft(!draft)} />
          <span>Open it as a draft</span>
        </label>

        {ahead > 0 && (
          <p className="mt-3 text-ink-faint">
            {ahead === 1 ? "One commit" : `${ahead} commits`} will be pushed to{" "}
            <span className="font-mono">{state.remote}</span> first.
          </p>
        )}
        {!state.gh ? (
          <p className="mt-2 text-ink-faint">
            <code className="select-text">gh</code> is not installed, so the branch will be pushed
            and the {noun} form opened in your browser to finish there. Install the GitHub CLI to
            open it from here, and to see its checks on the workspace row.
          </p>
        ) : state.loggedOut ? (
          <p className="mt-2 text-ink-faint">
            <code className="select-text">gh</code> is installed but not logged in, so the browser
            will finish the job. Run <code className="select-text">gh auth login</code> to open it
            from here, and to see its checks on the workspace row.
          </p>
        ) : (
          // Not an error — the branch and the form still work. It is simply the answer to "why
          // is there no number on my row", printed where someone is already asking.
          state.problem && (
            <p className="mt-2 text-ink-faint select-text">
              <code>gh</code> could not read this repository&rsquo;s pull requests, so there are no
              check results on the workspace rows: {state.problem}
            </p>
          )
        )}
        {error && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {error}
          </p>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={title.trim() === "" || busy !== null}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy === "pullRequest"
              ? "Opening…"
              : // Logged out ends in the browser just as a missing gh does, so it must not
                // promise to open anything here.
                state.gh && !state.loggedOut
                ? "Open"
                : "Push and continue"}
          </button>
        </div>
      </form>
    </div>
  );
}

import { useEffect, useId, useRef, useState } from "react";
import { selectWorkspace } from "@/features/keyboard/commands";
import { errorMessage, ipc, type ConflictHelper, type Excerpt, type LinePlace } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useProjectsStore } from "@/stores/projects";
import { usePullRequestsStore, type Target } from "@/stores/pullRequests";
import { useTerminalStore } from "@/stores/terminals";
import type { Row } from "./rows";

/** What pressing Send does with the note, said before it is pressed. */
function where(helper: ConflictHelper | null, row: Row): string {
  if (!row.workspace) {
    return "There is no workspace for this pull request yet. One will be started on its branch, with the note as the agent's first message — the composer opens for you to pick the agent and press Start.";
  }
  if (!helper) return "";
  const who = `${helper.harnessLabel} in “${helper.title}”`;
  switch (helper.reach) {
    case "type":
      return `It is typed into ${who}, which is running and quiet in ${row.workspace.name}. Anything you had typed there and not sent goes with it.`;
    case "resume":
      return `${who} has ended. It is resumed in a new tab in ${row.workspace.name}, with the note.`;
    case "start":
      return `${who} cannot be continued, so a new ${helper.harnessLabel} conversation starts in ${row.workspace.name} with the note and the workspace's task.`;
  }
}

/**
 * Lines of a pull request's diff, a note about them, and where it goes: to the agent that has
 * the pull request's workspace — the way the conflict helper reaches one, never into a busy
 * agent — or, with no workspace, as the first message of a new one. Optionally also to the
 * forge, as a comment on those lines; or only there.
 */
export function NoteDialog({
  row,
  excerpt,
  headOid,
  onClose,
}: {
  row: Row;
  excerpt: Excerpt;
  /** The head the lines are of, which is what a comment on them is placed by. */
  headOid: string;
  onClose: () => void;
}) {
  const target: Target = {
    key: row.key,
    projectId: row.project.id,
    pr: row.pr,
    viewer: row.viewer,
  };
  const [note, setNote] = useState("");
  const [post, setPost] = useState(false);
  const [helper, setHelper] = useState<ConflictHelper | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Whether GitHub has the comment already. Once it does, a retry after some later step failed
  // must not post it a second time.
  const [postedOnGitHub, setPostedOnGitHub] = useState(false);
  // The same two, for the handlers that are not re-created on every render.
  const busyRef = useRef(false);
  const postedRef = useRef(false);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  const noteRef = useRef<HTMLTextAreaElement>(null);
  useModalFocus(dialogRef);
  const workspace = row.workspace;

  // Closing is cancelling, and cancelling sends nothing: once a send is under way, neither Esc
  // nor the backdrop closes the dialog, the same as the disabled Cancel button.
  const cancel = () => {
    if (!busyRef.current) onClose();
  };
  useEffect(() => {
    noteRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busyRef.current) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // Who would be asked, before anything is sent: the same question the conflict helper asks.
  useEffect(() => {
    if (!workspace) return;
    let stale = false;
    ipc.pullRequestNoteHelper(workspace.id, row.pr.number).then(
      (found) => !stale && setHelper(found),
      (reason: unknown) => !stale && setProblem(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [workspace, row.pr.number]);

  const place: LinePlace = {
    commit: headOid,
    path: excerpt.path,
    side: excerpt.side,
    line: excerpt.to,
    startLine: excerpt.from === excerpt.to ? null : excerpt.from,
  };
  const lines =
    excerpt.from === excerpt.to ? `line ${excerpt.from}` : `lines ${excerpt.from}–${excerpt.to}`;
  const ready = note.trim() !== "" && !busy;
  const start = () => {
    busyRef.current = true;
    setBusy(true);
    setProblem(null);
  };
  const failed = (error: unknown) => {
    setProblem(errorMessage(error));
    busyRef.current = false;
    setBusy(false);
  };

  /** Post the note on GitHub, once: the second time through it is already there. */
  const postOnGitHub = async () => {
    if (postedRef.current) return;
    const ok = await usePullRequestsStore.getState().lineComment(target, place, note.trim());
    if (!ok) throw new Error(usePullRequestsStore.getState().error ?? "Could not comment.");
    postedRef.current = true;
    setPostedOnGitHub(true);
  };

  /**
   * GitHub goes first, because GitHub can refuse — it takes a comment only on a line inside the
   * diff's hunks, and the viewer lets you select any line of the file. A refusal before the agent
   * has the note leaves nothing half done: Send tries again, and the agent still gets it once.
   */
  const send = async () => {
    if (!ready) return;
    start();
    try {
      if (workspace) {
        if (!helper) throw new Error("Still finding out who to ask. Try again in a moment.");
        if (post) await postOnGitHub();
        const terminals = useTerminalStore.getState();
        const asked = await ipc.pullRequestSendNote(
          workspace.id,
          row.pr.number,
          helper.sessionId,
          excerpt,
          note.trim(),
          terminals.lastSize,
        );
        if (asked.reach === "type") terminals.activate(asked.session.id);
        else terminals.adopt(asked.session);
        onClose();
        selectWorkspace(workspace.id);
      } else {
        const text = await ipc.pullRequestNoteText(
          row.project.id,
          row.pr.number,
          excerpt,
          note.trim(),
        );
        if (post) await postOnGitHub();
        const prepared = await ipc.pullRequestPrepareBranch(row.project.id, row.pr.number);
        onClose();
        useProjectsStore.getState().compose(row.project.id, prepared, text);
      }
    } catch (error) {
      failed(error);
    }
  };

  const postOnly = async () => {
    if (!ready) return;
    start();
    try {
      await postOnGitHub();
      onClose();
    } catch (error) {
      failed(error);
    }
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[16vh]"
      onPointerDown={(event) => event.target === event.currentTarget && cancel()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
        className="w-[34rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Note on {lines} of {excerpt.path.slice(excerpt.path.lastIndexOf("/") + 1)}
        </h2>
        <p className="mt-1 text-ink-muted">
          <span className="font-mono text-[12px]">{excerpt.path}</span>, {lines}
          {excerpt.side === "left" ? ", as the file was before the pull request" : ""}.
        </p>
        <pre className="mt-3 max-h-40 overflow-auto rounded bg-canvas p-2 font-mono text-[12px] leading-relaxed select-text">
          {excerpt.text}
        </pre>
        <textarea
          ref={noteRef}
          aria-label="Note"
          placeholder="What should be done about these lines?"
          value={note}
          rows={4}
          disabled={busy}
          onChange={(event) => setNote(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              void send();
            }
          }}
          className="mt-3 w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
        />
        <p className="mt-2 text-ink-faint">{where(helper, row)}</p>
        <label className="mt-3 flex items-center gap-2 text-ink-muted">
          <input
            type="checkbox"
            checked={post}
            disabled={busy || postedOnGitHub}
            onChange={(event) => setPost(event.target.checked)}
            className="accent-(--color-accent)"
          />
          {postedOnGitHub
            ? "Posted on GitHub as a comment on these lines. Sending again does not post it twice."
            : "Also post it on GitHub, as a comment on these lines"}
        </label>
        {problem && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {problem}
          </p>
        )}
        <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
          <button
            type="button"
            onClick={cancel}
            disabled={busy}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!ready || postedOnGitHub}
            onClick={() => void postOnly()}
            className="rounded border border-line px-3 py-1.5 hover:bg-raised disabled:opacity-40"
          >
            Post on GitHub only
          </button>
          <button
            type="submit"
            disabled={!ready || (!!workspace && !helper)}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Sending…" : workspace ? "Send to agent" : "Start a workspace with it"}
          </button>
        </div>
      </form>
    </div>
  );
}

import { useEffect, useState } from "react";
import { errorMessage, ipc } from "@/lib/ipc";
import { useDraftStore } from "@/stores/draft";
import { NEW_WORKFLOW, useWorkflowStore } from "@/stores/workflows";

/**
 * A description, in the person's own words, written into a workflow by a model: the agent they
 * already have, in its non-interactive mode, or their Anthropic key — the same writers that
 * draft commit messages. What comes back replaces the workflow's text, unsaved, with any
 * problems marked in the editor. Nothing a model wrote is a workflow until it is saved.
 *
 * With `current`, the description is a change to that text — a built-in just customized, or a
 * file of the person's — and the model is shown it whole and told to keep its id.
 */
export function DescribeWorkflow({
  workflowId = NEW_WORKFLOW,
  current,
}: {
  workflowId?: string;
  current?: string;
} = {}) {
  const changing = current !== undefined;
  const status = useDraftStore((s) => s.workflowWriter?.status);
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    void useDraftStore.getState().loadWorkflowWriter();
  }, []);

  const writer = status?.harness ?? (status?.key ? status.model : null);
  const can = !!status?.available && description.trim().length > 0 && !busy;

  async function write() {
    if (!can) return;
    setBusy(true);
    setError(null);
    setNote(null);
    // The answer takes a while, and the person may type, save, discard, or ask again meanwhile.
    // It is kept only if the workflow is as it was when the question went out.
    const serial = useWorkflowStore.getState().beginDescribe(workflowId);
    try {
      const written = await ipc.workflowDescribe(description, current ?? null);
      if (!useWorkflowStore.getState().finishDescribe(workflowId, serial, written.text)) {
        setNote("The workflow changed while this was being written, so it was set aside.");
        return;
      }
      setNote(
        written.problems.length === 0
          ? `${changing ? "Changed" : "Written"} by ${written.writer}. Look it over, then save it.`
          : `${changing ? "Changed" : "Written"} by ${written.writer}, with ${written.problems.length} ${
              written.problems.length === 1 ? "problem" : "problems"
            } left to fix, marked in the editor.`,
      );
    } catch (failed) {
      setError(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      aria-label={changing ? "Describe a change" : "Describe it"}
      className="flex shrink-0 flex-col gap-2 border-b border-line bg-canvas/40 px-4 py-3"
    >
      <label className="flex flex-col gap-1.5">
        <span className="text-small font-semibold tracking-wider text-ink-faint uppercase">
          {changing ? "Describe a change" : "Or describe it"}
        </span>
        <textarea
          aria-label={changing ? "What to change" : "What the workflow should do"}
          className="min-h-16 rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
          placeholder={
            changing
              ? "Run the smoke tests before the review, and only tell me if something failed."
              : "Have Codex review the pull request, then tell me and the agent that wrote it."
          }
          value={description}
          disabled={busy || !status?.available}
          onChange={(event) => setDescription(event.target.value)}
        />
      </label>
      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className="rounded border border-line px-3 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
          disabled={!can}
          onClick={() => void write()}
        >
          {busy ? `Asking ${writer ?? "the model"}…` : changing ? "Change it" : "Write it"}
        </button>
        {status && !status.available && (
          <p className="text-ink-faint">
            {status.enabled
              ? (status.problem ?? "Nothing here can write one.")
              : "Writing with a model is switched off in Settings → Assist."}
          </p>
        )}
        {status?.available && !busy && !note && !error && writer && (
          <p className="text-ink-faint">
            Written by {writer}; choose the Workflow writer in Settings → Assist.
          </p>
        )}
        {note && <p className="text-ink-muted">{note}</p>}
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

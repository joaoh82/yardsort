import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type Problem } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useDraftStore } from "@/stores/draft";

/**
 * A description, in the person's own words, written into a smoke test by a model — the same
 * Drafting writers the workflow editor uses (the agent they already have, in its non-interactive
 * mode, or their Anthropic key). What comes back is editable, with any problems marked; nothing
 * is a test until **Save** writes it into a workspace's `.yardsort/tests`, where it is then an
 * ordinary uncommitted change.
 */
export function DescribeTest({
  workspaces,
  onClose,
  onSaved,
}: {
  workspaces: { id: string; name: string }[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);
  const status = useDraftStore((s) => s.workflowWriter?.status);

  const [description, setDescription] = useState("");
  const [text, setText] = useState("");
  const [problems, setProblems] = useState<Problem[]>([]);
  const [workspace, setWorkspace] = useState(workspaces[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useDraftStore.getState().loadWorkflowWriter();
  }, []);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy && !saving) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy, saving]);

  const writer = status?.harness ?? (status?.key ? status.model : null);
  const canWrite = !!status?.available && description.trim().length > 0 && !busy;
  const canSave = text.trim().length > 0 && !!workspace && !saving;

  async function write() {
    if (!canWrite) return;
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const written = await ipc.testDescribe(description);
      setText(written.text);
      setProblems(written.problems);
      setNote(
        written.problems.length === 0
          ? `Written by ${written.writer}. Look it over, then save it.`
          : `Written by ${written.writer}, with ${written.problems.length} ${
              written.problems.length === 1 ? "problem" : "problems"
            } left to fix.`,
      );
    } catch (failed) {
      setError(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      await ipc.testSave(workspace, text);
      onSaved();
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setSaving(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[8vh]"
      onPointerDown={(event) =>
        event.target === event.currentTarget && !busy && !saving && onClose()
      }
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[84vh] w-[40rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            Describe a smoke test
          </h2>
          <p className="mt-1 text-ink-muted">
            Say what it should check; a model writes the file. Save it into a workspace to keep it.
          </p>
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5 pt-4">
          <label className="flex flex-col gap-1.5">
            <span className="text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
              What it should check
            </span>
            <textarea
              aria-label="What the smoke test should check"
              className="min-h-20 rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
              placeholder="Start the dev server, open the login page, sign in as the test user, and check the dashboard shows their name."
              value={description}
              disabled={busy || !status?.available}
              onChange={(event) => setDescription(event.target.value)}
            />
          </label>
          <div className="flex flex-wrap items-center gap-3">
            <button
              type="button"
              className="rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
              disabled={!canWrite}
              onClick={() => void write()}
            >
              {busy ? `Asking ${writer ?? "the model"}…` : text ? "Write again" : "Write it"}
            </button>
            {status && !status.available && (
              <p className="text-ink-faint">
                {status.enabled
                  ? (status.problem ?? "Nothing here can write one.")
                  : "Writing with a model is switched off in Settings → Assist."}
              </p>
            )}
            {note && <p className="text-ink-muted">{note}</p>}
          </div>

          {text && (
            <>
              <label className="flex flex-col gap-1.5">
                <span className="text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
                  The file
                </span>
                <textarea
                  aria-label="The smoke test file"
                  className="min-h-48 rounded border border-line bg-canvas px-2 py-1.5 font-mono text-[12px] outline-none select-text focus:border-accent"
                  value={text}
                  onChange={(event) => setText(event.target.value)}
                  spellCheck={false}
                />
              </label>
              {problems.length > 0 && (
                <ul aria-label="Problems" className="flex flex-col gap-1 text-red-400">
                  {problems.map((problem, i) => (
                    <li key={i} className="select-text">
                      {problem.message}
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}

          {error && (
            <p role="alert" className="text-red-400 select-text">
              {error}
            </p>
          )}
        </div>

        <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-line p-3">
          {text && (
            <label className="mr-auto flex items-center gap-2 text-ink-muted">
              <span className="text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
                Save in
              </span>
              <select
                aria-label="Save in workspace"
                value={workspace}
                onChange={(event) => setWorkspace(event.target.value)}
                className="h-8 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent"
              >
                {workspaces.length === 0 && <option value="">No workspace</option>}
                {workspaces.map((w) => (
                  <option key={w.id} value={w.id}>
                    {w.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          <button
            type="button"
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
            onClick={onClose}
            disabled={busy || saving}
          >
            Cancel
          </button>
          <button
            type="button"
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            disabled={!canSave}
            onClick={() => void save()}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </footer>
      </div>
    </div>
  );
}

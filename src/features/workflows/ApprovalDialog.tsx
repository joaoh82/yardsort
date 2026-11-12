import { useEffect, useId, useRef, useState } from "react";
import type { WorkflowStepRun } from "@/lib/ipc";

type WorkflowArtifact = WorkflowStepRun["consumed"][number];
import { Markdown } from "@/lib/Markdown";
import { useModalFocus } from "@/lib/useModalFocus";

export type Decision = "approved" | "rejected" | "changes_requested";

/** A current that reads as Markdown: a `.md` file, or text that opens a heading somewhere. */
function looksLikeMarkdown(artifact: WorkflowArtifact): boolean {
  return /\.(md|markdown)$/i.test(artifact.detail ?? "") || /^#{1,6}\s/m.test(artifact.body);
}

/**
 * A waiting approval at the size of the window: the documents it reads, one at a time and
 * formatted, beside the note and the three decisions. Closing keeps the note: the box that
 * opened this holds it.
 */
export function ApprovalDialog({
  stepId,
  workspaceName,
  instructions,
  documents,
  feedback,
  onFeedback,
  busy,
  error,
  onDecide,
  onClose,
}: {
  stepId: string;
  workspaceName: string;
  /** What the step says about the decision, when it says something. */
  instructions: string | null;
  documents: WorkflowArtifact[];
  feedback: string;
  onFeedback: (text: string) => void;
  busy: boolean;
  error: string | null;
  onDecide: (decision: Decision) => void;
  onClose: () => void;
}) {
  const [shown, setShown] = useState(0);
  const [plain, setPlain] = useState<Record<string, boolean>>({});
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  const current = documents[shown];
  const formatted = current ? !(plain[current.name] ?? !looksLikeMarkdown(current)) : false;
  const noted = feedback.trim() !== "";

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/50"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex h-[88vh] w-[90rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="flex shrink-0 items-start gap-3 border-b border-line px-5 pt-4 pb-3">
          <div className="min-w-0 flex-1">
            <h2 id={titleId} className="text-base font-semibold">
              Your approval: <span className="font-mono">{stepId}</span>
            </h2>
            <p className="mt-0.5 text-ink-muted">{workspaceName} · No code is needed here.</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="px-2 py-1 text-ink-muted hover:text-ink"
          >
            Close
          </button>
        </header>

        <div className="flex min-h-0 flex-1 flex-col md:flex-row">
          <section aria-label="To review" className="flex min-h-0 min-w-0 flex-1 flex-col">
            {current ? (
              <>
                <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-line px-5 py-2">
                  {documents.length > 1 ? (
                    <div role="tablist" aria-label="Documents" className="flex flex-wrap gap-1">
                      {documents.map((artifact, index) => (
                        <button
                          key={artifact.name}
                          type="button"
                          role="tab"
                          aria-selected={index === shown}
                          onClick={() => setShown(index)}
                          className={`rounded border px-2.5 py-0.5 ${
                            index === shown
                              ? "border-accent text-ink"
                              : "border-line text-ink-muted hover:text-ink"
                          }`}
                        >
                          {artifact.name}
                        </button>
                      ))}
                    </div>
                  ) : (
                    <span className="font-medium">{current.name}</span>
                  )}
                  <span className="truncate font-mono text-detail text-ink-faint">
                    {current.detail ?? current.source}
                    {current.truncated ? " (cut short)" : ""}
                  </span>
                  <button
                    type="button"
                    aria-pressed={!formatted}
                    onClick={() => setPlain({ ...plain, [current.name]: formatted })}
                    className="ml-auto rounded border border-line px-2 py-0.5 text-detail text-ink-muted hover:text-ink"
                  >
                    {formatted ? "Show plain text" : "Show formatted"}
                  </button>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
                  <div className="mx-auto max-w-[52rem]">
                    {formatted ? (
                      <Markdown text={current.body} />
                    ) : (
                      <pre className="font-mono text-detail leading-relaxed whitespace-pre-wrap select-text">
                        {current.body}
                      </pre>
                    )}
                  </div>
                </div>
              </>
            ) : (
              <p className="px-5 py-4 text-ink-muted">This approval reads no documents.</p>
            )}
          </section>

          <aside className="flex shrink-0 flex-col gap-3 overflow-y-auto border-t border-line px-5 py-4 md:w-96 md:border-t-0 md:border-l">
            {instructions && (
              <p className="whitespace-pre-wrap text-ink-muted select-text">{instructions}</p>
            )}
            <label className="flex min-h-0 flex-col gap-1">
              <span className="text-detail text-ink-faint">
                Note — required when you request changes
              </span>
              <textarea
                aria-label={`Feedback for ${stepId}`}
                value={feedback}
                rows={10}
                disabled={busy}
                onChange={(event) => onFeedback(event.target.value)}
                placeholder="What should be different?"
                className="w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
              />
            </label>
            {error && (
              <p role="alert" className="text-red-400 select-text">
                {error}
              </p>
            )}
            <div className="flex flex-col gap-1.5">
              <button
                type="button"
                disabled={busy}
                onClick={() => onDecide("approved")}
                className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
              >
                Approve
              </button>
              <button
                type="button"
                disabled={busy || !noted}
                title={noted ? undefined : "Write a note first"}
                onClick={() => onDecide("changes_requested")}
                className="rounded border border-line px-4 py-1.5 hover:bg-raised disabled:opacity-40"
              >
                Request changes
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => onDecide("rejected")}
                className="rounded border border-line px-4 py-1.5 text-red-300 hover:bg-raised disabled:opacity-40"
              >
                Reject
              </button>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}

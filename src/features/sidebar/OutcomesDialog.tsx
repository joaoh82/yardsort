import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type Attempt, type Project, type ProjectOutcomes } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { describeHistory } from "./outcomeWords";

const LABELS = [
  { value: "kept", text: "Kept" },
  { value: "partly", text: "Partly" },
  { value: "discarded", text: "Discarded" },
] as const;

/**
 * What became of each attempt in a project — each workspace, and its task — and each agent's
 * history across them. An outcome is what the user says, or a merge; nothing else counts. See
 * `docs/guide/outcomes.md`.
 */
export function OutcomesDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const [outcomes, setOutcomes] = useState<ProjectOutcomes | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    void ipc.outcomesGet(project.id).then(setOutcomes, (reason) => setError(errorMessage(reason)));
  }, [project.id]);

  const label = async (attempt: Attempt, value: string) => {
    setBusy(true);
    try {
      // Choosing the label already given takes it back.
      setOutcomes(await ipc.outcomeLabel(attempt.id, attempt.label === value ? null : value));
      setError(null);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[8vh]"
      onPointerDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[84vh] w-[48rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            Outcomes — {project.name}
          </h2>
          <p className="mt-1 text-ink-faint">
            Each workspace is an attempt at its task. Say how each went; a merged pull request, or a
            branch merged into its base, counts as kept until you say otherwise. Nothing else is
            counted — a deleted workspace or a closed pull request is shown, never scored.
          </p>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5 pt-4">
          {error && (
            <p role="alert" className="mb-3 text-red-400 select-text">
              {error}
            </p>
          )}
          {outcomes && (
            <>
              <section aria-label="Agents" className="mb-5">
                <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
                  Agents in this project
                </h3>
                {outcomes.agents.length === 0 ? (
                  <p className="text-ink-faint">No attempts yet.</p>
                ) : (
                  <ul className="space-y-1">
                    {outcomes.agents.map((agent) => (
                      <li key={agent.harness} className="flex items-center gap-2">
                        <HarnessIcon id={agent.harness} label={agent.harness} size={12} />
                        <span className="w-24 shrink-0">{agent.harness}</span>
                        <span className="text-ink-muted">{describeHistory(agent)}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              <section aria-label="Attempts">
                <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
                  Attempts · {outcomes.attempts.length}
                </h3>
                <ul className="space-y-2">
                  {outcomes.attempts.map((attempt) => (
                    <li
                      key={attempt.id}
                      aria-label={attempt.workspaceName}
                      className="rounded border border-line bg-canvas/40 p-2.5"
                    >
                      <div className="flex items-baseline gap-2">
                        <span className="font-medium">{attempt.workspaceName}</span>
                        <span className="min-w-0 flex-1 truncate text-[11px] text-ink-faint">
                          {attempt.harnesses.join(", ") || "no agent"}
                        </span>
                        <span className="text-[11px] text-ink-muted">{outcomeWords(attempt)}</span>
                      </div>
                      {attempt.task && (
                        <p className="mt-1 text-[12px] text-ink-muted select-text">
                          {attempt.task}
                        </p>
                      )}
                      <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                        {LABELS.map(({ value, text }) => (
                          <button
                            key={value}
                            type="button"
                            disabled={busy}
                            aria-pressed={attempt.label === value}
                            onClick={() => void label(attempt, value)}
                            className="rounded border border-line px-2 py-0.5 text-[11px] text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40 aria-pressed:border-accent aria-pressed:text-ink"
                          >
                            {text}
                          </button>
                        ))}
                        <span className="ml-auto text-[11px] text-ink-faint">
                          {evidenceWords(attempt)}
                        </span>
                      </div>
                    </li>
                  ))}
                </ul>
              </section>
            </>
          )}
        </div>

        <footer className="flex justify-end border-t border-line p-3">
          <button
            type="button"
            onClick={onClose}
            className="rounded bg-raised px-3 py-1.5 hover:text-ink"
          >
            Done
          </button>
        </footer>
      </div>
    </div>
  );
}

function outcomeWords(attempt: Attempt): string {
  if (!attempt.outcome) return "no outcome yet";
  const source = attempt.outcomeSource === "merge" ? " — merged, not labelled" : "";
  return `${attempt.outcome}${source}`;
}

function evidenceWords(attempt: Attempt): string {
  const { evidence } = attempt;
  const bits: string[] = [];
  if (evidence.prNumber !== null) bits.push(`PR #${evidence.prNumber} ${evidence.prState}`);
  if (evidence.mergedIntoBase) bits.push(`merged into ${attempt.baseBranch ?? "its base"}`);
  else if (evidence.ahead) bits.push(`ahead of ${attempt.baseBranch ?? "its base"}`);
  if (evidence.ended) bits.push(evidence.ended);
  return bits.join(" · ");
}

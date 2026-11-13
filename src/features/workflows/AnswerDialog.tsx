import { useEffect, useId, useRef, useState } from "react";
import { isMac } from "@/lib/platform";
import { useModalFocus } from "@/lib/useModalFocus";

/**
 * A round of an agent's questions, one at a time, then the whole round to look over before it
 * is sent as one answer. Closing keeps what was typed: the box that opened this holds it.
 */
export function AnswerDialog({
  stepId,
  workspaceName,
  questions,
  answers,
  onAnswers,
  busy,
  error,
  onSend,
  onClose,
}: {
  stepId: string;
  workspaceName: string;
  questions: string[];
  answers: string[];
  onAnswers: (answers: string[]) => void;
  busy: boolean;
  error: string | null;
  onSend: () => void;
  onClose: () => void;
}) {
  const total = questions.length;
  // `total` is the review page, after the last question.
  const [at, setAt] = useState(() => {
    const first = answers.findIndex((answer) => answer.trim() === "");
    return first === -1 ? (total > 1 ? total : 0) : first;
  });
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  useModalFocus(dialogRef);

  const reviewing = at >= total;
  const answered = answers.filter((answer) => answer.trim() !== "").length;
  const ready = answered > 0 && !busy;

  useEffect(() => {
    if (!reviewing) fieldRef.current?.focus();
  }, [at, reviewing]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  const setAnswer = (text: string) => {
    const next = questions.map((_, index) => answers[index] ?? "");
    next[at] = text;
    onAnswers(next);
  };
  // One question has nothing to review: its Next is the send.
  const next = () => {
    if (total === 1) {
      if (ready) onSend();
    } else {
      setAt(Math.min(at + 1, total));
    }
  };
  const nextWord =
    total === 1 ? (busy ? "Sending…" : "Send answer") : at === total - 1 ? "Review" : "Next";
  const sendKey = isMac ? "⌘Enter" : "Ctrl+Enter";

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[10vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[80vh] w-[52rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="shrink-0 border-b border-line px-5 pt-4 pb-3">
          <h2 id={titleId} className="text-base font-semibold">
            {total === 1 ? "The agent has a question" : `The agent has ${total} questions`}
          </h2>
          <p className="mt-0.5 text-ink-muted">
            {workspaceName} · <span className="font-mono text-detail">{stepId}</span> · No code is
            needed here.
          </p>
          {total > 1 && (
            <nav aria-label="Questions" className="mt-3 flex flex-wrap gap-1.5">
              {questions.map((_, index) => {
                const done = (answers[index] ?? "").trim() !== "";
                return (
                  <button
                    key={index}
                    type="button"
                    aria-current={index === at ? "step" : undefined}
                    aria-label={`Question ${index + 1}${done ? ", answered" : ""}`}
                    onClick={() => setAt(index)}
                    className={`h-7 min-w-7 rounded-full border px-2 text-detail ${
                      index === at
                        ? "border-accent text-ink"
                        : done
                          ? "border-line bg-raised text-ink-muted"
                          : "border-line text-ink-faint"
                    } hover:border-accent`}
                  >
                    {index + 1}
                  </button>
                );
              })}
              <button
                type="button"
                aria-current={reviewing ? "step" : undefined}
                onClick={() => setAt(total)}
                className={`h-7 rounded-full border px-3 text-detail ${
                  reviewing ? "border-accent text-ink" : "border-line text-ink-faint"
                } hover:border-accent`}
              >
                All answers
              </button>
            </nav>
          )}
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {reviewing ? (
            <ol aria-label="Your answers" className="flex flex-col gap-4">
              {questions.map((question, index) => {
                const answer = (answers[index] ?? "").trim();
                return (
                  <li key={index} className="flex flex-col gap-1">
                    <p className="whitespace-pre-wrap text-ink-muted select-text">
                      {index + 1}. {question}
                    </p>
                    <p
                      className={`whitespace-pre-wrap pl-4 select-text ${answer ? "text-ink" : "text-ink-faint italic"}`}
                    >
                      {answer || "No answer"}
                    </p>
                    <button
                      type="button"
                      onClick={() => setAt(index)}
                      className="self-start pl-4 text-detail text-ink-faint hover:text-ink"
                      aria-label={`Change the answer to question ${index + 1}`}
                    >
                      Change
                    </button>
                  </li>
                );
              })}
            </ol>
          ) : (
            <>
              {total > 1 && (
                <p className="text-detail text-ink-faint">
                  Question {at + 1} of {total}
                </p>
              )}
              <p className="mt-1 text-base leading-relaxed whitespace-pre-wrap select-text">
                {questions[at]}
              </p>
              <textarea
                ref={fieldRef}
                aria-label={total === 1 ? "Your answer" : `Answer to question ${at + 1}`}
                placeholder="Your answer"
                value={answers[at] ?? ""}
                rows={6}
                disabled={busy}
                onChange={(event) => setAnswer(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                    event.preventDefault();
                    next();
                  }
                }}
                className="mt-4 w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
              />
              <p className="mt-1 text-detail text-ink-faint">
                {sendKey} {total === 1 ? "sends" : "goes to the next question"}. A question can be
                left blank.
              </p>
            </>
          )}
          {error && (
            <p role="alert" className="mt-3 text-red-400 select-text">
              {error}
            </p>
          )}
        </div>

        <footer className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-line px-5 py-3">
          {total > 1 && (
            <span className="mr-auto text-detail text-ink-faint">
              {answered} of {total} answered
            </span>
          )}
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            Close
          </button>
          {total > 1 && (
            <button
              type="button"
              disabled={at === 0 || busy}
              onClick={() => setAt(at - 1)}
              className="rounded border border-line px-3 py-1.5 hover:bg-raised disabled:opacity-40"
            >
              Back
            </button>
          )}
          {reviewing ? (
            <button
              type="button"
              disabled={!ready}
              onClick={onSend}
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              {busy ? "Sending…" : "Send answers"}
            </button>
          ) : (
            <button
              type="button"
              disabled={total === 1 ? !ready : busy}
              onClick={next}
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              {nextWord}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

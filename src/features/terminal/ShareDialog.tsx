import { useId, useRef, useState } from "react";
import { useModalFocus } from "@/lib/useModalFocus";

export interface Question {
  about: string;
  projectId: string;
  projectName: string;
  answer: (go: boolean) => void;
}

/** The once-per-project question: may this project's agents read each other's conversations? */
export function ShareDialog({
  question,
  onAnswer,
  onClose,
}: {
  question: Question;
  onAnswer: (share: boolean) => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);
  const answer = async (share: boolean) => {
    setBusy(true);
    await onAnswer(share);
  };
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[18vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onKeyDown={(event) => event.key === "Escape" && !busy && onClose()}
        className="w-[32rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Let {question.projectName}&rsquo;s agents read each other&rsquo;s conversations?
        </h2>
        <p className="mt-3 text-ink-muted">
          There is {question.about} in this workspace. With this on, the agent you are starting —
          and every agent started beside another in this project — is told it can read what was said
          there with <code className="font-mono text-[12px]">ys workspace context</code>: your
          messages and each turn&rsquo;s final reply. Tool output is left out, and nothing is
          stored.
        </p>
        <p className="mt-2 text-ink-muted">
          What an agent reads goes to its provider: Claude&rsquo;s conversation, read by Codex, is
          sent to OpenAI as part of Codex&rsquo;s work.
        </p>
        <p className="mt-2 text-[12px] text-ink-faint">
          Asked once for this project. Change it later in the project&rsquo;s Memory view.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => void answer(false)}
            className="rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
          >
            Not in this project
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void answer(true)}
            className="rounded bg-accent px-3 py-1 font-medium text-canvas disabled:opacity-40"
          >
            Share conversations
          </button>
        </div>
      </div>
    </div>
  );
}

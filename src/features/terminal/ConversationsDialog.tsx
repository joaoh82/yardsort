import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type ConversationContext } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";

/**
 * What an agent is told about the workspace's other conversations, and what it reads when it
 * runs `ys workspace context`, both as they would be now. Read from the agents' own transcripts
 * when the dialog opens; nothing is kept.
 */
export function ConversationsDialog({
  workspaceId,
  except,
  onClose,
}: {
  workspaceId: string;
  /** The conversation of the agent asking, which it is not shown. */
  except: string | null;
  onClose: () => void;
}) {
  // `undefined` while reading; `null` when there is nothing to read.
  const [context, setContext] = useState<ConversationContext | null>();
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    let stale = false;
    ipc.conversationContext(workspaceId, except).then(
      (read) => !stale && setContext(read),
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [workspaceId, except]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[10vh]"
      onPointerDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[80vh] w-[44rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          This workspace&rsquo;s other conversations
        </h2>
        {error ? (
          <p role="alert" className="mt-3 text-red-400 select-text">
            {error}
          </p>
        ) : context === undefined ? (
          <p className="mt-3 text-ink-muted">Reading the conversations…</p>
        ) : context === null ? (
          <p className="mt-3 text-ink-muted">
            Nothing to read now: the project does not share its agents&rsquo; conversations, or none
            of the others has said anything yet.
          </p>
        ) : (
          <div className="mt-3 flex min-h-0 flex-col gap-3">
            <section aria-label="The note">
              <h3 className="mb-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
                The note
              </h3>
              <p className="text-ink-muted select-text">{context.note}</p>
            </section>
            <section aria-label="What the agent reads" className="flex min-h-0 flex-col">
              <h3 className="mb-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
                What <code className="font-mono normal-case">ys workspace context</code> gives now
              </h3>
              <pre className="min-h-0 overflow-auto rounded border border-line bg-canvas p-2 text-[11px] whitespace-pre-wrap select-text">
                {context.digest}
              </pre>
            </section>
          </div>
        )}
        <div className="mt-4 flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

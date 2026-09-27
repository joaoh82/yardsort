import { useEffect, useId, useRef, useState } from "react";
import {
  errorMessage,
  ipc,
  type MemoryCheck,
  type MemoryDecision,
  type MemoryEntry,
  type Project,
  type ProjectMemory,
} from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useAssistStore } from "@/stores/assist";
import { useMemoryStore } from "@/stores/memory";

const MAX_CHARS = 500;
const button =
  "shrink-0 rounded border border-line px-2 py-0.5 text-[11px] text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40";

/**
 * A project's memory: short lessons for its agents. The user writes entries, approved as they are
 * written; agents propose them with `ys memory propose`, and those wait here. Only approved
 * entries ever reach an agent, and only when the project shares them. See
 * `docs/guide/memory.md`.
 */
export function MemoryDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const [memory, setMemory] = useState<ProjectMemory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState("");
  // Jev's answers, with what they were judged against: shown only while that still holds.
  const [judged, setJudged] = useState<{ against: string; checks: Record<string, MemoryCheck> }>({
    against: "",
    checks: {},
  });
  const checkMemory = useAssistStore((s) => s.status?.checkMemory ?? false);
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    void ipc.memoryGet(project.id).then(setMemory, (reason) => setError(errorMessage(reason)));
    void useAssistStore.getState().load();
  }, [project.id]);

  // Jev's word on each proposal, when the switch is on: advisory, and silent when it fails. A
  // verdict depends on the approved entries as much as on the proposal, so a change to either
  // asks again, and an answer about an older list is not shown.
  const candidates = memory?.entries.filter((entry) => entry.state === "candidate") ?? [];
  const against = (memory?.entries ?? [])
    .filter((entry) => entry.state === "candidate" || entry.state === "approved")
    .map((entry) => `${entry.state}:${entry.id}:${entry.text}`)
    .join("|");
  const hasCandidates = candidates.length > 0;
  useEffect(() => {
    if (!checkMemory || !hasCandidates) return;
    let stale = false;
    ipc.memoryCheck(project.id).then(
      (list) =>
        !stale && setJudged({ against, checks: Object.fromEntries(list.map((c) => [c.id, c])) }),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [checkMemory, hasCandidates, against, project.id]);
  const checks = checkMemory && judged.against === against ? judged.checks : {};

  const run = async (action: () => Promise<ProjectMemory>) => {
    setBusy(true);
    try {
      setMemory(await action());
      setError(null);
      void useMemoryStore.getState().refreshWaiting();
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const write = async (event: React.FormEvent) => {
    event.preventDefault();
    if (await run(() => ipc.memoryWrite(project.id, draft))) setDraft("");
  };
  const decide = (id: string, decision: MemoryDecision) =>
    void run(() => ipc.memoryDecide(id, decision));

  const approved = memory?.entries.filter((entry) => entry.state === "approved") ?? [];
  const setAside =
    memory?.entries.filter((entry) => entry.state === "rejected" || entry.state === "revoked") ??
    [];

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
        className="flex max-h-[84vh] w-[44rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            Memory — {project.name}
          </h2>
          <p className="mt-1 text-ink-faint">
            Short lessons about this project for its agents. What you write here is approved as you
            write it; what an agent proposes waits for you. Only approved entries ever reach an
            agent.
          </p>
          <label className="mt-3 flex items-start gap-2">
            <input
              type="checkbox"
              checked={memory?.shared ?? false}
              disabled={busy || !memory}
              onChange={(event) =>
                void run(() => ipc.memoryShare(project.id, event.target.checked))
              }
              className="mt-0.5 accent-(--color-accent)"
            />
            <span>
              Give this project&rsquo;s agents its memory
              <span className="block text-ink-faint">
                Approved entries go after the first message of each agent you start here, and into
                handoffs, cited, as notes rather than instructions. Agents can also run{" "}
                <code className="font-mono text-[11px]">ys memory search</code>. What the workspace
                was asked stays your own words.
              </span>
            </span>
          </label>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5 pt-4">
          <form onSubmit={write} className="flex items-start gap-2">
            <textarea
              aria-label="New memory entry"
              value={draft}
              maxLength={MAX_CHARS}
              rows={2}
              disabled={busy}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="Something every agent in this project should know — “The tests need TZ=UTC.”"
              className="min-w-0 flex-1 resize-none rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
            />
            <button
              type="submit"
              disabled={busy || draft.trim() === ""}
              className="rounded bg-accent px-3 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              Add
            </button>
          </form>
          {error && (
            <p role="alert" className="mt-2 text-red-400 select-text">
              {error}
            </p>
          )}

          <Section title="Waiting for you" count={candidates.length}>
            {candidates.length === 0 ? (
              <p className="text-ink-faint">
                No proposals. An agent proposes with{" "}
                <code className="font-mono text-[11px]">ys memory propose</code>.
              </p>
            ) : (
              candidates.map((entry) => (
                <Entry key={entry.id} entry={entry} busy={busy} onEdit={run}>
                  <CheckTags check={checks[entry.id]} />
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => decide(entry.id, "approve")}
                    className={button}
                  >
                    Approve
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => decide(entry.id, "reject")}
                    className={button}
                  >
                    Reject
                  </button>
                </Entry>
              ))
            )}
          </Section>

          <Section title="Approved" count={approved.length}>
            {approved.length === 0 ? (
              <p className="text-ink-faint">Nothing yet. Write one above, or approve a proposal.</p>
            ) : (
              approved.map((entry) => (
                <Entry key={entry.id} entry={entry} busy={busy} onEdit={run}>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => decide(entry.id, "revoke")}
                    className={button}
                    title="Stop giving this to agents. It is kept, with its history."
                  >
                    Revoke
                  </button>
                </Entry>
              ))
            )}
          </Section>

          {setAside.length > 0 && (
            <details className="mt-5">
              <summary className="cursor-pointer text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
                Rejected and revoked · {setAside.length}
              </summary>
              <ul className="mt-2 space-y-2">
                {setAside.map((entry) => (
                  <Entry key={entry.id} entry={entry} busy={busy}>
                    <span className="text-[11px] text-ink-faint">{entry.state}</span>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() =>
                        decide(entry.id, entry.state === "revoked" ? "restore" : "approve")
                      }
                      className={button}
                    >
                      {entry.state === "revoked" ? "Restore" : "Approve"}
                    </button>
                  </Entry>
                ))}
              </ul>
            </details>
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

function Section({
  title,
  count,
  children,
}: {
  title: string;
  count: number;
  children: React.ReactNode;
}) {
  return (
    <section aria-label={title} className="mt-5">
      <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
        {title} · {count}
      </h3>
      <ul className="space-y-2">{children}</ul>
    </section>
  );
}

/** One entry: its text, where it came from, and what can be done to it. Editable in place. */
function Entry({
  entry,
  busy,
  onEdit,
  children,
}: {
  entry: MemoryEntry;
  busy: boolean;
  onEdit?: (action: () => Promise<ProjectMemory>) => Promise<boolean>;
  children: React.ReactNode;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const edited = entry.history.some((item) => item.action === "edited");
  return (
    <li className="rounded border border-line bg-canvas/40 p-2.5">
      {editing === null ? (
        <p className="select-text">{entry.text}</p>
      ) : (
        <textarea
          aria-label="Edit memory entry"
          value={editing}
          maxLength={MAX_CHARS}
          rows={2}
          onChange={(event) => setEditing(event.target.value)}
          className="w-full resize-none rounded border border-line bg-canvas px-2 py-1 outline-none select-text focus:border-accent"
        />
      )}
      <div className="mt-1.5 flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 text-[11px] text-ink-faint">
          from {entry.from} · {entry.shortId}
          {edited ? " · edited" : ""}
        </span>
        {onEdit &&
          (editing === null ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => setEditing(entry.text)}
              className={button}
            >
              Edit
            </button>
          ) : (
            <>
              <button
                type="button"
                disabled={busy || editing.trim() === ""}
                onClick={() =>
                  void onEdit(() => ipc.memoryEdit(entry.id, editing)).then(
                    (saved) => saved && setEditing(null),
                  )
                }
                className={button}
              >
                Save
              </button>
              <button type="button" onClick={() => setEditing(null)} className={button}>
                Cancel
              </button>
            </>
          ))}
        {editing === null && children}
      </div>
    </li>
  );
}

/** Jev's word on a proposal: advisory, in the same amber as Assist's badges. */
function CheckTags({ check }: { check: MemoryCheck | undefined }) {
  if (!check) return null;
  const tags = [
    ...(check.repeats
      ? [
          {
            label: "repeats an entry",
            title: "Assist: this looks like it says what an approved entry already says.",
          },
        ]
      : []),
    ...(check.contradicts
      ? [
          {
            label: "may contradict an entry",
            title: "Assist: this looks like it says the opposite of an approved entry.",
          },
        ]
      : []),
  ];
  return (
    <>
      {tags.map((tag) => (
        <span
          key={tag.label}
          title={tag.title}
          className="rounded border border-amber-400/40 px-1 text-[10px] leading-4 text-amber-300"
        >
          {tag.label}
        </span>
      ))}
    </>
  );
}

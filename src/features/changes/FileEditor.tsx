import { lazy, Suspense, useMemo, useState } from "react";
import { errorMessage, ipc } from "@/lib/ipc";
import { native } from "@/lib/native";
import { useChangesStore } from "@/stores/changes";
import { useProjectsStore } from "@/stores/projects";

const CodeView = lazy(() => import("./CodeView").then((m) => ({ default: m.CodeView })));
interface Draft {
  expected: string;
  text: string;
}
// Serialize writes so a slower keystroke cannot replace a newer draft in the core's UI store.
let pending = Promise.resolve();
function remember(key: string, draft: Draft | null) {
  const value = JSON.stringify(draft);
  useProjectsStore.setState((s) => ({ ui: { ...s.ui, [key]: value } }));
  pending = pending.catch(() => {}).then(() => ipc.uiStateSave(key, value));
  return pending;
}

export function FileEditor({
  workspaceId,
  path,
  text,
}: {
  workspaceId: string;
  path: string;
  text: string | null;
}) {
  const key = `fileDraft:${JSON.stringify([workspaceId, path])}`;
  const raw = useProjectsStore((s) => s.ui[key]);
  const draft = useMemo(() => (raw ? (JSON.parse(raw) as Draft | null) : null), [raw]);
  const [preview, setPreview] = useState(/\.svg$/i.test(path));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const current = draft?.text ?? text ?? "";
  const dirty = draft !== null && current !== draft.expected;
  const changed = draft !== null && draft.expected !== text;
  const keep = (next: Draft | null) =>
    remember(key, next).catch((reason) => setError(errorMessage(reason)));
  const save = async () => {
    if (!draft || busy) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.workspaceSaveFile(workspaceId, path, draft.expected, current);
      // Preserve anything typed while saving and advance its comparison baseline.
      const latestRaw = useProjectsStore.getState().ui[key];
      const latest = latestRaw ? (JSON.parse(latestRaw) as Draft | null) : null;
      const changes = useChangesStore.getState();
      if (
        changes.workspaceId === workspaceId &&
        changes.viewing?.kind === "file" &&
        changes.viewing.path === path
      ) {
        useChangesStore.setState({ file: { type: "text", text: current } });
      }
      await remember(
        key,
        latest && latest.text !== current ? { expected: current, text: latest.text } : null,
      );
      await useChangesStore.getState().refresh();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };
  const discard = async () => {
    if (
      dirty &&
      !(await native.confirm(`Discard your unsaved edits to “${path}”? The file on disk is kept.`, {
        title: "Discard edits",
        okLabel: "Discard",
      }))
    )
      return;
    await keep(null);
    await useChangesStore.getState().refresh();
  };
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b border-line px-2 py-1 text-[11px]">
        {/\.svg$/i.test(path) && (
          <button type="button" onClick={() => setPreview(!preview)}>
            {preview ? "Edit source" : "Preview"}
          </button>
        )}
        <span className="mr-auto text-ink-faint">{dirty ? "Unsaved edits" : "Text editor"}</span>
        <button
          type="button"
          disabled={!dirty || busy}
          onClick={() => void save()}
          className="rounded px-2 py-1 text-accent disabled:opacity-40"
        >
          {busy ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          disabled={!draft || busy}
          onClick={() => void discard()}
          className="rounded px-2 py-1 text-ink-muted disabled:opacity-40"
        >
          Discard
        </button>
      </div>
      {changed && (
        <p role="status" className="px-2 py-1 text-ink-muted">
          The file changed on disk. Your draft is kept; copy your edits before discarding to reload.
        </p>
      )}
      {error && (
        <p role="alert" className="px-2 py-1 text-red-400">
          {error}
        </p>
      )}
      <div className="min-h-0 flex-1">
        {preview ? (
          <div className="flex h-full items-center justify-center p-3">
            <img
              alt={path}
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(current)}`}
              className="max-h-full max-w-full object-contain"
            />
          </div>
        ) : (
          <Suspense fallback={<p>Loading…</p>}>
            <CodeView
              path={path}
              text={current}
              onChange={(next) => {
                const expected = draft?.expected ?? text ?? "";
                void keep(next === expected ? null : { expected, text: next });
              }}
            />
          </Suspense>
        )}
      </div>
    </div>
  );
}

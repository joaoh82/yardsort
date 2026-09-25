import { useEffect, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { hasCore, ipc } from "@/lib/ipc";
import { useAssistStore } from "@/stores/assist";
import { useChangesStore } from "@/stores/changes";
import { recall, useProjectsStore } from "@/stores/projects";
import { useDraftStore } from "@/stores/draft";
import { usePublishStore } from "@/stores/publish";
import { useProvenanceStore } from "@/stores/provenance";
import { ChangeList } from "./ChangeList";
import { FileTree } from "./FileTree";
import { PublishBar } from "./PublishBar";
import { Viewer } from "./Viewer";
import { keyOf, titleOf } from "./viewing";

type Tab = "changes" | "files";

/** Remembered across restarts, for every workspace. */
const SHOW_IGNORED = "files.showIgnored";

/** Right panel: what changed in the selected workspace, its files, and a viewer for either. */
export function ChangesPanel() {
  const selectedWorkspaceId = useProjectsStore((s) => s.selectedWorkspaceId);
  const composing = useProjectsStore((s) => s.composingProjectId !== null);
  const workspaceId = composing ? null : selectedWorkspaceId;

  const changes = useChangesStore((s) => s.changes);
  const error = useChangesStore((s) => s.error);
  const viewing = useChangesStore((s) => s.viewing);
  const [tab, setTab] = useState<Tab>("changes");
  const showIgnored = useProjectsStore((s) => recall(s.ui, SHOW_IGNORED, false));
  // Expansion belongs to one file: opening another, or closing the viewer, returns to the panel.
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const expanded = viewing !== null && expandedKey === keyOf(viewing);
  /** Bumped on every file-system signal; open folders in the tree reload when it changes. */
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    void useChangesStore.getState().follow(workspaceId);
    // Assist, when it is on, checks the same workspace shortly after the writing stops.
    void useAssistStore.getState().load();
    useAssistStore.getState().follow(workspaceId);
    void usePublishStore.getState().follow(workspaceId);
    // What the agents reported writing, joined to the list above.
    void useProvenanceStore.getState().follow(workspaceId);
    void useDraftStore.getState().load();
  }, [workspaceId]);

  useEffect(() => {
    if (!hasCore()) return;
    const refresh = () => {
      setRevision((value) => value + 1);
      void useChangesStore.getState().refresh();
      useAssistStore.getState().reviewSoon();
      // A commit or a checkout moves what there is to push; the forge has not changed, so its
      // last answer is reused rather than asked for again.
      void usePublishStore.getState().refresh();
      void useProvenanceStore.getState().refresh();
    };
    const unlisten = ipc.onWorkspaceFilesChanged((changedId) => {
      if (changedId === useChangesStore.getState().workspaceId) refresh();
    });
    // A hook landing is a report that may name a file already on the list.
    const unlistenActivity = ipc.onActivityChanged((ids) => {
      const followed = useProvenanceStore.getState().workspaceId;
      if (followed && ids.includes(followed)) void useProvenanceStore.getState().refresh();
    });
    // The watcher is best-effort; coming back to the window always catches up.
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      void unlisten.then((stop) => stop());
      void unlistenActivity.then((stop) => stop());
    };
  }, []);

  const count = changes ? changes.uncommitted.length + changes.committed.length : 0;
  const tabClass =
    "rounded px-2 py-0.5 text-[11px] font-semibold tracking-wider text-ink-faint uppercase hover:text-ink aria-selected:bg-raised aria-selected:text-ink";

  const list = (
    <div className="flex h-full min-h-0 flex-col">
      {!workspaceId ? (
        <p className="p-3 text-ink-faint">Select a workspace to see its changes.</p>
      ) : error ? (
        <p role="alert" className="p-3 text-red-400 select-text">
          {error}
        </p>
      ) : tab === "files" ? (
        <FileTree workspaceId={workspaceId} revision={revision} showIgnored={showIgnored} />
      ) : changes ? (
        <ChangeList changes={changes} />
      ) : (
        <p className="p-3 text-ink-faint">Loading…</p>
      )}
      {tab === "changes" && workspaceId && !error && (
        <PublishBar changes={changes} workspaceId={workspaceId} />
      )}
    </div>
  );

  return (
    <aside aria-label="Changes" className="flex h-full flex-col bg-surface">
      <header className="flex h-9 shrink-0 items-center gap-1 border-b border-line px-2">
        <div role="tablist" className="flex gap-1">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "changes"}
            onClick={() => setTab("changes")}
            className={tabClass}
          >
            Changes{count > 0 ? ` ${count}` : ""}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "files"}
            onClick={() => setTab("files")}
            className={tabClass}
          >
            Files
          </button>
        </div>
        {tab === "files" && (
          <button
            type="button"
            aria-pressed={showIgnored}
            title="Also show .git and files ignored by git (node_modules, build output…)"
            onClick={() => useProjectsStore.getState().remember(SHOW_IGNORED, !showIgnored)}
            className="ml-auto rounded px-2 py-0.5 text-[11px] text-ink-faint hover:bg-raised hover:text-ink aria-pressed:bg-raised aria-pressed:text-accent"
          >
            ignored
          </button>
        )}
      </header>

      {viewing && !expanded ? (
        <Group orientation="vertical" className="min-h-0 flex-1">
          <Panel id="list" defaultSize="40%" minSize={80}>
            {list}
          </Panel>
          <Separator className="h-px bg-line outline-none hover:bg-accent data-[separator=active]:bg-accent" />
          <Panel id="viewer" minSize={120}>
            <Viewer expanded={false} onToggleExpanded={() => setExpandedKey(keyOf(viewing))} />
          </Panel>
        </Group>
      ) : (
        <div className="min-h-0 flex-1">{list}</div>
      )}

      {viewing && expanded && (
        <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-6">
          <div
            role="dialog"
            aria-modal="true"
            aria-label={titleOf(viewing)}
            className="h-full w-full max-w-6xl overflow-hidden rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
          >
            <Viewer expanded onToggleExpanded={() => setExpandedKey(null)} />
          </div>
        </div>
      )}
    </aside>
  );
}

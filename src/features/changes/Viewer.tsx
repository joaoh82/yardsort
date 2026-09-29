import { lazy, Suspense, useState } from "react";
import { errorMessage, ipc, type Content } from "@/lib/ipc";
import { useChangesStore, type Viewing } from "@/stores/changes";
import { recall, useProjectsStore } from "@/stores/projects";
import { ReportedLine } from "./ReportedBadges";
import { FileEditor } from "./FileEditor";
import { titleOf } from "./viewing";

/** Inline or two panes, remembered across restarts like the composer's last picks. */
const DIFF_MODE_KEY = "changes.diffMode";
type DiffMode = "inline" | "split";

// CodeMirror is the heaviest thing in the app; load it when a file is first opened.
const CodeView = lazy(() => import("./CodeView").then((module) => ({ default: module.CodeView })));

const asText = (content: Content) => (content.type === "text" ? content.text : "");

/** Why a side cannot be shown as text, if it cannot. */
function obstacle(content: Content): string | null {
  if (content.type === "binary") return "Binary file — not shown.";
  if (content.type === "tooLarge") {
    return `File too large to show (${(content.bytes / 1_048_576).toFixed(1)} MB).`;
  }
  return null;
}

/** The open file or diff, with its header. `expanded` renders the same thing in the big overlay. */
export function Viewer({
  expanded,
  onToggleExpanded,
}: {
  expanded: boolean;
  onToggleExpanded: () => void;
}) {
  const workspaceId = useChangesStore((s) => s.workspaceId);
  const viewing = useChangesStore((s) => s.viewing);
  const diff = useChangesStore((s) => s.diff);
  const file = useChangesStore((s) => s.file);
  const viewError = useChangesStore((s) => s.viewError);
  const close = useChangesStore((s) => s.view);
  const ui = useProjectsStore((s) => s.ui);
  const mode = recall<DiffMode>(ui, DIFF_MODE_KEY, "inline");
  if (!viewing || !workspaceId) return null;

  const path = titleOf(viewing);
  const deleted = viewing.kind === "diff" && viewing.change.kind === "deleted";
  const openInEditor = () =>
    ipc
      .openInEditor(workspaceId, deleted ? null : path)
      .catch((error) => useProjectsStore.setState({ error: errorMessage(error) }));

  const headerButton =
    "shrink-0 rounded px-1.5 py-0.5 whitespace-nowrap text-ink-faint hover:bg-raised hover:text-ink";
  return (
    <section aria-label={`Viewing ${path}`} className="flex h-full min-h-0 flex-col">
      <header className="flex h-8 shrink-0 items-center gap-1 border-b border-line bg-surface px-2">
        {/* In a narrow panel the file name is what identifies it; the folder is in the tooltip. */}
        <span className="min-w-0 flex-1 truncate font-mono text-[12px]" title={path}>
          {expanded && viewing.kind === "diff" && viewing.change.oldPath && (
            <span className="text-ink-faint">{viewing.change.oldPath} → </span>
          )}
          {expanded ? path : path.slice(path.lastIndexOf("/") + 1)}
        </span>
        {/* The side panel is narrow and the list above already says which group this is. */}
        {expanded && viewing.kind === "diff" && (
          <span className="shrink-0 text-[11px] text-ink-faint">
            {viewing.scope === "uncommitted" ? "uncommitted" : "on this branch"}
          </span>
        )}
        {expanded && viewing.kind === "diff" && <ReportedLine path={viewing.change.path} />}
        {viewing.kind === "diff" && (
          <button
            type="button"
            onClick={() =>
              useProjectsStore
                .getState()
                .remember(DIFF_MODE_KEY, mode === "split" ? "inline" : "split")
            }
            className={headerButton}
            title={
              mode === "split"
                ? "Show removals inline, in one pane"
                : "Show the two versions side by side"
            }
          >
            {mode === "split" ? "Inline" : "Side by side"}
          </button>
        )}
        {viewing.kind === "diff" && !deleted && (
          <button
            type="button"
            className={headerButton}
            onClick={() => void close({ kind: "file", path })}
          >
            Open file
          </button>
        )}
        <button
          type="button"
          onClick={openInEditor}
          className={headerButton}
          aria-label="Open in editor"
          title="Open in your editor"
        >
          {expanded ? "Open in editor" : "Edit ↗"}
        </button>
        <button type="button" onClick={onToggleExpanded} className={headerButton}>
          {expanded ? "Shrink" : "Expand"}
        </button>
        <button
          type="button"
          aria-label="Close viewer"
          onClick={() => void close(null)}
          className={headerButton}
        >
          ×
        </button>
      </header>
      <div className="min-h-0 flex-1 bg-canvas">
        <Body
          workspaceId={workspaceId}
          viewing={viewing}
          diff={diff}
          file={file}
          error={viewError}
          mode={mode}
        />
      </div>
    </section>
  );
}

function Body(props: {
  workspaceId: string;
  viewing: Viewing;
  diff: ReturnType<typeof useChangesStore.getState>["diff"];
  file: Content | null;
  error: string | null;
  mode: DiffMode;
}) {
  const { workspaceId, viewing, diff, file, error, mode } = props;
  const path = titleOf(viewing);
  const draft = useProjectsStore((s) => s.ui[`fileDraft:${JSON.stringify([workspaceId, path])}`]);
  if (viewing.kind === "file" && draft && draft !== "null") {
    return (
      <FileEditor
        key={`${workspaceId}:${path}`}
        workspaceId={workspaceId}
        path={path}
        text={file?.type === "text" ? file.text : null}
      />
    );
  }
  const note = (text: string, alert = false) => (
    <p
      role={alert ? "alert" : undefined}
      className={`p-3 ${alert ? "text-red-400" : "text-ink-faint"}`}
    >
      {text}
    </p>
  );
  if (error) return note(error, true);
  let view: { text: string; original?: string };
  if (viewing.kind === "file") {
    if (!file) return note("Loading…");
    if (file.type === "absent") return note("This file no longer exists.");
    const blocked = obstacle(file);
    if (blocked) return note(blocked);
    if (file.type === "image") return <ImagePreview content={file} path={path} />;
    if (file.type === "text")
      return (
        <FileEditor
          key={`${workspaceId}:${path}`}
          workspaceId={workspaceId}
          path={path}
          text={file.text}
        />
      );
    view = { text: asText(file) };
  } else {
    if (!diff) return note("Loading…");
    if (diff.new.type === "image" || diff.old.type === "image") {
      return (
        <div className="flex h-full gap-2 overflow-auto p-2">
          {(
            [
              ["Before", diff.old],
              ["After", diff.new],
            ] as const
          ).map(([label, content]) => (
            <figure key={label} className="min-w-0 flex-1">
              <figcaption className="text-ink-faint">{label}</figcaption>
              {content.type === "image" ? (
                <ImagePreview content={content} path={`${path} — ${label}`} />
              ) : (
                note(content.type === "absent" ? "No file" : (obstacle(content) ?? "Text file"))
              )}
            </figure>
          ))}
        </div>
      );
    }
    const blocked = obstacle(diff.new) ?? obstacle(diff.old);
    if (blocked) return note(blocked);
    // A deleted file is shown as its old text, all removed; an added one as all new.
    view = { text: asText(diff.new), original: asText(diff.old) };
  }
  return (
    <Suspense fallback={note("Loading…")}>
      <CodeView path={path} text={view.text} original={view.original} split={mode === "split"} />
    </Suspense>
  );
}

function ImagePreview({
  content,
  path,
}: {
  content: Extract<Content, { type: "image" }>;
  path: string;
}) {
  const [failed, setFailed] = useState<string | null>(null);
  return (
    <div className="flex h-full items-center justify-center overflow-auto p-3">
      {failed === content.data ? (
        <p role="alert">This image could not be displayed.</p>
      ) : (
        <img
          src={`data:${content.mime};base64,${content.data}`}
          alt={path}
          onError={() => setFailed(content.data)}
          className="max-h-full max-w-full object-contain"
        />
      )}
    </div>
  );
}

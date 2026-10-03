import { lazy, Suspense, useState, type ReactNode } from "react";
import type { Content, FileDiff } from "@/lib/ipc";
import type { LineNote, LineSelection } from "./CodeView";
import { asText, obstacle, type DiffMode } from "./viewing";

// CodeMirror is the heaviest thing in the app; load it when a file is first opened.
const CodeView = lazy(() => import("./CodeView").then((module) => ({ default: module.CodeView })));

/** A line in place of a file's contents: why there is nothing to show, or that it is coming. */
export function Note({ children, alert = false }: { children: ReactNode; alert?: boolean }) {
  return (
    <p
      role={alert ? "alert" : undefined}
      className={`p-3 ${alert ? "text-red-400" : "text-ink-faint"}`}
    >
      {children}
    </p>
  );
}

/**
 * Both sides of one file as a diff: inline or in two panes, with images side by side and a line
 * of explanation for what cannot be shown as text.
 *
 * It knows nothing about where the two sides came from — a workspace's working tree against
 * its last commit, or two commits of a pull request that was never checked out.
 */
export function DiffBody({
  path,
  diff,
  mode,
  notes,
  renderNote,
  onSelect,
}: {
  path: string;
  diff: FileDiff | null;
  mode: DiffMode;
  /** Blocks under lines, and what to draw in them: comment threads, for a pull request. */
  notes?: LineNote[];
  renderNote?: (key: string) => ReactNode;
  onSelect?: (selection: LineSelection | null) => void;
}) {
  if (!diff) return <Note>Loading…</Note>;
  if (diff.new.type === "image" || diff.old.type === "image") {
    return (
      <div className="flex h-full gap-2 overflow-auto p-2">
        {(
          [
            ["Before", diff.old],
            ["After", diff.new],
          ] as const
        ).map(([label, content]) => (
          <figure key={label} className="flex min-h-0 min-w-0 flex-1 flex-col">
            <figcaption className="text-ink-faint">{label}</figcaption>
            <div className="min-h-0 flex-1">
              {content.type === "image" ? (
                <ImagePreview content={content} path={`${path} — ${label}`} />
              ) : content.type === "text" ? (
                <Suspense fallback={<Note>Loading…</Note>}>
                  <CodeView path={path} text={content.text} />
                </Suspense>
              ) : (
                <Note>
                  {content.type === "absent"
                    ? "No file"
                    : (obstacle(content) ?? "Cannot display this file.")}
                </Note>
              )}
            </div>
          </figure>
        ))}
      </div>
    );
  }
  const blocked = obstacle(diff.new) ?? obstacle(diff.old);
  if (blocked) return <Note>{blocked}</Note>;
  // A deleted file is shown as its old text, all removed; an added one as all new.
  return (
    <Suspense fallback={<Note>Loading…</Note>}>
      <CodeView
        path={path}
        text={asText(diff.new)}
        original={asText(diff.old)}
        split={mode === "split"}
        notes={notes}
        renderNote={renderNote}
        onSelect={onSelect}
      />
    </Suspense>
  );
}

export function ImagePreview({
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

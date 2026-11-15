import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import {
  errorMessage,
  ipc,
  type Content,
  type FileChange,
  type FileDiff,
  type FileEntry,
  type OtherChanges,
  type Scope,
} from "@/lib/ipc";
import { treeNavigation } from "@/lib/treeNavigation";
import { DiffBody, Note } from "@/features/changes/DiffBody";
import {
  CHANGE_KIND,
  DIFF_MODE_KEY,
  keyOf,
  obstacle,
  titleOf,
  type DiffMode,
} from "@/features/changes/viewing";
import { linkText } from "@/features/sidebar/otherComputersText";
import type { Viewing } from "@/stores/changes";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { RemotePublishBar } from "./RemotePublishBar";
import { recall, useProjectsStore } from "@/stores/projects";

const CodeView = lazy(() =>
  import("@/features/changes/CodeView").then((module) => ({ default: module.CodeView })),
);

type Tab = "changes" | "files";

/**
 * The right panel for a workspace on another computer: what changed in it and its files, read
 * from that computer when asked and shown as a local workspace's are. Nothing here writes: there
 * is no commit box and no editor, and nothing read is kept on this computer.
 *
 * It is offered where that computer lets this one type and its Yardsort is new enough to be
 * asked. Otherwise the panel keeps its place and says why it is empty.
 */
export function RemoteChangesPanel({
  hostId,
  workspaceId,
}: {
  hostId: string;
  workspaceId: string;
}) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const hostName = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const name = view?.name || hostName || "that computer";
  const online = view?.link.kind === "online";
  const mayLook = online && !!view?.mayLook && view.readAt !== null;
  // That computer was read again: what this panel shows of it is read again too.
  const readAt = view?.readAt ?? null;

  const [tab, setTab] = useState<Tab>("changes");
  const [listed, setListed] = useState<OtherChanges | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewing, setViewing] = useState<Viewing | null>(null);
  const [shown, setShown] = useState<{ key: string; diff?: FileDiff; file?: Content } | null>(null);
  const [viewError, setViewError] = useState<string | null>(null);
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  // Bumped when something was committed from here: the list reads again without waiting.
  const [committed, setCommitted] = useState(0);
  const mayPublish = !!view?.mayPublish;
  const mode = useProjectsStore((s) => recall<DiffMode>(s.ui, DIFF_MODE_KEY, "inline"));

  useEffect(() => {
    if (!mayLook) return;
    let stale = false;
    ipc.otherHostChanges(hostId, workspaceId).then(
      (read) => {
        if (stale) return;
        setListed(read);
        setError(null);
      },
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [mayLook, hostId, workspaceId, readAt, committed]);

  // What is open is read when it is opened, and again when that computer was read again.
  const viewingKey = viewing ? keyOf(viewing) : null;
  useEffect(() => {
    if (!mayLook || !viewing) return;
    let stale = false;
    const key = keyOf(viewing);
    const loaded =
      viewing.kind === "diff"
        ? ipc
            .otherHostDiff(
              hostId,
              workspaceId,
              viewing.change.path,
              viewing.change.oldPath,
              viewing.scope,
            )
            .then((diff) => ({ key, diff }))
        : ipc.otherHostFile(hostId, workspaceId, viewing.path).then((file) => ({ key, file }));
    loaded.then(
      (content) => {
        if (stale) return;
        setShown(content);
        setViewError(null);
      },
      (reason) => !stale && setViewError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
    // `viewing` is replaced on every press; its key says whether it is another file.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mayLook, hostId, workspaceId, viewingKey, readAt, committed]);

  // The tree's own state is kept here, above the list: the list is laid out again when a file
  // opens beside it, and a tree that forgot its open folders then would fold, and ask that
  // computer for them again.
  const [openDirs, setOpenDirs] = useState<ReadonlySet<string>>(() => new Set());
  // One map for as long as the panel is this workspace's: what each folder listed, kept where
  // drawing the list again does not lose it.
  const [folders] = useState(() => new Map<string, Listed>());
  const toggleDir = useCallback((dir: string) => {
    setOpenDirs((before) => {
      const next = new Set(before);
      if (!next.delete(dir)) next.add(dir);
      return next;
    });
  }, []);

  const open = useCallback((next: Viewing | null) => {
    setViewing(next);
    setViewError(null);
  }, []);

  if (!mayLook) {
    return (
      <aside aria-label="Changes" className="h-full bg-surface p-4">
        <p className="text-ink-muted">Changes and files stay on {name}.</p>
        <p className="mt-2 text-detail text-ink-faint">
          {!view || view.readAt === null
            ? `This computer has not read ${name} yet.`
            : !online
              ? `${name} is ${linkText(view.link)}. They are read from it when it is back.`
              : !view.mayType
                ? `${name} lets this computer watch its terminals, not read its files.`
                : `${name}'s Yardsort is older than this one, so its changes and files cannot be read from here. Update it there.`}
        </p>
      </aside>
    );
  }

  const changes = listed?.changes ?? null;
  const count = changes ? changes.uncommitted.length + changes.committed.length : 0;
  const current = viewing && shown?.key === keyOf(viewing) ? shown : null;
  const expanded = viewing !== null && expandedKey === keyOf(viewing);
  const tabClass =
    "rounded px-2 py-0.5 text-small font-semibold tracking-wider text-ink-faint uppercase hover:text-ink aria-selected:bg-raised aria-selected:text-ink";

  const list = (
    <div className="flex h-full min-h-0 flex-col">
      {error ? (
        <p role="alert" className="p-3 text-red-400 select-text">
          {error}
        </p>
      ) : tab === "files" ? (
        <ul
          onKeyDown={treeNavigation}
          role="tree"
          aria-label="Files"
          className="min-h-0 flex-1 overflow-y-auto py-1"
        >
          <Folder
            hostId={hostId}
            workspaceId={workspaceId}
            revision={readAt}
            dir=""
            depth={0}
            viewing={viewing}
            onOpen={open}
            openDirs={openDirs}
            onToggle={toggleDir}
            known={folders}
          />
        </ul>
      ) : changes ? (
        <div className="min-h-0 flex-1 overflow-y-auto pb-2">
          {count === 0 && <p className="p-3 text-ink-faint">No changes.</p>}
          <Changed
            title="Uncommitted"
            scope="uncommitted"
            files={changes.uncommitted}
            viewing={viewing}
            onOpen={open}
          />
          {changes.base && (
            <Changed
              title={`On this branch · vs ${changes.base}`}
              scope="committed"
              files={changes.committed}
              viewing={viewing}
              onOpen={open}
            />
          )}
          {listed?.more && (
            <p className="px-3 py-2 text-small text-ink-faint">
              More files changed than {name} lists here.
            </p>
          )}
        </div>
      ) : (
        <p className="p-3 text-ink-faint">Reading {name}…</p>
      )}
    </div>
  );

  const viewer = viewing && (
    <Viewer
      viewing={viewing}
      diff={current?.diff ?? null}
      file={current?.file ?? null}
      error={viewError}
      mode={mode}
      expanded={expanded}
      onToggleExpanded={() => setExpandedKey(expanded ? null : keyOf(viewing))}
      onOpen={open}
    />
  );

  return (
    <aside aria-label="Changes" className="flex h-full flex-col bg-surface">
      <header className="flex h-10 shrink-0 items-center gap-1 border-b border-line px-2">
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
      </header>

      {viewing && !expanded ? (
        <Group orientation="vertical" className="min-h-0 flex-1">
          <Panel id="list" defaultSize="40%" minSize={80}>
            {list}
          </Panel>
          <Separator className="h-px bg-line outline-none hover:bg-accent data-[separator=active]:bg-accent" />
          <Panel id="viewer" minSize={120}>
            {viewer}
          </Panel>
        </Group>
      ) : (
        <div className="min-h-0 flex-1">{list}</div>
      )}

      {/* Below the list and the viewer, and not inside either: what is typed here stays when a
          file opens or closes above it. */}
      {tab === "changes" && mayPublish && changes && !error && (
        <RemotePublishBar
          hostId={hostId}
          workspaceId={workspaceId}
          name={name}
          uncommitted={changes.uncommitted.length}
          revision={readAt}
          onChanged={() => setCommitted((count) => count + 1)}
        />
      )}
      <p className="shrink-0 border-t border-line px-3 py-1.5 text-small text-ink-faint">
        Read from {name}, and not kept here.{" "}
        {mayPublish
          ? `Commits, pushes and pull requests are made by ${name}, as itself.`
          : "Nothing is changed from this panel."}
      </p>

      {viewing && expanded && (
        <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-6">
          <div
            role="dialog"
            aria-modal="true"
            aria-label={titleOf(viewing)}
            className="h-full w-full max-w-6xl overflow-hidden rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
          >
            {viewer}
          </div>
        </div>
      )}
    </aside>
  );
}

/** One group of changed files, as the local list draws them. */
function Changed({
  title,
  scope,
  files,
  viewing,
  onOpen,
}: {
  title: string;
  scope: Scope;
  files: FileChange[];
  viewing: Viewing | null;
  onOpen: (viewing: Viewing) => void;
}) {
  if (files.length === 0) return null;
  return (
    <section aria-label={title}>
      <h3 className="sticky top-0 flex items-center justify-between bg-surface px-3 py-1.5 text-small font-semibold tracking-wider text-ink-faint uppercase">
        {title}
        <span className="font-normal">{files.length}</span>
      </h3>
      <ul>
        {files.map((change) => {
          const kind = CHANGE_KIND[change.kind];
          const selected =
            viewing?.kind === "diff" &&
            viewing.scope === scope &&
            viewing.change.path === change.path;
          const slash = change.path.lastIndexOf("/");
          return (
            <li key={change.path}>
              <button
                type="button"
                aria-current={selected}
                title={change.oldPath ? `${change.oldPath} → ${change.path}` : change.path}
                onClick={() => onOpen({ kind: "diff", change, scope })}
                className="flex h-8 w-full items-center gap-2 px-3.5 text-left hover:bg-raised aria-[current=true]:bg-raised"
              >
                <span
                  className={`w-3 shrink-0 text-center font-mono text-small ${kind.colour}`}
                  title={kind.label}
                >
                  {kind.letter}
                </span>
                <span className={`truncate ${change.kind === "deleted" ? "line-through" : ""}`}>
                  {change.path.slice(slash + 1)}
                </span>
                <span className="min-w-0 flex-1 truncate text-small text-ink-faint" dir="rtl">
                  {slash > 0 ? change.path.slice(0, slash) : ""}
                </span>
                <span className="shrink-0 font-mono text-small">
                  {change.additions != null && (
                    <span className="text-green-400">+{change.additions}</span>
                  )}{" "}
                  {change.deletions != null && (
                    <span className="text-red-400">−{change.deletions}</span>
                  )}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** One folder as that computer listed it, and at which reading. */
interface Listed {
  revision: number | null;
  entries: FileEntry[];
  more: boolean;
}

interface TreeProps {
  hostId: string;
  workspaceId: string;
  /** Changes when that computer was read again: open folders are listed again. */
  revision: number | null;
  viewing: Viewing | null;
  onOpen: (viewing: Viewing) => void;
  /** The folders that are open, by their path. Kept by the panel, so laying the list out again
   *  does not fold the tree. */
  openDirs: ReadonlySet<string>;
  onToggle: (dir: string) => void;
  /** What each folder listed at the last reading, so a folder drawn again is not asked again. */
  known: Map<string, Listed>;
}

/** One folder of that workspace's tree, listed when it is opened. */
function Folder(props: TreeProps & { dir: string; depth: number }) {
  const { hostId, workspaceId, revision, dir, depth, known } = props;
  const [listed, setListed] = useState<Listed | null>(() => known.get(dir) ?? null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Listed at this reading already, by this folder drawn before: not asked again.
    if (known.get(dir)?.revision === revision) return;
    let stale = false;
    ipc.otherHostFiles(hostId, workspaceId, dir).then(
      (list) => {
        if (stale) return;
        const now = { ...list, revision };
        known.set(dir, now);
        setListed(now);
        setError(null);
      },
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [hostId, workspaceId, dir, revision, known]);

  if (error) return <li className="px-3 py-1 text-red-400 select-text">{error}</li>;
  if (!listed) return null;
  if (listed.entries.length === 0 && depth === 0) {
    return <li className="p-3 text-ink-faint">No files.</li>;
  }
  return (
    <>
      {listed.entries.map((entry) => (
        <Entry key={entry.path} {...props} entry={entry} />
      ))}
      {listed.more && (
        <li
          style={{ paddingLeft: 27 + depth * 14 }}
          className="py-1 pr-2 text-small text-ink-faint"
        >
          more than are listed
        </li>
      )}
    </>
  );
}

function Entry(props: TreeProps & { entry: FileEntry; depth: number }) {
  const { entry, depth, viewing, onOpen, openDirs, onToggle } = props;
  const open = entry.isDir && openDirs.has(entry.path);
  const selected = viewing?.kind === "file" && viewing.path === entry.path;
  return (
    <li
      role="treeitem"
      aria-expanded={entry.isDir ? open : undefined}
      aria-selected={selected}
      aria-label={entry.name}
    >
      <button
        type="button"
        onClick={() =>
          entry.isDir ? onToggle(entry.path) : onOpen({ kind: "file", path: entry.path })
        }
        style={{ paddingLeft: 12 + depth * 14 }}
        className={`flex h-7 w-full items-center gap-1.5 pr-2 text-left hover:bg-raised ${
          selected ? "bg-raised text-ink" : "text-ink-muted"
        }`}
        title={entry.path}
      >
        <span aria-hidden className={`w-3 text-micro text-ink-faint ${open ? "rotate-90" : ""}`}>
          {entry.isDir ? "▶" : ""}
        </span>
        <span className="truncate">{entry.name}</span>
      </button>
      {entry.isDir && open && (
        <ul role="group">
          <Folder {...props} dir={entry.path} depth={depth + 1} />
        </ul>
      )}
    </li>
  );
}

/** The open file or diff, read-only, with its header. */
function Viewer({
  viewing,
  diff,
  file,
  error,
  mode,
  expanded,
  onToggleExpanded,
  onOpen,
}: {
  viewing: Viewing;
  diff: FileDiff | null;
  file: Content | null;
  error: string | null;
  mode: DiffMode;
  expanded: boolean;
  onToggleExpanded: () => void;
  onOpen: (viewing: Viewing | null) => void;
}) {
  const path = titleOf(viewing);
  const deleted = viewing.kind === "diff" && viewing.change.kind === "deleted";
  const headerButton =
    "shrink-0 rounded px-1.5 py-0.5 whitespace-nowrap text-ink-faint hover:bg-raised hover:text-ink";
  return (
    <section aria-label={`Viewing ${path}`} className="flex h-full min-h-0 flex-col">
      <header className="flex h-9 shrink-0 items-center gap-1 border-b border-line bg-surface px-2">
        <span className="min-w-0 flex-1 truncate font-mono text-detail" title={path}>
          {expanded && viewing.kind === "diff" && viewing.change.oldPath && (
            <span className="text-ink-faint">{viewing.change.oldPath} → </span>
          )}
          {expanded ? path : path.slice(path.lastIndexOf("/") + 1)}
        </span>
        {expanded && viewing.kind === "diff" && (
          <span className="shrink-0 text-small text-ink-faint">
            {viewing.scope === "uncommitted" ? "uncommitted" : "on this branch"}
          </span>
        )}
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
            onClick={() => onOpen({ kind: "file", path })}
          >
            Open file
          </button>
        )}
        <button type="button" onClick={onToggleExpanded} className={headerButton}>
          {expanded ? "Shrink" : "Expand"}
        </button>
        <button
          type="button"
          aria-label="Close viewer"
          onClick={() => onOpen(null)}
          className={headerButton}
        >
          ×
        </button>
      </header>
      <div className="min-h-0 flex-1 bg-canvas">
        {error ? (
          <Note alert>{error}</Note>
        ) : viewing.kind === "diff" ? (
          <DiffBody path={path} diff={diff} mode={mode} />
        ) : !file ? (
          <Note>Loading…</Note>
        ) : file.type === "absent" ? (
          <Note>This file no longer exists.</Note>
        ) : obstacle(file) ? (
          <Note>{obstacle(file)}</Note>
        ) : file.type !== "text" ? (
          <Note>Cannot display this file.</Note>
        ) : (
          <Suspense fallback={<Note>Loading…</Note>}>
            <CodeView path={path} text={file.text} />
          </Suspense>
        )}
      </div>
    </section>
  );
}

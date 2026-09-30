import { useFileContextMenu } from "./useFileContextMenu";
import { useCallback, useEffect, useState } from "react";
import { errorMessage, ipc, type FileEntry } from "@/lib/ipc";
import { useChangesStore } from "@/stores/changes";

/**
 * The workspace's files, one folder at a time: a folder's contents are only asked for when it is
 * opened, so the size of the repository never matters. `revision` bumps on every file-system
 * signal and makes open folders reload.
 */
interface TreeProps {
  workspaceId: string;
  revision: number;
  /** Also list `.git` and what the ignore rules exclude, dimmed. */
  showIgnored: boolean;
}

export function FileTree(props: TreeProps) {
  return (
    <ul role="tree" aria-label="Files" className="min-h-0 flex-1 overflow-y-auto py-1">
      <Folder {...props} dir="" depth={0} insideIgnored={false} />
    </ul>
  );
}

/** `insideIgnored`: the rules name an ignored *folder*, not its contents, so the flag is inherited. */
function Folder(props: TreeProps & { dir: string; depth: number; insideIgnored: boolean }) {
  const { workspaceId, dir, depth, revision, showIgnored } = props;
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    ipc.workspaceFiles(workspaceId, dir, showIgnored).then(
      (list) => !stale && (setEntries(list), setError(null)),
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [workspaceId, dir, revision, showIgnored]);

  if (error) return <li className="px-3 py-1 text-red-400">{error}</li>;
  if (!entries) return null;
  if (entries.length === 0 && depth === 0) return <li className="p-3 text-ink-faint">No files.</li>;
  return entries.map((entry) => (
    <Entry
      key={entry.path}
      {...props}
      entry={entry}
      ignored={props.insideIgnored || entry.ignored}
    />
  ));
}

function Entry(props: TreeProps & { entry: FileEntry; depth: number; ignored: boolean }) {
  const { entry, depth, ignored } = props;
  const context = useFileContextMenu(props.workspaceId);
  const [open, setOpen] = useState(false);
  const selected = useChangesStore(
    (s) => s.viewing?.kind === "file" && s.viewing.path === entry.path,
  );
  const view = useChangesStore((s) => s.view);

  const activate = useCallback(() => {
    if (entry.isDir) setOpen((value) => !value);
    else void view({ kind: "file", path: entry.path });
  }, [entry, view]);

  return (
    <li
      role="treeitem"
      aria-expanded={entry.isDir ? open : undefined}
      aria-selected={selected}
      aria-label={entry.name}
    >
      <button
        type="button"
        onClick={activate}
        onContextMenu={(event) => context.onContextMenu(event, entry.path)}
        style={{ paddingLeft: 12 + depth * 14 }}
        className={`flex h-6 w-full items-center gap-1.5 pr-2 text-left hover:bg-raised ${
          selected ? "bg-raised text-ink" : ignored ? "text-ink-faint italic" : "text-ink-muted"
        }`}
        title={ignored ? `${entry.path} — ignored by git` : entry.path}
      >
        <span aria-hidden className={`w-3 text-[9px] text-ink-faint ${open ? "rotate-90" : ""}`}>
          {entry.isDir ? "▶" : ""}
        </span>
        <span className="truncate">{entry.name}</span>
      </button>
      {context.menu}
      {entry.isDir && open && (
        <ul role="group">
          <Folder {...props} dir={entry.path} depth={depth + 1} insideIgnored={ignored} />
        </ul>
      )}
    </li>
  );
}

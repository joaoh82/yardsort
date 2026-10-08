import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { native } from "@/lib/native";
import { errorMessage, type RemoteRepository } from "@/lib/ipc";
import { age } from "@/features/pull-requests/rows";
import { useProjectsStore, type RemoteRepositories } from "@/stores/projects";
import { enterWorkspace, openProjectFromDisk } from "./actions";

/** Add an existing folder, create a repository, or clone one from the forge. */
export function AddProjectDialog({ onClose }: { onClose: () => void }) {
  const [mode, setMode] = useState<"choose" | "create" | "clone">("choose");
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) =>
      event.key === "Escape" && (!busy || mode === "clone") && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy, mode]);

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[18vh]"
      onPointerDown={(event) =>
        event.target === event.currentTarget && (!busy || mode === "clone") && onClose()
      }
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={`${mode === "clone" ? "w-[36rem]" : "w-[28rem]"} max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50`}
      >
        <h2 id={titleId} className="text-base font-semibold">
          {mode === "choose"
            ? "Add a project"
            : mode === "clone"
              ? "Clone a repository"
              : "Create a project"}
        </h2>
        {mode === "choose" ? (
          <div className="mt-4 grid gap-2">
            <Choice
              autoFocus
              title="Open a folder"
              detail="Add a repository you already have on this computer."
              onClick={async () => {
                onClose();
                await openProjectFromDisk();
              }}
            />
            <Choice
              title="Clone a repository"
              detail="Pick one of yours on GitHub, search it, or paste a URL from any host."
              onClick={() => setMode("clone")}
            />
            <Choice
              title="Create a new project"
              detail="Make a new folder with an empty git repository in it."
              onClick={() => setMode("create")}
            />
          </div>
        ) : mode === "clone" ? (
          <CloneForm
            busy={busy}
            setBusy={setBusy}
            onBack={() => setMode("choose")}
            onDone={onClose}
          />
        ) : (
          <CreateForm
            busy={busy}
            setBusy={setBusy}
            onBack={() => setMode("choose")}
            onDone={onClose}
          />
        )}
      </div>
    </div>
  );
}

function Choice(props: {
  title: string;
  detail: string;
  autoFocus?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      autoFocus={props.autoFocus}
      onClick={props.onClick}
      className="rounded-md border border-line px-4 py-3 text-left outline-none hover:border-accent focus-visible:border-accent"
    >
      <div className="font-medium">{props.title}</div>
      <div className="mt-0.5 text-ink-muted">{props.detail}</div>
    </button>
  );
}

const field =
  "w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none focus:border-accent";

type FormProps = {
  onBack: () => void;
  onDone: () => void;
  busy: boolean;
  setBusy: (value: boolean) => void;
};

/** The folder name the clone or the new project gets, from a URL, `owner/name` or a name. */
function suggestedName(repository: string): string {
  return (
    repository
      .trim()
      .replace(/\/+$/, "")
      .split("/")
      .at(-1)
      ?.replace(/\.git$/, "") ?? ""
  );
}

function useMounted() {
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return mounted;
}

/** Name and location, shared by cloning and creating. `collapsed` shows a remembered location
 *  as one line with **Change…**, so the usual path asks nothing about folders. */
function Destination({
  name,
  setName,
  nameRef,
  parent,
  setParent,
  collapsed,
  onChange,
  cloning,
  ready,
}: {
  name: string;
  setName: (value: string) => void;
  nameRef?: React.RefObject<HTMLInputElement | null>;
  parent: string;
  setParent: (value: string) => void;
  collapsed: boolean;
  onChange: () => void;
  cloning: boolean;
  ready: boolean;
}) {
  const browse = async () => {
    const folder = await native.pickFolder(
      cloning ? "Clone the repository into…" : "Create the project in…",
      parent || undefined,
    );
    if (folder) setParent(folder);
  };
  const separator = parent.includes("\\") ? "\\" : "/";
  return (
    <>
      <label className="grid gap-1">
        <span className="text-ink-muted">Name</span>
        <input
          ref={nameRef}
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="my-project"
          spellCheck={false}
          autoComplete="off"
          className={`${field} select-text`}
        />
      </label>
      {collapsed ? (
        <div className="flex items-baseline gap-2">
          <span className="text-ink-muted">Location</span>
          <span className="min-w-0 flex-1 truncate font-mono text-[12px] select-text">
            {parent}
          </span>
          <button
            type="button"
            onClick={onChange}
            className="text-ink-muted underline hover:text-ink"
          >
            Change…
          </button>
        </div>
      ) : (
        <div className="grid gap-1">
          <span className="text-ink-muted">Location</span>
          <div className="flex gap-2">
            <input
              aria-label="Location"
              value={parent}
              onChange={(event) => setParent(event.target.value)}
              placeholder="Choose a folder…"
              spellCheck={false}
              className={`${field} min-w-0 flex-1 font-mono text-[12px] select-text`}
            />
            <button
              type="button"
              onClick={browse}
              className="rounded border border-line px-3 hover:border-accent"
            >
              Browse…
            </button>
          </div>
        </div>
      )}
      <p className="min-h-4 truncate font-mono text-[11px] text-ink-faint">
        {ready || (name.trim() && parent)
          ? `${parent.replace(/[\\/]+$/, "")}${separator}${name.trim()}`
          : ""}
      </p>
    </>
  );
}

function Footer({ onBack, ready, label }: { onBack: () => void; ready: boolean; label: string }) {
  return (
    <div className="flex justify-between">
      <button type="button" onClick={onBack} className="px-2 py-1.5 text-ink-muted hover:text-ink">
        ← Back
      </button>
      <button
        type="submit"
        disabled={!ready}
        className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
      >
        {label}
      </button>
    </div>
  );
}

function CreateForm({ onBack, onDone, busy, setBusy }: FormProps) {
  const lastParentDir = useProjectsStore((s) => s.lastParentDir);
  const error = useProjectsStore((s) => s.error);
  const [name, setName] = useState("");
  const [parent, setParent] = useState(lastParentDir ?? "");
  const [changingLocation, setChangingLocation] = useState(!lastParentDir);
  const mounted = useMounted();
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    useProjectsStore.getState().dismiss();
    nameRef.current?.focus();
  }, []);

  const ready = name.trim() !== "" && parent.trim() !== "" && !busy;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    useProjectsStore.getState().dismiss();
    const created = await useProjectsStore.getState().createProject(name, parent);
    if (!mounted.current) return;
    setBusy(false);
    if (!created) return;
    const selected = useProjectsStore.getState().selectedWorkspaceId;
    if (selected) enterWorkspace(selected);
    onDone();
  };

  return (
    <form onSubmit={submit} className="mt-4">
      <fieldset disabled={busy} className="grid gap-3">
        <Destination
          name={name}
          setName={setName}
          nameRef={nameRef}
          parent={parent}
          setParent={setParent}
          collapsed={!changingLocation}
          onChange={() => setChangingLocation(true)}
          cloning={false}
          ready={ready}
        />
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
        <Footer onBack={onBack} ready={ready} label={busy ? "Creating…" : "Create project"} />
      </fieldset>
    </form>
  );
}

/** What the forge said to a search, while it is what the list shows. */
type Search = {
  text: string;
  status: "searching" | "ready" | "failed";
  list: RemoteRepository[];
  problem: string | null;
};

/** One line of the list under the field: a repository, the typed URL, or the search. */
type Row =
  | { kind: "repository"; repository: RemoteRepository }
  | { kind: "url"; text: string }
  | { kind: "search"; text: string };

const rowKey = (row: Row) =>
  row.kind === "repository" ? `r:${row.repository.nameWithOwner}` : row.kind;

/** Typed text that is a place to clone from rather than a word to look for. */
const looksLikeRepository = (text: string) => /^[^\s/]+\/[^\s/]+$/.test(text) || /[:/]/.test(text);

function CloneForm({ onBack, onDone, busy, setBusy }: FormProps) {
  const lastParentDir = useProjectsStore((s) => s.lastParentDir);
  const error = useProjectsStore((s) => s.error);
  const repositories = useProjectsStore((s) => s.repositories);
  const [text, setText] = useState("");
  const [chosen, setChosen] = useState<RemoteRepository | null>(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [parent, setParent] = useState(lastParentDir ?? "");
  const [changingLocation, setChangingLocation] = useState(!lastParentDir);
  const [search, setSearch] = useState<Search | null>(null);
  const [active, setActive] = useState(0);
  const mounted = useMounted();
  const inputRef = useRef<HTMLInputElement>(null);
  const formRef = useRef<HTMLFormElement>(null);
  const listId = useId();
  const [now] = useState(() => Date.now());

  useEffect(() => {
    useProjectsStore.getState().dismiss();
    inputRef.current?.focus();
    void useProjectsStore.getState().loadRepositories();
  }, []);

  const typed = text.trim();
  const needle = typed.toLowerCase();
  const rows = useMemo((): Row[] => {
    const rows: Row[] = [];
    if (!chosen && typed && looksLikeRepository(typed)) rows.push({ kind: "url", text: typed });
    if (search) {
      rows.push(...search.list.map((repository): Row => ({ kind: "repository", repository })));
      return rows;
    }
    const matching = needle
      ? repositories.list.filter(
          (repository) =>
            repository.nameWithOwner.toLowerCase().includes(needle) ||
            repository.description?.toLowerCase().includes(needle),
        )
      : repositories.list;
    rows.push(...matching.map((repository): Row => ({ kind: "repository", repository })));
    if (repositories.status === "ready" && typed.length >= 3 && !chosen)
      rows.push({ kind: "search", text: typed });
    return rows;
  }, [chosen, typed, needle, search, repositories]);

  // Rows come and go with the text; the index follows the shortest list it has seen since.
  const activeIndex = rows.length === 0 ? 0 : Math.min(active, rows.length - 1);

  const runSearch = async (what: string) => {
    setSearch({ text: what, status: "searching", list: [], problem: null });
    try {
      const list = await useProjectsStore.getState().searchRepositories(what);
      if (mounted.current) setSearch({ text: what, status: "ready", list, problem: null });
    } catch (error) {
      if (mounted.current)
        setSearch({ text: what, status: "failed", list: [], problem: errorMessage(error) });
    }
  };

  const choose = (repository: RemoteRepository) => {
    setChosen(repository);
    setText(repository.nameWithOwner);
    if (!nameEdited) setName(suggestedName(repository.nameWithOwner));
    inputRef.current?.focus();
  };

  const activate = (row: Row) => {
    if (row.kind === "repository") choose(row.repository);
    else if (row.kind === "search") void runSearch(row.text);
    else formRef.current?.requestSubmit();
  };

  const alreadyAdded = chosen?.projectId ?? null;
  const ready =
    name.trim() !== "" && parent.trim() !== "" && typed !== "" && !busy && !alreadyAdded;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (alreadyAdded) {
      const project = useProjectsStore.getState().projects.find((p) => p.id === alreadyAdded);
      const local = project?.workspaces.find((w) => w.kind === "local");
      if (local) enterWorkspace(local.id);
      onDone();
      return;
    }
    if (!ready) return;
    setBusy(true);
    useProjectsStore.getState().dismiss();
    const created = await useProjectsStore
      .getState()
      .cloneProject(
        chosen?.cloneUrl ?? typed,
        name,
        parent,
        () => mounted.current,
        chosen?.parent?.cloneUrl ?? null,
      );
    if (!mounted.current) return;
    setBusy(false);
    if (!created) return;
    const selected = useProjectsStore.getState().selectedWorkspaceId;
    if (selected) enterWorkspace(selected);
    onDone();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (rows.length === 0) return;
      const by = event.key === "ArrowDown" ? 1 : -1;
      setActive((activeIndex + by + rows.length) % rows.length);
    } else if (event.key === "Enter" && rows[activeIndex] && rows[activeIndex].kind !== "url") {
      event.preventDefault();
      activate(rows[activeIndex]);
    }
  };

  const activeId = rows[activeIndex] ? `${listId}-${rowKey(rows[activeIndex])}` : undefined;

  return (
    <form ref={formRef} onSubmit={submit} className="mt-4">
      <fieldset disabled={busy} className="grid gap-3">
        <div className="grid gap-1">
          <label className="grid gap-1">
            <span className="text-ink-muted">Repository</span>
            <input
              ref={inputRef}
              value={text}
              onChange={(event) => {
                setText(event.target.value);
                setChosen(null);
                setActive(0);
                if (search) setSearch(null);
                if (!nameEdited) setName(suggestedName(event.target.value));
              }}
              onKeyDown={onKeyDown}
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-activedescendant={activeId}
              aria-autocomplete="list"
              placeholder="Type to filter, or paste a URL"
              spellCheck={false}
              autoComplete="off"
              className={`${field} select-text`}
            />
          </label>
          <ul
            id={listId}
            role="listbox"
            aria-label={search ? "Repositories on GitHub" : "Your repositories"}
            className="max-h-56 overflow-y-auto rounded border border-line bg-canvas"
          >
            {rows.map((row, index) => (
              <RowItem
                key={rowKey(row)}
                id={`${listId}-${rowKey(row)}`}
                row={row}
                active={index === activeIndex}
                chosen={row.kind === "repository" && row.repository === chosen}
                now={now}
                onHover={() => setActive(index)}
                onPick={() => activate(row)}
              />
            ))}
            <ListNotice
              repositories={repositories}
              search={search}
              typed={typed}
              matching={rows.some((row) => row.kind === "repository")}
              onRetry={() => void useProjectsStore.getState().loadRepositories(true)}
              onBack={() => {
                setSearch(null);
                inputRef.current?.focus();
              }}
            />
          </ul>
          <span className="text-xs text-ink-faint">
            {chosen?.parent
              ? `A fork: ${chosen.parent.nameWithOwner} becomes its upstream remote.`
              : "owner/repository on GitHub, or an HTTPS or SSH URL from any host. Uses your existing git credentials."}
          </span>
        </div>
        <Destination
          name={name}
          setName={(value) => {
            setName(value);
            setNameEdited(true);
          }}
          parent={parent}
          setParent={setParent}
          collapsed={!changingLocation}
          onChange={() => setChangingLocation(true)}
          cloning
          ready={ready}
        />
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
        <Footer
          onBack={onBack}
          ready={ready || !!alreadyAdded}
          label={busy ? "Cloning…" : alreadyAdded ? "Go to project" : "Clone project"}
        />
      </fieldset>
      {busy && (
        <button
          type="button"
          onClick={onDone}
          className="mt-3 rounded border border-line px-3 py-1.5 hover:border-accent"
        >
          Run in background
        </button>
      )}
    </form>
  );
}

function RowItem({
  id,
  row,
  active,
  chosen,
  now,
  onHover,
  onPick,
}: {
  id: string;
  row: Row;
  active: boolean;
  chosen: boolean;
  now: number;
  onHover: () => void;
  onPick: () => void;
}) {
  const shade = active ? "bg-surface" : "";
  const common = {
    id,
    role: "option" as const,
    "aria-selected": chosen,
    onMouseMove: onHover,
    // Mouse down, not click: a click would blur the field first and move the list.
    onMouseDown: (event: React.MouseEvent) => {
      event.preventDefault();
      onPick();
    },
  };
  if (row.kind === "url") {
    return (
      <li {...common} className={`cursor-pointer px-3 py-1.5 ${shade}`}>
        Clone <span className="font-mono text-[12px] select-text">{row.text}</span>
      </li>
    );
  }
  if (row.kind === "search") {
    return (
      <li {...common} className={`cursor-pointer px-3 py-1.5 text-ink-muted ${shade}`}>
        Search GitHub for “{row.text}”…
      </li>
    );
  }
  const { repository } = row;
  const pushed = repository.pushedAt ? Date.parse(repository.pushedAt) : NaN;
  const facts = [
    repository.language,
    Number.isNaN(pushed) ? null : `pushed ${age(pushed, now)} ago`,
    repository.isPrivate ? "private" : null,
    repository.isArchived ? "archived" : null,
    repository.isFork
      ? repository.parent
        ? `fork of ${repository.parent.nameWithOwner}`
        : "fork"
      : null,
  ].filter((fact): fact is string => !!fact);
  return (
    <li
      {...common}
      aria-label={`${repository.nameWithOwner}${repository.projectId ? ", already added" : ""}`}
      className={`flex cursor-pointer items-baseline gap-2 px-3 py-1.5 ${shade} ${chosen ? "text-accent" : ""}`}
    >
      <span className="min-w-0 flex-1 truncate">
        <span className="font-medium">{repository.nameWithOwner}</span>
        {repository.description && (
          <span className="ml-2 text-ink-muted">{repository.description}</span>
        )}
      </span>
      <span className="shrink-0 text-xs text-ink-faint">{facts.join(" · ")}</span>
      {repository.projectId && (
        <span className="shrink-0 rounded bg-surface px-1.5 text-xs text-ink-muted">
          Already added
        </span>
      )}
    </li>
  );
}

/** What the list says besides its rows: that it is loading, why there is none, that there are
 *  more, or that the rows are the forge's. */
function ListNotice({
  repositories,
  search,
  typed,
  matching,
  onRetry,
  onBack,
}: {
  repositories: RemoteRepositories;
  search: Search | null;
  typed: string;
  matching: boolean;
  onRetry: () => void;
  onBack: () => void;
}) {
  const line = "px-3 py-1.5 text-xs text-ink-faint";
  const retry = (
    <button type="button" onClick={onRetry} className="ml-1 underline hover:text-ink">
      Retry
    </button>
  );
  if (search) {
    const back = (
      <button type="button" onClick={onBack} className="ml-1 underline hover:text-ink">
        Your repositories
      </button>
    );
    if (search.status === "searching")
      return <li className={line}>Searching GitHub for “{search.text}”…</li>;
    if (search.status === "failed")
      return (
        <li className={line} role="alert">
          {search.problem}
          {back}
        </li>
      );
    return (
      <li className={line}>
        {search.list.length === 0
          ? `Nothing on GitHub is named like “${search.text}”.`
          : `On GitHub, the ${search.list.length} the forge ranks first.`}
        {back}
      </li>
    );
  }
  if (repositories.status === "loading" || repositories.status === "idle")
    return <li className={line}>Reading your repositories…</li>;
  if (repositories.notInstalled)
    return (
      <li className={line}>
        Picking from your repositories needs the GitHub CLI. Install <code>gh</code>, run{" "}
        <code className="select-text">gh auth login</code>, or paste a URL.
      </li>
    );
  if (repositories.loggedOut)
    return (
      <li className={line}>
        Nobody is logged in to the GitHub CLI. Run{" "}
        <code className="select-text">gh auth login</code> in a terminal, then
        {retry}, or paste a URL.
      </li>
    );
  if (repositories.status === "failed" || (repositories.problem && repositories.list.length === 0))
    return (
      <li className={line} role="alert">
        {repositories.problem}
        {retry}
      </li>
    );
  const more =
    repositories.total !== null && repositories.total > repositories.list.length
      ? `Showing the ${repositories.list.length} most recently pushed of ${repositories.total}; search for the rest.`
      : null;
  if (!matching && typed)
    return <li className={line}>Nothing of yours is named like “{typed}”.</li>;
  if (!matching) return <li className={line}>You have no repositories on GitHub yet.</li>;
  if (repositories.problem)
    return (
      <li className={line}>
        Stopped short: {repositories.problem}
        {retry}
      </li>
    );
  return more ? <li className={line}>{more}</li> : null;
}

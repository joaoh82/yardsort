import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import { native } from "@/lib/native";
import { useProjectsStore } from "@/stores/projects";
import { enterWorkspace, openProjectFromDisk } from "./actions";

/** Add an existing folder, create a repository, or clone one from GitHub. */
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
        className="w-[28rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          {mode === "choose"
            ? "Add a project"
            : mode === "clone"
              ? "Clone a GitHub repository"
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
              title="Clone a GitHub repository"
              detail="Download a repository into a new local folder."
              onClick={() => setMode("clone")}
            />
            <Choice
              title="Create a new project"
              detail="Make a new folder with an empty git repository in it."
              onClick={() => setMode("create")}
            />
          </div>
        ) : (
          <CreateForm
            cloning={mode === "clone"}
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

function CreateForm({
  onBack,
  onDone,
  cloning,
  busy,
  setBusy,
}: {
  onBack: () => void;
  onDone: () => void;
  cloning: boolean;
  busy: boolean;
  setBusy: (value: boolean) => void;
}) {
  const lastParentDir = useProjectsStore((s) => s.lastParentDir);
  const error = useProjectsStore((s) => s.error);
  const [name, setName] = useState("");
  const [parent, setParent] = useState(lastParentDir ?? "");
  const [repository, setRepository] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const mounted = useRef(false);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    mounted.current = true;
    useProjectsStore.getState().dismiss();
    nameRef.current?.focus();
    return () => {
      mounted.current = false;
    };
  }, []);

  const browse = async () => {
    const folder = await native.pickFolder(
      cloning ? "Clone the repository into…" : "Create the project in…",
      parent || undefined,
    );
    if (folder) setParent(folder);
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    useProjectsStore.getState().dismiss();
    const created = cloning
      ? await useProjectsStore
          .getState()
          .cloneProject(repository, name, parent, () => mounted.current)
      : await useProjectsStore.getState().createProject(name, parent);
    if (!mounted.current) return;
    setBusy(false);
    if (!created) return;
    const selected = useProjectsStore.getState().selectedWorkspaceId;
    if (selected) enterWorkspace(selected);
    onDone();
  };

  const separator = parent.includes("\\") ? "\\" : "/";
  const ready =
    name.trim() !== "" && parent.trim() !== "" && (!cloning || repository.trim() !== "") && !busy;
  const field =
    "w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none focus:border-accent";

  return (
    <form onSubmit={submit} className="mt-4">
      <fieldset disabled={busy} className="grid gap-3">
        {cloning && (
          <div className="grid gap-1">
            <label className="grid gap-1">
              <span className="text-ink-muted">GitHub repository</span>
              <input
                ref={cloning ? nameRef : undefined}
                value={repository}
                onChange={(event) => {
                  const value = event.target.value;
                  setRepository(value);
                  if (!nameEdited) {
                    setName(
                      value
                        .trim()
                        .replace(/\/+$/, "")
                        .split("/")
                        .at(-1)
                        ?.replace(/\.git$/, "") ?? "",
                    );
                  }
                }}
                placeholder="https://github.com/owner/repository"
                spellCheck={false}
                autoComplete="off"
                className={`${field} select-text`}
              />
            </label>
            <span className="text-xs text-ink-faint">
              HTTPS, SSH, or owner/repository. Uses your existing git credentials.
            </span>
          </div>
        )}
        <label className="grid gap-1">
          <span className="text-ink-muted">Name</span>
          <input
            ref={cloning ? undefined : nameRef}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setNameEdited(true);
            }}
            placeholder="my-project"
            spellCheck={false}
            autoComplete="off"
            className={`${field} select-text`}
          />
        </label>
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
        <p className="min-h-4 truncate font-mono text-[11px] text-ink-faint">
          {ready || (name.trim() && parent)
            ? `${parent.replace(/[\\/]+$/, "")}${separator}${name.trim()}`
            : ""}
        </p>
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="flex justify-between">
          <button
            type="button"
            onClick={onBack}
            className="px-2 py-1.5 text-ink-muted hover:text-ink"
          >
            ← Back
          </button>
          <button
            type="submit"
            disabled={!ready}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy
              ? cloning
                ? "Cloning…"
                : "Creating…"
              : cloning
                ? "Clone project"
                : "Create project"}
          </button>
        </div>
      </fieldset>
      {cloning && busy && (
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

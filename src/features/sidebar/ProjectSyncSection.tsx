import { useEffect, useState } from "react";
import { errorMessage, ipc, type Project, type ProjectSyncStatus } from "@/lib/ipc";
import { timeAgo } from "@/features/terminal/sessionText";
import { useLayoutStore } from "@/stores/layout";
import { LinkSyncedProjectDialog } from "./LinkSyncedProjectDialog";

const button =
  "rounded border border-line px-3 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40 disabled:hover:border-line disabled:hover:text-ink-muted";
const primary = "rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40";

/**
 * Whether a project syncs with the account's other computers. Its buttons act at once: they are
 * not part of the settings dialog's Save. What turning it on sends is shown before it goes, and
 * linking to a project another computer syncs is always the person's choice.
 */
export function ProjectSyncSection({
  project,
  onLeave,
}: {
  project: Project;
  /** Called before Settings → Remote opens, so the dialog this sits in can close first. */
  onLeave?: () => void;
}) {
  const [status, setStatus] = useState<ProjectSyncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const [linking, setLinking] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      ipc.projectSyncGet(project.id).then(
        (value) => !cancelled && setStatus(value),
        (reason) => !cancelled && setError(errorMessage(reason)),
      );
    void load();
    const unlisten = ipc.onSyncChanged(() => void load());
    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, [project.id]);

  const act = async (call: () => Promise<ProjectSyncStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await call());
      setAsking(false);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const sync = status?.sync;
  return (
    <fieldset disabled={!status || busy} className="mt-5 space-y-3 disabled:opacity-50">
      <legend className="font-semibold">Sync</legend>
      {sync?.state === "remoteOff" && (
        <p className="text-ink-muted">
          Sync needs Remote. Turn it on in{" "}
          <button
            type="button"
            onClick={() => {
              onLeave?.();
              useLayoutStore.getState().openSettings("remote");
            }}
            className="text-accent underline-offset-2 hover:underline"
          >
            Settings → Remote
          </button>
          .
        </p>
      )}
      {sync?.state === "off" && (
        <>
          <p className="text-ink-muted">
            Syncs this project with your other computers signed in to the same account: its approved
            notes and workspaces, as later updates bring them. Never its files, path or terminals.
          </p>
          <div className="rounded border border-line bg-canvas px-3 py-2">
            <p className="text-ink-faint">Turning it on sends:</p>
            <dl
              aria-label="What turning sync on sends"
              className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5"
            >
              <dt className="text-ink-muted">Name</dt>
              <dd className="min-w-0 truncate select-text">{sync.preview.name}</dd>
              <dt className="text-ink-muted">Remote</dt>
              <dd className="min-w-0 truncate font-mono select-text">
                {sync.preview.remote ?? "none"}
              </dd>
            </dl>
          </div>
          <p className="text-ink-faint">
            The server keeps these in plain text, readable by whoever runs it.
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void act(() => ipc.projectSyncOn(project.id))}
              className={primary}
            >
              Turn on sync
            </button>
            <button type="button" onClick={() => setLinking(true)} className={button}>
              Link to a synced project…
            </button>
          </div>
        </>
      )}
      {sync?.state === "on" && (
        <>
          {sync.refused !== null ? (
            <p role="alert" className="text-red-400 select-text">
              The server refused it: {sync.refused}
            </p>
          ) : (
            <p className="text-ink-muted">
              {sync.removing
                ? "Being removed from the server…"
                : !sync.reached
                  ? "Waiting to reach the server…"
                  : "Syncing with your other computers."}
            </p>
          )}
          {status?.problem ? (
            <p className="text-ink-faint select-text">Last attempt failed: {status.problem}</p>
          ) : (
            status?.syncedAt != null && (
              <p className="text-ink-faint">Last synced {timeAgo(status.syncedAt)}</p>
            )
          )}
          {asking ? (
            <div className="grid gap-2 rounded border border-red-400/40 bg-red-400/10 px-3 py-2">
              <p className="text-ink">
                This removes the project&rsquo;s records from the server for every computer. Each
                keeps its own copy of the project, and stops syncing it.
              </p>
              <div className="flex gap-2">
                <button type="button" autoFocus onClick={() => setAsking(false)} className={button}>
                  Keep it
                </button>
                <button
                  type="button"
                  onClick={() => void act(() => ipc.projectSyncRemove(project.id))}
                  className="rounded bg-red-500 px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
                >
                  Remove from the server
                </button>
              </div>
            </div>
          ) : (
            <div className="grid gap-1">
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => void act(() => ipc.projectSyncStop(project.id))}
                  className={button}
                >
                  Stop syncing
                </button>
                {!sync.removing && (
                  <button type="button" onClick={() => setAsking(true)} className={button}>
                    Remove from the server…
                  </button>
                )}
              </div>
              <p className="text-ink-faint">
                Stop syncing unlinks this computer. Your other computers keep syncing it.
              </p>
            </div>
          )}
        </>
      )}
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      {linking && (
        <LinkSyncedProjectDialog
          project={project}
          onClose={() => setLinking(false)}
          onLinked={(linked) => {
            setStatus(linked);
            setError(null);
          }}
        />
      )}
    </fieldset>
  );
}

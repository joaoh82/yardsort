import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useRef } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { useTerminalStore } from "@/stores/terminals";
import { useUpdatesStore } from "@/stores/updates";

const megabytes = (bytes: number) => `${(bytes / 1_048_576).toFixed(1)} MB`;

/** "A new version is available": what it is, what installing it costs you, and the button. */
export function UpdateDialog() {
  const status = useUpdatesStore((s) => s.status);
  const installing = useUpdatesStore((s) => s.installing);
  const progress = useUpdatesStore((s) => s.progress);
  const error = useUpdatesStore((s) => s.error);
  const running = useTerminalStore((s) => s.tabs.filter((tab) => !tab.exit).length);
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  useModalFocus(dialogRef);

  const close = () => !installing && useUpdatesStore.getState().show(false);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && close();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const update = status?.available;
  if (!status || !update) return null;
  const selfUpdating = status.installKind === "selfUpdating";
  const percent =
    progress?.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[14vh]"
      onPointerDown={(event) => event.target === event.currentTarget && close()}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[70vh] w-[32rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50 outline-none"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Yardsort {update.version} is available
        </h2>
        <p className="mt-1 text-ink-muted">You have {status.currentVersion}.</p>

        {update.notes && (
          <pre className="mt-3 min-h-0 flex-1 overflow-y-auto rounded bg-canvas p-3 font-sans text-detail whitespace-pre-wrap text-ink-muted select-text">
            {update.notes}
          </pre>
        )}

        {selfUpdating ? (
          running > 0 && (
            <p className="mt-3 text-ink-muted">
              <span className="font-medium text-ink">
                {running === 1 ? "1 terminal is" : `${running} terminals are`} running.
              </span>{" "}
              Installing restarts Yardsort, which stops them. Agent conversations are kept: pick the
              workspace afterwards and press Resume.
            </p>
          )
        ) : (
          <p className="mt-3 text-ink-muted">
            {status.installKind === "development"
              ? "This is a development build, which does not update itself."
              : "This copy of Yardsort was installed by a package manager, so it is updated through it — or download the new version from the release page."}
          </p>
        )}

        {installing && (
          <div className="mt-3" role="status">
            <div className="h-1.5 overflow-hidden rounded bg-raised">
              <div
                className={`h-full bg-accent transition-[width] ${percent === null ? "w-1/3 animate-pulse" : ""}`}
                style={percent === null ? undefined : { width: `${percent}%` }}
              />
            </div>
            <p className="mt-1 text-detail text-ink-faint">
              {percent === null
                ? `Downloading… ${megabytes(progress?.downloaded ?? 0)}`
                : percent < 100
                  ? `Downloading… ${percent}% of ${megabytes(progress!.total!)}`
                  : "Verifying and installing…"}
            </p>
          </div>
        )}
        {error && (
          <p role="alert" className="mt-3 text-red-400 select-text">
            {error}
          </p>
        )}

        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={() => void openUrl(update.url).catch(console.error)}
            className="mr-auto text-detail text-accent hover:underline"
          >
            Release page ↗
          </button>
          <button
            type="button"
            disabled={installing}
            onClick={close}
            className="px-3 py-1.5 text-ink-muted hover:text-ink disabled:opacity-40"
          >
            {selfUpdating ? "Later" : "Close"}
          </button>
          {selfUpdating && (
            <button
              type="button"
              disabled={installing}
              onClick={() => void useUpdatesStore.getState().install()}
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              {installing ? "Installing…" : "Install and restart"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

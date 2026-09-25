import { useState } from "react";
import { buttonClass, primaryButtonClass } from "@/features/settings/fields";
import { errorMessage, isIpcError, type YsStatus } from "@/lib/ipc";
import { usePreflightStore } from "@/stores/preflight";

/** The folder a path is in, however the OS separates them. */
function folderOf(path: string): string {
  return path.replace(/[\\/][^\\/]+$/, "") || path;
}

/** The `ys` a terminal finds is the one that came with this Yardsort. */
export function ysIsCurrent(ys: YsStatus): boolean {
  return ys.found !== null && ys.foundVersion === ys.version;
}

/** One line about `ys`, for a heading. */
export function ysSummary(ys: YsStatus): string {
  if (ysIsCurrent(ys)) return "The ys command is installed";
  if (ys.found !== null) {
    return ys.foundVersion === null
      ? "Another ys is on your PATH"
      : "The ys command is out of date";
  }
  if (ys.installed && !ys.targetOnPath) return "The ys command is not on your PATH";
  return "The ys command is not installed";
}

/**
 * Where `ys` stands and the button that fixes it. `ys` is optional — nothing in the app needs it
 * — so this informs rather than insists. See `src-tauri/src/ys.rs` for how each platform gets it.
 */
export function YsCommand({ ys }: { ys: YsStatus }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The file in the way, when the core refused to replace something that is not a `ys`. */
  const [conflict, setConflict] = useState<string | null>(null);

  const install = async (replace = false) => {
    setBusy(true);
    setError(null);
    setConflict(null);
    try {
      await usePreflightStore.getState().installYs(replace);
    } catch (reason) {
      if (isIpcError(reason) && reason.code === "ys_exists") setConflict(reason.message);
      else if (!(isIpcError(reason) && reason.code === "ys_cancelled"))
        setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const checking = usePreflightStore((s) => s.checking);
  const current = ysIsCurrent(ys);
  /** Installed, but a terminal cannot find it until the user fixes their `PATH`. */
  const offPath = ys.found === null && ys.installed && !ys.targetOnPath;
  const canInstall = ys.bundled !== null && ys.target !== null;
  const ours = ys.found !== null && ys.foundIsOurs;
  const asksForPassword = ys.method === "link" && ys.target?.startsWith("/usr/local/") === true;

  return (
    <div className="grid gap-2">
      <p className="text-ink-muted">
        {current ? (
          <>
            <code>ys</code> {ys.version} is at <Path>{ys.found!}</Path>.{" "}
            {ours && ys.method === "packaged"
              ? "Your package manager keeps it up to date with the app."
              : ours && ys.method === "link"
                ? "It points into the app, so it is updated whenever Yardsort is."
                : ours
                  ? "Yardsort updates it whenever it updates itself."
                  : null}
          </>
        ) : ys.found !== null ? (
          ys.foundVersion === null ? (
            <>
              The <code>ys</code> your terminal finds, <Path>{ys.found}</Path>, is not
              Yardsort&apos;s.
            </>
          ) : (
            <>
              Your terminal finds <code>ys</code> {ys.foundVersion} at <Path>{ys.found}</Path>, but
              this is Yardsort {ys.version}.
            </>
          )
        ) : offPath && ys.target ? (
          <>
            <code>ys</code> is at <Path>{ys.target}</Path>, but <Path>{folderOf(ys.target)}</Path>{" "}
            is not on your <code>PATH</code>. Add it in your shell&apos;s startup file, then check
            again.
          </>
        ) : (
          <>
            <code>ys</code> starts and deletes workspaces and agents from a terminal or a script.
            {ys.bundled === null && " This copy of Yardsort does not include it."}
          </>
        )}
      </p>

      {!current && ys.found !== null && !ours && canInstall && (
        <p className="text-[12px] text-ink-faint">
          Installing puts Yardsort&apos;s own at <Path>{ys.target!}</Path>. The one above comes
          first on your <code>PATH</code>, so remove it afterwards.
        </p>
      )}
      {!current && ys.found === null && canInstall && !ys.installed && (
        <p className="text-[12px] text-ink-faint">
          It goes to <Path>{ys.target!}</Path>
          {asksForPassword && ", so macOS will ask for your password"}.
          {!ys.targetOnPath && (
            <>
              {" "}
              That folder is not on your <code>PATH</code> yet: add it in your shell&apos;s startup
              file.
            </>
          )}
        </p>
      )}

      {conflict ? (
        <div role="alertdialog" aria-label="Replace the file?" className="grid gap-2">
          <p className="text-ink select-text">{conflict} Replace it?</p>
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void install(true)}
              className="h-8 rounded bg-red-500 px-4 font-medium text-canvas disabled:opacity-40"
            >
              Replace it
            </button>
            <button type="button" onClick={() => setConflict(null)} className={buttonClass}>
              Cancel
            </button>
          </div>
        </div>
      ) : offPath ? (
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={checking}
            onClick={() => void usePreflightStore.getState().check(true)}
            className={buttonClass}
          >
            {checking ? "Checking…" : "Check again"}
          </button>
        </div>
      ) : (
        !current &&
        canInstall && (
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void install()}
              className={primaryButtonClass}
            >
              {busy ? "Installing…" : ours ? `Update to ${ys.version}` : "Install ys"}
            </button>
          </div>
        )
      )}
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
    </div>
  );
}

function Path({ children }: { children: string }) {
  return <code className="font-mono text-[12px] break-all text-ink select-text">{children}</code>;
}

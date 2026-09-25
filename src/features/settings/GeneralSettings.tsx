import { useEffect, useState } from "react";
import { errorMessage, ipc, type SettingsInfo } from "@/lib/ipc";
import { useAppStore } from "@/stores/app";
import { useUpdatesStore } from "@/stores/updates";
import { ActivitySettings } from "./ActivitySettings";
import { buttonClass, Field, inputClass, primaryButtonClass } from "./fields";

/** The running version, and a way to look for a newer one right now. */
function UpdateNow() {
  const version = useAppStore((s) => s.info?.version);
  const status = useUpdatesStore((s) => s.status);
  const checking = useUpdatesStore((s) => s.checking);
  const error = useUpdatesStore((s) => s.error);
  const { check, show } = useUpdatesStore.getState();

  return (
    <div className="flex flex-wrap items-center gap-3 rounded border border-line px-3 py-2">
      <span className="font-mono text-[12px] text-ink-muted">Yardsort {version ?? "…"}</span>
      <button
        type="button"
        disabled={checking}
        onClick={() => void check({ manual: true })}
        className={buttonClass}
      >
        {checking ? "Checking…" : "Check now"}
      </button>
      {error ? (
        <span role="alert" className="text-red-400 select-text">
          {error}
        </span>
      ) : status?.available ? (
        <button type="button" onClick={() => show(true)} className="text-accent hover:underline">
          Version {status.available.version} is available →
        </button>
      ) : (
        status && <span className="text-ink-faint">You have the latest version.</span>
      )}
    </div>
  );
}

export function GeneralSettings() {
  const [info, setInfo] = useState<SettingsInfo | null>(null);
  const [editor, setEditor] = useState("");
  const [notify, setNotify] = useState(true);
  const [autoCheck, setAutoCheck] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const adopt = (next: SettingsInfo) => {
    setInfo(next);
    setEditor(next.editorCommand ?? "");
    setNotify(next.notifyWhenQuiet);
    setAutoCheck(next.checkForUpdates);
    useAppStore.setState({
      notifyWhenQuiet: next.notifyWhenQuiet,
      checkForUpdates: next.checkForUpdates,
    });
  };
  useEffect(() => {
    ipc.settingsGet().then(adopt, (reason) => setError(errorMessage(reason)));
  }, []);

  if (!info) return <p className="p-5 text-ink-faint">{error ?? "Loading…"}</p>;
  const dirty =
    editor.trim() !== (info.editorCommand ?? "") ||
    notify !== info.notifyWhenQuiet ||
    autoCheck !== info.checkForUpdates;

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      adopt(
        await ipc.settingsSaveGeneral({
          editorCommand: editor.trim() || null,
          notifyWhenQuiet: notify,
          checkForUpdates: autoCheck,
        }),
      );
      setSaved(true);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <form aria-label="General settings" onSubmit={save} className="grid max-w-2xl gap-4 p-5">
      <Field
        label="Editor command"
        hint={
          <>
            Used by “Open in editor”. It is given the workspace folder, then the file — for example{" "}
            <code>code</code>, <code>cursor</code> or <code>zed</code>. Leave empty to try the
            common editors in turn.
          </>
        }
      >
        <input
          value={editor}
          placeholder="auto-detect"
          spellCheck={false}
          onChange={(e) => {
            setEditor(e.target.value);
            setSaved(false);
          }}
          className={`${inputClass} w-72 font-mono text-[12px]`}
        />
      </Field>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={notify}
          onChange={(e) => {
            setNotify(e.target.checked);
            setSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Notify me when an agent finishes
          <span className="block text-ink-faint">
            A desktop notification when an agent that worked for a while goes quiet and Yardsort is
            not the window you are looking at.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={autoCheck}
          onChange={(e) => {
            setAutoCheck(e.target.checked);
            setSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Check for updates automatically
          <span className="block text-ink-faint">
            Looks at the GitHub release page shortly after starting and once a day. Nothing is
            downloaded or installed until you say so.
          </span>
        </span>
      </label>
      <UpdateNow />
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <button type="submit" disabled={!dirty} className={primaryButtonClass}>
          Save
        </button>
        {saved && !dirty && <span className="text-ink-faint">Saved.</span>}
      </div>
      <ActivitySettings initial={info.activity} />
    </form>
  );
}

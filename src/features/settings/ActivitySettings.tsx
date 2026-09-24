import { useEffect, useState } from "react";
import { errorMessage, ipc, type ActivityDiagnostics, type ActivitySettingsDto } from "@/lib/ipc";
import { useAppStore } from "@/stores/app";
import { buttonClass, primaryButtonClass } from "./fields";

/**
 * The activity switches, and what has been recorded. Its own form inside General: the two
 * checkboxes save on their own, and the diagnostics beneath them are read-only.
 */
export function ActivitySettings({ initial }: { initial: ActivitySettingsDto }) {
  const [saved, setSaved] = useState(initial);
  const [record, setRecord] = useState(initial.recordLifecycle);
  const [timeline, setTimeline] = useState(initial.showTimeline);
  const [justSaved, setJustSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<ActivityDiagnostics | null>(null);

  const loadDiagnostics = () =>
    ipc.activityDiagnostics().then(setDiagnostics, (reason) => setError(errorMessage(reason)));
  useEffect(() => {
    void loadDiagnostics();
  }, []);

  const dirty = record !== saved.recordLifecycle || timeline !== saved.showTimeline;
  const save = async () => {
    setError(null);
    try {
      const info = await ipc.settingsSaveActivity({
        recordLifecycle: record,
        showTimeline: timeline,
      });
      setSaved(info.activity);
      useAppStore.setState({ showTimeline: info.activity.showTimeline });
      setJustSaved(true);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };
  const clearAll = async () => {
    setError(null);
    try {
      await ipc.activityClear(null);
      await loadDiagnostics();
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <fieldset aria-label="Activity settings" className="mt-4 grid gap-3 border-t border-line pt-4">
      <legend className="sr-only">Activity</legend>
      <div>
        <h3 className="font-semibold">Activity</h3>
        <p className="text-ink-faint">
          A local record of what Yardsort itself saw: when an agent, shell or run command started in
          a workspace and how it ended. It is kept in your Yardsort database and never leaves this
          machine. Nothing an agent prints or does is read.
        </p>
      </div>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={record}
          onChange={(e) => {
            setRecord(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Record when agents start and exit
          <span className="block text-ink-faint">
            Off, and nothing new is written; what is already recorded stays until cleared.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={timeline}
          onChange={(e) => {
            setTimeline(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Show the activity timeline (experimental)
          <span className="block text-ink-faint">
            An <strong>Activity</strong> button in each workspace&apos;s footer opens the list.
          </span>
        </span>
      </label>
      <div className="flex items-center gap-3">
        <button
          type="button"
          disabled={!dirty}
          onClick={() => void save()}
          className={primaryButtonClass}
        >
          Save activity settings
        </button>
        {justSaved && !dirty && <span className="text-ink-faint">Saved.</span>}
      </div>
      {diagnostics && (
        <div className="grid gap-1 rounded border border-line px-3 py-2 text-ink-muted">
          <div>
            {diagnostics.events} events across {diagnostics.runs} runs.{" "}
            {diagnostics.spoolPending > 0
              ? `${diagnostics.spoolPending} exit(s) waiting in the spool.`
              : "Nothing waiting in the exit spool."}
          </div>
          <div
            className="truncate font-mono text-[11px] text-ink-faint"
            title={diagnostics.spoolDir}
          >
            spool: {diagnostics.spoolDir}
          </div>
          {diagnostics.counters.length > 0 && (
            <ul className="text-[11px] text-ink-faint">
              {diagnostics.counters.map((counter) => (
                <li key={counter.name}>
                  {counter.name}: {counter.count}
                  {counter.lastDetail ? ` — ${counter.lastDetail}` : ""}
                </li>
              ))}
            </ul>
          )}
          <div>
            <button
              type="button"
              disabled={diagnostics.events === 0 && diagnostics.runs === 0}
              onClick={() => void clearAll()}
              className={buttonClass}
            >
              Clear all recorded activity
            </button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
    </fieldset>
  );
}

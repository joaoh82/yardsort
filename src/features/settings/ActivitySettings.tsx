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
  const [claude, setClaude] = useState(initial.captureClaude);
  const [codex, setCodex] = useState(initial.captureCodex);
  const [opencode, setOpencode] = useState(initial.captureOpencode);
  const [grok, setGrok] = useState(initial.captureGrok);
  const [omp, setOmp] = useState(initial.captureOmp);
  const [pi, setPi] = useState(initial.capturePi);
  const [cursor, setCursor] = useState(initial.captureCursor);
  const [justSaved, setJustSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<ActivityDiagnostics | null>(null);

  const loadDiagnostics = () =>
    ipc.activityDiagnostics().then(setDiagnostics, (reason) => setError(errorMessage(reason)));
  useEffect(() => {
    void loadDiagnostics();
  }, []);

  const dirty =
    record !== saved.recordLifecycle ||
    timeline !== saved.showTimeline ||
    claude !== saved.captureClaude ||
    codex !== saved.captureCodex ||
    opencode !== saved.captureOpencode ||
    grok !== saved.captureGrok ||
    omp !== saved.captureOmp ||
    pi !== saved.capturePi ||
    cursor !== saved.captureCursor;
  const save = async () => {
    setError(null);
    try {
      const info = await ipc.settingsSaveActivity({
        recordLifecycle: record,
        showTimeline: timeline,
        // Capture needs recording: without a run there is nothing to link a report to.
        captureClaude: claude && record,
        captureCodex: codex && record,
        captureOpencode: opencode && record,
        captureGrok: grok && record,
        captureOmp: omp && record,
        capturePi: pi && record,
        captureCursor: cursor && record,
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
          machine. Nothing an agent prints is read; what an agent <em>reports</em> is a separate
          switch, per agent, below.
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
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={claude}
          disabled={!record}
          onChange={(e) => {
            setClaude(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what Claude Code reports
          <span className="block text-ink-faint">
            Claude Code started from Yardsort is given hooks that report each prompt, tool, turn and
            session end — the tool&apos;s name and the file&apos;s path, never the prompt, the
            command or the output. Your own Claude Code settings are not touched: the hooks ride on
            a per-launch settings file and are gone when this is off.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={codex}
          disabled={!record}
          onChange={(e) => {
            setCodex(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what Codex reports
          <span className="block text-ink-faint">
            Codex started from Yardsort is given a <code>notify</code> program that reports the end
            of each turn, and the turn&apos;s commands, file changes and token usage are read from
            Codex&apos;s own session file — exit codes and paths, never a command or a message. Your
            Codex configuration is not edited; a <code>notify</code> of your own still runs.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={opencode}
          disabled={!record}
          onChange={(e) => {
            setOpencode(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what OpenCode reports
          <span className="block text-ink-faint">
            OpenCode started from Yardsort is given a small plugin, through its environment for that
            launch alone, that reports each prompt, tool, file edit, permission prompt and turn with
            its token usage — names, paths, exit codes and counts, never a command, a file&apos;s
            contents or a message. Your OpenCode configuration is not edited and your own plugins
            keep running.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={grok}
          disabled={!record}
          onChange={(e) => {
            setGrok(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Read what Grok records
          <span className="block text-ink-faint">
            Grok keeps its own log of each session — which tool ran and how long it took, the
            permissions you were asked for, each turn and its tokens — with no commands, paths or
            messages in it. Yardsort reads that log for the sessions it started. Nothing is given to
            Grok and nothing in <code>~/.grok</code> is written.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={omp}
          disabled={!record}
          onChange={(e) => {
            setOmp(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what OMP reports
          <span className="block text-ink-faint">
            OMP started from Yardsort is given a small extension, on its command line for that
            launch alone, that reports each turn with its tokens, each tool with its file or
            duration, and each permission you answer — never a prompt, a command, a file or a reply.
            Your own OMP extensions keep running.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={pi}
          disabled={!record}
          onChange={(e) => {
            setPi(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what pi reports
          <span className="block text-ink-faint">
            The same extension, for pi: turns with their tokens, tools with their file or duration,
            model switches. Your own pi extensions keep running.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={cursor}
          disabled={!record}
          onChange={(e) => {
            setCursor(e.target.checked);
            setJustSaved(false);
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Capture what Cursor reports
          <span className="block text-ink-faint">
            The Cursor agent started from Yardsort is given a small plugin, on its command line for
            that launch alone, whose hooks report each tool with its file or duration and each file
            it changes — never the prompt, a command, a file or a reply. Your own Cursor hooks keep
            running.
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
          <div>
            {diagnostics.inboxPending > 0
              ? `${diagnostics.inboxPending} reported event(s) waiting in the inbox.`
              : "Nothing waiting in the agents' inbox."}
          </div>
          <div
            className="truncate font-mono text-[11px] text-ink-faint"
            title={diagnostics.inboxDir}
          >
            inbox: {diagnostics.inboxDir}
          </div>
          <div
            className="truncate font-mono text-[11px] text-ink-faint"
            title={diagnostics.claudeHooksFile}
          >
            Claude Code hooks: {diagnostics.claudeHooksFile}
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

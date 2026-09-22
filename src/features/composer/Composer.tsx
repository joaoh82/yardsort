import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  ipc,
  type BranchList,
  type HarnessInfo,
  type Project,
  type Suggestion,
  type Workspace,
} from "@/lib/ipc";
import { formatShortcut } from "@/lib/platform";
import { assistOn, useAssistStore } from "@/stores/assist";
import { useHarnessStore } from "@/stores/harnesses";
import { recall, useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";

/** What was picked last time in a project, offered again next time. */
interface LastPicks {
  harness?: string;
  model?: string;
  effort?: string;
}
const picksKey = (projectId: string) => `composer.last.${projectId}`;

const control =
  "h-8 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";

/** How long typing must pause before Assist is asked about the message. */
const SUGGEST_DELAY_MS = 800;
/** Shorter than this there is nothing to judge, and the core refuses to ask anyway. */
const SUGGEST_FROM_CHARS = 15;

/**
 * The start of every workspace: say what you want, pick who does it, press Enter. Nothing exists
 * until then — and if any part of starting fails, nothing is left behind and the message stays.
 *
 * With `runIn`, it starts the agent in a workspace that already exists instead of creating one.
 * That is how `local` takes a message: its checkout is the repository itself, on whatever branch
 * is checked out, so there is no branch to pick and no worktree to make.
 */
export function Composer({ project, runIn }: { project: Project; runIn?: Workspace }) {
  const allHarnesses = useHarnessStore((s) => s.harnesses);
  const harnesses = useMemo(() => allHarnesses.filter((h) => h.enabled), [allHarnesses]);
  const harnessesLoaded = useHarnessStore((s) => s.loaded);
  const ui = useProjectsStore((s) => s.ui);
  const last = useMemo(() => recall<LastPicks>(ui, picksKey(project.id), {}), [ui, project.id]);

  const [message, setMessage] = useState("");
  const [harnessId, setHarnessId] = useState<string | null>(null);
  const [model, setModel] = useState(last.model ?? "");
  const [effort, setEffort] = useState(last.effort ?? "");
  const [branches, setBranches] = useState<BranchList | null>(null);
  // "new:<branch>" starts a new branch from <branch>; "open:<branch>" opens <branch> itself.
  const [base, setBase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const messageRef = useRef<HTMLTextAreaElement>(null);
  const modelsId = useId();
  const assist = useAssistStore((s) => s.status);
  // Kept together with the message it was asked about, so an answer for older text is ignored.
  const [suggested, setSuggested] = useState<{ forMessage: string; suggestion: Suggestion } | null>(
    null,
  );

  useEffect(() => void useHarnessStore.getState().load(), []);
  useEffect(() => void useAssistStore.getState().load(), []);
  useEffect(() => messageRef.current?.focus(), [project.id]);

  // Ask Assist what suits the message, once typing pauses. A failure simply offers nothing.
  useEffect(() => {
    const text = message.trim();
    if (!assist?.suggestInComposer || !assistOn(assist) || text.length < SUGGEST_FROM_CHARS) return;
    let stale = false;
    const timer = setTimeout(() => {
      ipc.assistSuggest(text).then(
        (suggestion) => !stale && setSuggested({ forMessage: text, suggestion }),
        () => {},
      );
    }, SUGGEST_DELAY_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [message, assist]);

  useEffect(() => {
    // Running in a workspace that already exists picks no branch, so there is no list to fetch.
    if (runIn) return;
    let stale = false;
    ipc.projectBranches(project.id).then(
      (list) => {
        if (stale) return;
        setBranches(list);
        const start = list.default ?? list.branches[0];
        setBase(start ? `new:${start}` : "");
      },
      (reason) => !stale && setError(reason?.message ?? String(reason)),
    );
    return () => {
      stale = true;
    };
  }, [project.id, runIn]);

  // Prefer what was used last here, then the first harness that is actually installed.
  const installed = harnesses.filter((h) => h.resolvedPath);
  const harness: HarnessInfo | undefined =
    harnesses.find((h) => h.id === harnessId) ??
    installed.find((h) => h.id === last.harness) ??
    installed[0] ??
    harnesses[0];
  const effortChoice = harness?.efforts.includes(effort) ? effort : "";

  const chooseHarness = (id: string) => {
    setHarnessId(id);
    // A model name means nothing to a different harness.
    if (id !== harness?.id) setModel("");
  };

  // Only ever offered, and only the parts that differ from what is picked right now.
  const suggestion = suggested?.forMessage === message.trim() ? suggested.suggestion : null;
  const suggestedHarness = harnesses.find(
    (h) => h.id === suggestion?.harnessId && h.id !== harness?.id && !!h.resolvedPath,
  );
  const forHarness = (suggestedHarness ?? harness)?.id ?? "";
  const suggestedEffort = suggestion?.effortByHarness[forHarness];
  const offeredEffort = suggestedEffort === effortChoice ? undefined : suggestedEffort;
  const offering = suggestedHarness ?? offeredEffort;
  const takeSuggestion = () => {
    if (suggestedHarness) chooseHarness(suggestedHarness.id);
    if (offeredEffort) setEffort(offeredEffort);
    setSuggested(null);
  };

  const ready = !busy && !!harness?.resolvedPath && (!!runIn || base !== "");
  const [mode, branch] = [base.slice(0, base.indexOf(":")), base.slice(base.indexOf(":") + 1)];
  // A branch lives in one worktree at a time, so only branches nobody has checked out can open.
  const openable = branches?.branches.filter((name) => !branches.checkedOut.includes(name)) ?? [];

  const start = async () => {
    if (!ready || !harness) return;
    setBusy(true);
    setError(null);
    const projects = useProjectsStore.getState();
    const request = {
      id: harness.id,
      model: model.trim() || null,
      effort: effortChoice || null,
      prompt: message.trim() || null,
    };
    const remember = () =>
      projects.remember(picksKey(project.id), {
        harness: harness.id,
        model: model.trim(),
        effort: effortChoice,
      } satisfies LastPicks);

    // Running in a workspace that already exists is an ordinary spawn: the core labels the
    // session with the workspace and writes its record, exactly as it does for a new one.
    if (runIn) {
      try {
        const session = await ipc.ptySpawn({
          program: null,
          args: [],
          cwd: null,
          workspaceId: runIn.id,
          harness: request,
          size: useTerminalStore.getState().lastSize,
        });
        setBusy(false);
        remember();
        projects.select(runIn.id);
        useTerminalStore.getState().adopt(session);
      } catch (error) {
        setBusy(false);
        setError(error instanceof Error ? error.message : String(error));
      }
      return;
    }

    const result = await projects.createWorkspace({
      projectId: project.id,
      baseBranch: mode === "new" ? branch : null,
      existingBranch: mode === "open" ? branch : null,
      harness: request,
      size: useTerminalStore.getState().lastSize,
    });
    setBusy(false);
    if ("error" in result) return setError(result.error);
    remember();
    useTerminalStore.getState().adopt(result);
  };

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto p-6">
      <form
        aria-label={runIn ? `Run in ${runIn.name}` : "New workspace"}
        className="w-full max-w-2xl"
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <h1 className="mb-3 text-center text-ink-muted">
          {runIn ? (
            <>
              Run in <span className="font-medium text-ink">{project.name}</span>
              {runIn.head && (
                <>
                  {" on "}
                  <span className="font-mono text-[12px] text-ink">{runIn.head.label}</span>
                </>
              )}
            </>
          ) : (
            <>
              New workspace in <span className="font-medium text-ink">{project.name}</span>
            </>
          )}
        </h1>
        <textarea
          ref={messageRef}
          aria-label="What should the agent work on?"
          placeholder="What should the agent work on?"
          value={message}
          rows={5}
          disabled={busy}
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            // Enter sends, Shift+Enter breaks the line — and never while an IME is composing.
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void start();
            } else if (event.key === "Escape") {
              const projects = useProjectsStore.getState();
              if (runIn) projects.select(runIn.id);
              else projects.compose(null);
            }
          }}
          className="w-full resize-none rounded-lg border border-line bg-surface p-3 text-[14px] leading-relaxed outline-none select-text focus:border-accent disabled:opacity-60"
        />

        <div className="mt-2 flex flex-wrap items-center gap-2">
          <select
            aria-label="Harness"
            value={harness?.id ?? ""}
            disabled={busy || !harnessesLoaded}
            onChange={(event) => chooseHarness(event.target.value)}
            className={control}
          >
            {harnesses.map((h) => (
              <option key={h.id} value={h.id} disabled={!h.resolvedPath}>
                {h.label}
                {h.resolvedPath ? "" : " — not installed"}
              </option>
            ))}
          </select>

          <input
            aria-label="Model"
            placeholder="model: default"
            list={modelsId}
            value={model}
            disabled={busy}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setModel(event.target.value)}
            className={`${control} w-40 select-text`}
          />
          <datalist id={modelsId}>
            {harness?.models.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>

          {harness && harness.efforts.length > 0 && (
            <select
              aria-label="Effort"
              value={effortChoice}
              disabled={busy}
              onChange={(event) => setEffort(event.target.value)}
              className={control}
            >
              <option value="">effort: default</option>
              {harness.efforts.map((level) => (
                <option key={level} value={level}>
                  effort: {level}
                </option>
              ))}
            </select>
          )}

          {runIn ? (
            <span className="ml-auto text-[12px] text-ink-faint">no new branch or worktree</span>
          ) : (
            <select
              aria-label="Branch"
              title="Start a new branch from one of these, or open a branch that already exists"
              value={base}
              disabled={busy || !branches}
              onChange={(event) => setBase(event.target.value)}
              className={`${control} ml-auto max-w-56 font-mono text-[12px]`}
            >
              <optgroup label="New branch from">
                {branches?.branches.map((name) => (
                  <option key={name} value={`new:${name}`}>
                    {name}
                  </option>
                ))}
              </optgroup>
              {openable.length > 0 && (
                <optgroup label="Open existing branch">
                  {openable.map((name) => (
                    <option key={name} value={`open:${name}`}>
                      {name}
                    </option>
                  ))}
                </optgroup>
              )}
            </select>
          )}

          <button
            type="submit"
            disabled={!ready}
            className="h-8 rounded bg-accent px-4 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Starting…" : "Start"}
          </button>
        </div>

        {offering && !busy && (
          <p className="mt-2 flex items-center justify-end gap-2 text-[11px] text-ink-faint">
            <span>
              Assist suggests{" "}
              {suggestedHarness && <span className="text-ink-muted">{suggestedHarness.label}</span>}
              {suggestedHarness && offeredEffort ? " · " : ""}
              {offeredEffort && <span className="text-ink-muted">effort {offeredEffort}</span>}
            </span>
            <button
              type="button"
              onClick={takeSuggestion}
              className="rounded border border-line px-2 py-0.5 hover:border-accent hover:text-ink"
            >
              Use
            </button>
          </p>
        )}

        <div className="mt-3 min-h-10 text-center">
          {error ? (
            <p role="alert" className="text-red-400 select-text">
              {error}
            </p>
          ) : harnessesLoaded && installed.length === 0 ? (
            <p role="alert" className="text-red-400">
              No enabled harness was found on your PATH. Check Settings → Harnesses.
            </p>
          ) : (
            <p className="text-ink-faint">
              Enter to start · Shift+Enter for a new line · Esc to cancel · {formatShortcut("N")}{" "}
              opens this again.{" "}
              {runIn
                ? "Runs in the project's own checkout, on the branch it has out. Nothing is created."
                : mode === "open"
                  ? `Opens the existing branch "${branch}" in a new git worktree.`
                  : "A new branch and git worktree are created when you start."}
            </p>
          )}
        </div>
      </form>
    </div>
  );
}

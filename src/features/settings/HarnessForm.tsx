import { useEffect, useMemo, useState } from "react";
import {
  errorMessage,
  ipc,
  type HarnessDef,
  type HarnessInfo,
  type HarnessPreview,
} from "@/lib/ipc";
import { joinWords, splitWords } from "@/lib/shellWords";
import { useHarnessStore } from "@/stores/harnesses";
import { buttonClass, Field, inputClass, primaryButtonClass } from "./fields";
import { TestLaunch } from "./TestLaunch";

/** The argument groups, in the order they appear on the command line. */
const ARG_FIELDS = [
  ["baseArgs", "Always", "Passed on every launch."],
  ["modelArgs", "Model", "Used when a model is chosen. {model}"],
  ["effortArgs", "Effort", "Used when an effort level is chosen. {effort}"],
  ["sessionArgs", "Session id", "Used when Yardsort assigns the session id. {session_id}"],
  ["promptArgs", "Prompt", "Used when there is an opening message (argv transport). {prompt}"],
  ["resumeArgs", "Resume", "Replace the session and prompt args when resuming. {session_id}"],
  ["forkArgs", "Fork", "Replace them when forking a session. {session_id}"],
] as const;
type ArgField = (typeof ARG_FIELDS)[number][0];

const LIST_FIELDS = [
  ["efforts", "Effort levels", "Comma-separated. Leave empty to hide the effort picker."],
  ["models", "Model suggestions", "Comma-separated. Any model name can still be typed."],
] as const;

/** The form edits text; this is the text for one definition. */
function toDraft(def: HarnessDef) {
  return {
    ...def,
    args: Object.fromEntries(ARG_FIELDS.map(([key]) => [key, joinWords(def[key])])) as Record<
      ArgField,
      string
    >,
    lists: { efforts: def.efforts.join(", "), models: def.models.join(", ") },
    stdinReadyText: String(def.stdinReadyMs),
  };
}
type Draft = ReturnType<typeof toDraft>;

const commaList = (text: string) =>
  text
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);

/** The definition a draft describes, or which argument fields cannot be parsed. */
function fromDraft(draft: Draft): { def: HarnessDef } | { broken: ArgField[] } {
  const broken: ArgField[] = [];
  const parsed = {} as Record<ArgField, string[]>;
  for (const [key] of ARG_FIELDS) {
    const words = splitWords(draft.args[key]);
    if (words) parsed[key] = words;
    else broken.push(key);
  }
  if (broken.length > 0) return { broken };
  const { args: _args, lists, stdinReadyText, ...rest } = draft;
  const ready = Number.parseInt(stdinReadyText, 10);
  return {
    def: {
      ...rest,
      ...parsed,
      efforts: commaList(lists.efforts),
      models: commaList(lists.models),
      stdinReadyMs: Number.isFinite(ready) && ready >= 0 ? ready : rest.stdinReadyMs,
    },
  };
}

function stripInfo({
  resolvedPath: _r,
  builtin: _b,
  modified: _m,
  ...def
}: HarnessInfo): HarnessDef {
  return def;
}

interface Props {
  harness: HarnessInfo;
  /** A harness being added: the id is editable and nothing is saved yet. */
  isNew: boolean;
  onSaved: (id: string) => void;
  onRemoved: () => void;
}

export function HarnessForm({ harness, isNew, onSaved, onRemoved }: Props) {
  const [draft, setDraft] = useState(() => toDraft(stripInfo(harness)));
  const [preview, setPreview] = useState<HarnessPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState<HarnessDef | null>(null);

  const result = useMemo(() => fromDraft(draft), [draft]);
  const def = "def" in result ? result.def : null;
  const broken = "broken" in result ? result.broken : [];
  const dirty =
    isNew || (def !== null && JSON.stringify(def) !== JSON.stringify(stripInfo(harness)));

  // Ask the core what would actually run — the fastest way to debug a template.
  const defKey = def ? JSON.stringify(def) : null;
  useEffect(() => {
    if (!defKey) return;
    let stale = false;
    const timer = setTimeout(() => {
      ipc.harnessPreview(JSON.parse(defKey) as HarnessDef).then(
        (next) => !stale && setPreview(next),
        (reason) => !stale && setError(errorMessage(reason)),
      );
    }, 150);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [defKey]);

  const set = (patch: Partial<Draft>) => {
    setDraft((current) => ({ ...current, ...patch }));
    setError(null);
  };

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    def &&
    run(async () => {
      await useHarnessStore.getState().save(def);
      onSaved(def.id);
    });

  const reset = () =>
    run(async () => {
      await useHarnessStore.getState().reset(harness.id);
      onRemoved();
    });

  const problem = error ?? preview?.problem ?? null;

  return (
    <form
      aria-label={`${harness.label} settings`}
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div className="grid grid-cols-2 gap-3">
        <Field label="Label">
          <input
            value={draft.label}
            onChange={(e) => set({ label: e.target.value })}
            className={inputClass}
          />
        </Field>
        <Field
          label="Id"
          hint={isNew ? "Lowercase letters, digits, - and _. Cannot be changed later." : undefined}
        >
          <input
            value={draft.id}
            disabled={!isNew}
            spellCheck={false}
            onChange={(e) => set({ id: e.target.value })}
            className={`${inputClass} font-mono text-[12px]`}
          />
        </Field>
      </div>

      <Field
        label="Command"
        hint={
          preview?.resolvedPath ? (
            <>Found at {preview.resolvedPath}</>
          ) : preview ? (
            <span className="text-red-400">Not found on your PATH.</span>
          ) : undefined
        }
      >
        <input
          value={draft.command}
          spellCheck={false}
          onChange={(e) => set({ command: e.target.value })}
          className={`${inputClass} font-mono text-[12px]`}
        />
      </Field>

      <Field
        label="Good at"
        hint="Optional, and only used by Assist: when you describe two or more harnesses here, the composer can suggest which one suits the message you are typing. Yardsort never guesses this for you."
      >
        <input
          value={draft.strengths ?? ""}
          placeholder="e.g. long refactors and tricky debugging"
          onChange={(e) => set({ strengths: e.target.value })}
          className={inputClass}
        />
      </Field>

      <fieldset className="grid gap-3">
        <legend className="mb-2 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          Arguments
        </legend>
        {ARG_FIELDS.map(([key, label, hint]) => (
          <Field
            key={key}
            label={`${label} args`}
            hint={hint}
            error={broken.includes(key) ? "A quote is left open." : null}
          >
            <input
              value={draft.args[key]}
              spellCheck={false}
              onChange={(e) => set({ args: { ...draft.args, [key]: e.target.value } })}
              className={`${inputClass} font-mono text-[12px]`}
            />
          </Field>
        ))}
      </fieldset>

      <div className="grid grid-cols-2 gap-3">
        {LIST_FIELDS.map(([key, label, hint]) => (
          <Field key={key} label={label} hint={hint}>
            <input
              value={draft.lists[key]}
              spellCheck={false}
              onChange={(e) => set({ lists: { ...draft.lists, [key]: e.target.value } })}
              className={inputClass}
            />
          </Field>
        ))}
        <Field
          label="Prompt transport"
          hint={
            draft.promptTransport === "stdin"
              ? "The message is pasted into the terminal once the harness has gone quiet."
              : "The message is passed as an argument."
          }
        >
          <select
            value={draft.promptTransport}
            onChange={(e) => set({ promptTransport: e.target.value as Draft["promptTransport"] })}
            className={inputClass}
          >
            <option value="argv">argv</option>
            <option value="stdin">stdin</option>
          </select>
        </Field>
        {draft.promptTransport === "stdin" ? (
          <Field
            label="Ready after quiet for (ms)"
            hint="How long the harness must be silent first."
          >
            <input
              inputMode="numeric"
              value={draft.stdinReadyText}
              onChange={(e) => set({ stdinReadyText: e.target.value.replace(/\D/g, "") })}
              className={inputClass}
            />
          </Field>
        ) : (
          <span />
        )}
        <Field
          label="Session id"
          hint={
            draft.sessionIdMode === "assigned"
              ? "Yardsort chooses the id up front, so resume is exact."
              : "The harness chooses; resume means its latest session in the workspace folder."
          }
        >
          <select
            value={draft.sessionIdMode}
            onChange={(e) => set({ sessionIdMode: e.target.value as Draft["sessionIdMode"] })}
            className={inputClass}
          >
            <option value="assigned">assigned by Yardsort</option>
            <option value="latestInCwd">chosen by the harness</option>
          </select>
        </Field>
        <label className="flex items-center gap-2 self-end pb-2">
          <input
            type="checkbox"
            checked={draft.enabled}
            onChange={(e) => set({ enabled: e.target.checked })}
          />
          Offer this harness when starting work
        </label>
      </div>

      <section aria-label="Command preview" className="rounded border border-line bg-canvas p-3">
        <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          What will run
        </h3>
        {preview && def ? (
          <dl className="grid gap-2 font-mono text-[12px]">
            {(["start", "resume", "fork"] as const).map((kind) => (
              <div key={kind} className="grid grid-cols-[4rem_1fr] gap-2">
                <dt className="text-ink-faint">{kind}</dt>
                <dd className="break-all select-text">{joinWords(preview[kind])}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-ink-faint">Fix the arguments above to see the command line.</p>
        )}
      </section>

      {problem && (
        <p role="alert" className="text-red-400 select-text">
          {problem}
        </p>
      )}

      <div className="flex items-center gap-2">
        <button
          type="submit"
          disabled={busy || !def || !dirty || !!preview?.problem}
          className={primaryButtonClass}
        >
          {isNew ? "Add harness" : "Save"}
        </button>
        <button
          type="button"
          disabled={busy || !def}
          onClick={() => setTesting(def)}
          className={buttonClass}
        >
          Test launch
        </button>
        <span className="flex-1" />
        {!isNew && harness.builtin && (
          <button
            type="button"
            disabled={busy || !harness.modified}
            onClick={reset}
            className={buttonClass}
          >
            Restore defaults
          </button>
        )}
        {!isNew && !harness.builtin && (
          <button
            type="button"
            disabled={busy}
            onClick={reset}
            className={`${buttonClass} text-red-400`}
          >
            Delete harness
          </button>
        )}
      </div>

      {testing && <TestLaunch def={testing} onClose={() => setTesting(null)} />}
    </form>
  );
}

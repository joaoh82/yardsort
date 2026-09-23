import { useEffect, useState } from "react";
import { errorMessage, ipc, type AssistStatus, type ThresholdsDto } from "@/lib/ipc";
import { useAssistStore } from "@/stores/assist";
import { DraftSettings } from "./DraftSettings";
import { buttonClass, Field, inputClass } from "./fields";

/** The three thresholds, in the order they are shown. */
const THRESHOLDS = [
  [
    "flagAtPercent",
    "Flag a risky change at (%)",
    "How sure Jev must be that a change adds a secret, weakens a test or switches a check off before it is badged. Lower catches more and cries wolf more.",
  ],
  [
    "offTaskAtPercent",
    "Call a file off-task at (%)",
    "How much of the answer must say “unrelated to the task” before a file is badged off-task.",
  ],
  [
    "suggestAtPercent",
    "Offer a composer suggestion at (%)",
    "How sure Jev must be about a harness or a difficulty before the composer offers it.",
  ],
] as const;

type Percents = Pick<ThresholdsDto, (typeof THRESHOLDS)[number][0]>;

const percentsOf = (thresholds: ThresholdsDto): Percents => ({
  flagAtPercent: thresholds.flagAtPercent,
  offTaskAtPercent: thresholds.offTaskAtPercent,
  suggestAtPercent: thresholds.suggestAtPercent,
});

/**
 * The numbers that turn Jev's probabilities into badges and suggestions. They are settings
 * because the right values depend on the code being written and on how much noise you will put
 * up with — and because moving one costs nothing: answers already given are re-read, not re-asked.
 */
function ThresholdFields({
  status,
  busy,
  run,
}: {
  status: AssistStatus;
  busy: boolean;
  run: (what: "saving", action: () => Promise<AssistStatus | void>) => Promise<boolean>;
}) {
  // `null` means "whatever the core last told us"; editing takes a copy.
  const [draft, setDraft] = useState<Percents | null>(null);
  const saved = percentsOf(status.thresholds);
  const shown = draft ?? saved;
  const [low, high] = status.thresholds.range;
  const defaults = status.thresholds.defaults;
  const dirty = THRESHOLDS.some(([key]) => shown[key] !== saved[key]);
  const atDefaults = THRESHOLDS.every(([key], index) => shown[key] === defaults[index]);

  const save = async (percents: Percents) => {
    const done = await run("saving", () =>
      ipc.assistSaveSettings(status.reviewChanges, status.suggestInComposer, {
        ...status.thresholds,
        ...percents,
      }),
    );
    if (done) {
      setDraft(null);
      // Re-reads the answers already given against the new numbers; no requests to TypeSafe.
      void useAssistStore.getState().reviewNow();
    }
  };

  return (
    <fieldset className="grid gap-3">
      <legend className="mb-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
        How sure Jev must be
      </legend>
      <p className="text-[11px] text-ink-faint">
        Changing these re-reads the answers Jev has already given — nothing is sent again. The
        defaults are a starting point, not a measurement; tune them against your own work.
      </p>
      {THRESHOLDS.map(([key, label, hint]) => (
        <Field key={key} label={label} hint={hint}>
          <input
            type="number"
            min={low}
            max={high}
            step={5}
            value={shown[key]}
            disabled={busy}
            onChange={(event) =>
              setDraft({ ...shown, [key]: Number.parseInt(event.target.value, 10) || 0 })
            }
            className={`${inputClass} w-24 font-mono text-[12px]`}
          />
        </Field>
      ))}
      <div className="flex items-center gap-3">
        <button
          type="button"
          disabled={!dirty || busy}
          onClick={() => void save(shown)}
          className={buttonClass}
        >
          Save thresholds
        </button>
        <button
          type="button"
          disabled={atDefaults || busy}
          onClick={() =>
            void save({
              flagAtPercent: defaults[0],
              offTaskAtPercent: defaults[1],
              suggestAtPercent: defaults[2],
            })
          }
          className={buttonClass}
        >
          Restore defaults
        </button>
      </div>
    </fieldset>
  );
}

/**
 * Assist: an API key for TypeSafe's Jev model, and a switch per feature. Everything here is off
 * until a key is entered and a box is ticked, because every feature sends something off the
 * machine — so each one says what.
 */
export function AssistSettings() {
  const status = useAssistStore((s) => s.status);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<null | "saving" | "testing" | "forgetting">(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => void useAssistStore.getState().load(), []);

  if (!status) return <p className="p-5 text-ink-faint">Loading…</p>;

  const run = async (
    what: NonNullable<typeof busy>,
    action: () => Promise<AssistStatus | void>,
  ) => {
    setBusy(what);
    setError(null);
    setNote(null);
    try {
      const next = await action();
      if (next) useAssistStore.getState().adopt(next);
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const saveKey = async (event: React.FormEvent) => {
    event.preventDefault();
    const saved = await run("saving", () => ipc.assistSaveKey(key));
    if (saved) {
      setKey("");
      setNote("Key saved.");
    }
  };

  const toggle = (patch: Partial<Pick<AssistStatus, "reviewChanges" | "suggestInComposer">>) =>
    void run("saving", () =>
      ipc.assistSaveSettings(
        patch.reviewChanges ?? status.reviewChanges,
        patch.suggestInComposer ?? status.suggestInComposer,
        status.thresholds,
      ),
    );

  const hasKey = status.keySource !== "none";

  return (
    <div aria-label="Assist settings" className="grid max-w-2xl gap-5 p-5">
      <p className="text-ink-muted">
        Assist asks{" "}
        <a
          href="https://docs.typesafe.ai"
          target="_blank"
          rel="noreferrer"
          className="text-accent hover:underline"
        >
          TypeSafe&rsquo;s Jev model
        </a>{" "}
        small, typed questions — is this file part of what you asked for, does this change switch a
        check off — and turns the answers into badges and hints.{" "}
        <span className="text-ink-faint">
          It is billed to your own TypeSafe account, and Yardsort uses {status.model}.
        </span>
      </p>

      <form onSubmit={saveKey} className="grid gap-2">
        <Field
          label="TypeSafe API key"
          hint={
            status.keySource === "environment" ? (
              <>
                In use from <code>TYPESAFE_API_KEY</code> in your environment ({status.keyHint}).
                Saving a key here replaces it.
              </>
            ) : hasKey ? (
              <>
                Saved in your system credential store ({status.keyHint}). It is never shown again.
              </>
            ) : (
              <>
                Kept in your system credential store — the Keychain, Credential Manager or Secret
                Service — never in the settings file. Get one from console.typesafe.ai.
              </>
            )
          }
          trailing={
            <>
              <button type="submit" disabled={!key.trim() || busy !== null} className={buttonClass}>
                {busy === "saving" ? "Checking…" : "Save"}
              </button>
              {hasKey && (
                <>
                  <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() =>
                      void run("testing", async () => {
                        await ipc.assistTestKey();
                        setNote("The key works.");
                      })
                    }
                    className={buttonClass}
                  >
                    Test
                  </button>
                  <button
                    type="button"
                    disabled={busy !== null || status.keySource === "environment"}
                    title={
                      status.keySource === "environment"
                        ? "This key comes from your environment; unset TYPESAFE_API_KEY to remove it."
                        : undefined
                    }
                    onClick={() => void run("forgetting", () => ipc.assistForgetKey())}
                    className={buttonClass}
                  >
                    Forget
                  </button>
                </>
              )}
            </>
          }
        >
          <input
            type="password"
            value={key}
            placeholder={hasKey ? "replace the key…" : "paste your key"}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setKey(event.target.value)}
            className={`${inputClass} font-mono text-[12px]`}
          />
        </Field>
      </form>

      <fieldset className="grid gap-3">
        <legend className="mb-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          What Assist may do
        </legend>
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={status.reviewChanges}
            disabled={!hasKey || busy !== null}
            onChange={(event) => toggle({ reviewChanges: event.target.checked })}
            className="mt-0.5 accent-(--color-accent)"
          />
          <span>
            Check changed files against what the workspace was asked to do
            <span className="block text-ink-faint">
              Flags files that look unrelated to the task, and changes that add a secret, weaken a
              test or switch a check off. <strong>Sends the diff of each changed file</strong> and
              the first message of the workspace&rsquo;s conversations to TypeSafe. Files whose name
              says they hold credentials (<code>.env</code>, <code>*.pem</code>) are flagged without
              being sent.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={status.suggestInComposer}
            disabled={!hasKey || busy !== null}
            onChange={(event) => toggle({ suggestInComposer: event.target.checked })}
            className="mt-0.5 accent-(--color-accent)"
          />
          <span>
            Suggest a harness and an effort in the composer
            <span className="block text-ink-faint">
              <strong>Sends the message you are typing</strong> and the &ldquo;Good at&rdquo;
              descriptions from Settings → Harnesses. Suggestions are only ever offered; nothing is
              picked for you.
            </span>
          </span>
        </label>
      </fieldset>

      <ThresholdFields status={status} busy={busy !== null} run={run} />

      <DraftSettings />

      {status.problem && (
        <p role="alert" className="text-amber-400 select-text">
          {status.problem}
        </p>
      )}
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      {note && <p className="text-ink-faint">{note}</p>}
      {!hasKey && (
        <p className="text-ink-faint">
          Without a key nothing is sent and nothing changes: Yardsort works exactly as it does
          today.
        </p>
      )}
      <p className="text-ink-faint">
        <span className="font-medium text-ink-muted">What Assist never does:</span> it does not read
        your terminals, and it never decides anything on its own — every answer becomes a badge or a
        suggestion you can ignore.
      </p>
    </div>
  );
}

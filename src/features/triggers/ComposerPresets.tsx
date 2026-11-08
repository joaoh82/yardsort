import { useEffect, useMemo, useState } from "react";
import {
  hasCore,
  ipc,
  type HarnessInfo,
  type PresetPick,
  type TestItem,
  type TriggerPreset,
} from "@/lib/ipc";
import { useHarnessStore } from "@/stores/harnesses";
import { useTriggersStore } from "@/stores/triggers";

const control =
  "h-7 rounded border border-line bg-canvas px-1.5 text-[12px] text-ink outline-none focus:border-accent disabled:opacity-50";

/** The presets ticked, by id, with the answer to each one's question: `""` for one that asks
 *  nothing. What the composer remembers per project. */
export type Picks = Record<string, string>;

/** What the presets' questions can be answered with in a project. */
export interface Choices {
  presets: TriggerPreset[];
  /** Whether the presets have been read: until then, a remembered tick has no preset yet. */
  presetsLoaded: boolean;
  /** The agents that can review: enabled and installed. */
  agents: HarnessInfo[];
  /** The project's runnable smoke tests, or `null` while they are being read. */
  tests: TestItem[] | null;
}

/** The presets, and what their questions can be answered with in this project. */
export function usePresetChoices(projectId: string, wanted: boolean): Choices {
  const presets = useTriggersStore((s) => s.presets);
  const presetsLoaded = useTriggersStore((s) => s.presetsLoaded);
  const harnesses = useHarnessStore((s) => s.harnesses);
  const agents = useMemo(() => harnesses.filter((h) => h.enabled && h.resolvedPath), [harnesses]);
  const [tests, setTests] = useState<{ project: string; items: TestItem[] } | null>(null);
  useEffect(() => {
    if (!wanted) return;
    void useTriggersStore.getState().loadPresets();
  }, [wanted]);
  useEffect(() => {
    if (!wanted || !hasCore()) return;
    let current = true;
    ipc.testListInProject(projectId).then(
      (found) => current && setTests({ project: projectId, items: found }),
      () => current && setTests({ project: projectId, items: [] }),
    );
    return () => {
      current = false;
    };
  }, [projectId, wanted]);
  const runnable = useMemo(
    () =>
      tests?.project === projectId
        ? tests.items.filter((test) => test.runnable && !test.browserMissing)
        : null,
    [tests, projectId],
  );
  return { presets, presetsLoaded, agents, tests: runnable };
}

/** The answers a preset's question can have here. */
function options(preset: TriggerPreset, choices: Choices): { id: string; label: string }[] {
  if (!preset.asks) return [];
  if (preset.asks.input === "suite") {
    return (choices.tests ?? []).map((test) => ({ id: test.id, label: test.name }));
  }
  return choices.agents.map((agent) => ({ id: agent.id, label: agent.label }));
}

/** The answer a ticked preset goes with: the one picked if it can still be given, else the
 *  first there is — what the control shows is what is sent. `null` when there is none. */
function answer(preset: TriggerPreset, picked: string, choices: Choices): string | null {
  if (!preset.asks) return "";
  const can = options(preset, choices);
  return (can.find((option) => option.id === picked) ?? can[0])?.id ?? null;
}

/** Whether a ticked preset cannot be answered yet: the presets, or the project's smoke tests a
 *  preset asks about, are still being read. Starting then would leave that preset out, so the
 *  composer waits. A question with nothing to answer it once they are read is another matter. */
export function pending(picks: Picks, choices: Choices): boolean {
  if (Object.keys(picks).length === 0) return false;
  if (!choices.presetsLoaded) return true;
  return choices.presets.some(
    (preset) => preset.id in picks && preset.asks?.input === "suite" && choices.tests === null,
  );
}

/** The ticked presets as the core takes them, leaving out one whose question has no answer. */
export function answered(picks: Picks, choices: Choices): PresetPick[] {
  return choices.presets.flatMap((preset) => {
    if (!(preset.id in picks)) return [];
    const given = answer(preset, picks[preset.id] ?? "", choices);
    return given === null ? [] : [{ id: preset.id, answer: given || null }];
  });
}

/**
 * The composer's presets: ready-made triggers to put on the new workspace, each with its one
 * question beside it — _☐ Review when the work is done, by [Codex ▾]_. Ticked as they were last
 * time in the project.
 */
export function ComposerPresets({
  choices,
  picks,
  onChange,
  disabled,
}: {
  choices: Choices;
  picks: Picks;
  onChange: (picks: Picks) => void;
  disabled: boolean;
}) {
  if (choices.presets.length === 0) return null;
  return (
    <fieldset className="mt-2 text-[12px] text-ink-faint">
      <legend className="mb-1">When something happens in the new workspace</legend>
      <ul className="flex flex-col gap-1">
        {choices.presets.map((preset) => {
          const ticked = preset.id in picks;
          const can = options(preset, choices);
          // A question nobody can answer here: no smoke tests, no agent to review.
          const none = !!preset.asks && can.length === 0 && choices.tests !== null;
          const given = answer(preset, picks[preset.id] ?? "", choices) ?? "";
          return (
            <li key={preset.id} className="flex items-center gap-2" title={preset.does}>
              <label className={`flex items-center gap-2 ${none ? "opacity-50" : ""}`}>
                <input
                  type="checkbox"
                  checked={ticked && !none}
                  disabled={disabled || none}
                  onChange={(event) => {
                    const next = { ...picks };
                    if (event.target.checked) next[preset.id] = given;
                    else delete next[preset.id];
                    onChange(next);
                  }}
                  className="accent-(--color-accent)"
                />
                <span className={ticked ? "text-ink-muted" : ""}>{preset.name}</span>
              </label>
              {preset.asks && !none && (
                <select
                  aria-label={`${preset.name}: ${preset.asks.question}`}
                  className={control}
                  value={given}
                  disabled={disabled || (preset.asks.input === "suite" && choices.tests === null)}
                  onChange={(event) => onChange({ ...picks, [preset.id]: event.target.value })}
                >
                  {can.map((option) => (
                    <option key={option.id} value={option.id}>
                      {preset.asks?.input === "suite" ? option.label : `by ${option.label}`}
                    </option>
                  ))}
                </select>
              )}
              {ticked && preset.asks?.input === "suite" && choices.tests === null && (
                <span>— reading the smoke tests…</span>
              )}
              {none && (
                <span>
                  {preset.asks?.input === "suite" ? "— no smoke tests here" : "— no agent to ask"}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}

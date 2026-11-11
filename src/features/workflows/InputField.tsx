import { HarnessIcon } from "@/features/harness/HarnessIcon";
import type { TestItem } from "@/lib/ipc";
import { useHarnessStore } from "@/stores/harnesses";

const control =
  "h-9 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-small font-semibold tracking-wider text-ink-faint uppercase";

/** What a field needs of an input: a workflow's, or a smoke test's, which has fewer kinds. */
export interface FieldInput {
  id: string;
  /** `harness`, `test`, `choice` or `text`. */
  kind: string;
  label: string;
  required: boolean;
  options: string[];
}

interface Props {
  input: FieldInput;
  value: string;
  onChange: (value: string) => void;
  /** For a `test` input: the runnable smoke tests of the chosen workspace, or `null` while they
   *  are being read. */
  tests?: TestItem[] | null;
  /** Whether a workspace is chosen yet, so a `test` input can say it is loading. */
  hasWorkspace?: boolean;
}

/**
 * One input of a run, asked the way its kind is answered: an agent from the ones set up, a
 * choice from its options, a smoke test from the workspace's, or text. The Run dialog and the
 * schedule dialog ask the same questions, so they ask them with this.
 */
export function InputField({ input, value, onChange, tests = null, hasWorkspace = false }: Props) {
  const allHarnesses = useHarnessStore((s) => s.harnesses);
  const harnesses = allHarnesses.filter((h) => h.enabled);
  const testsLoaded = tests !== null;
  const runnableTests = tests ?? [];
  return (
    <label className="flex flex-col gap-1.5">
      <span className={label}>
        {input.label}
        {input.required ? "" : " (optional)"}
      </span>
      {input.kind === "harness" ? (
        <span className="relative flex items-center">
          <HarnessIcon id={value} className="pointer-events-none absolute left-2" />
          <select
            aria-label={input.label}
            className={`${control} pl-7`}
            value={value}
            onChange={(event) => onChange(event.target.value)}
          >
            {!input.required && <option value="">None</option>}
            {harnesses.map((h) => (
              <option key={h.id} value={h.id} disabled={!h.resolvedPath}>
                {h.label}
                {h.resolvedPath ? "" : " — not installed"}
              </option>
            ))}
          </select>
        </span>
      ) : input.kind === "choice" ? (
        <select
          aria-label={input.label}
          className={control}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        >
          {!input.required && <option value="">None</option>}
          {input.options.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      ) : input.kind === "test" ? (
        <select
          aria-label={input.label}
          className={control}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={!testsLoaded || runnableTests.length === 0}
        >
          {!input.required && <option value="">None</option>}
          {hasWorkspace && !testsLoaded && <option value="">Loading…</option>}
          {testsLoaded && runnableTests.length === 0 && (
            <option value="">No runnable smoke tests here</option>
          )}
          {runnableTests.map((test) => (
            <option key={test.id} value={test.id}>
              {test.name}
            </option>
          ))}
        </select>
      ) : (
        <input
          aria-label={input.label}
          className="rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
          value={value}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </label>
  );
}

import { useEffect, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import type { HarnessInfo } from "@/lib/ipc";
import { useHarnessStore } from "@/stores/harnesses";
import { buttonClass } from "./fields";
import { HarnessForm } from "./HarnessForm";

/** A blank harness for the "add" form. It becomes real only when saved. */
const blank: HarnessInfo = {
  id: "",
  label: "",
  command: "",
  baseArgs: [],
  modelArgs: [],
  effortArgs: [],
  sessionArgs: [],
  promptArgs: ["{prompt}"],
  resumeArgs: [],
  forkArgs: [],
  efforts: [],
  models: [],
  promptTransport: "argv",
  sessionIdMode: "latestInCwd",
  stdinReadyMs: 1500,
  enabled: true,
  resolvedPath: null,
  builtin: false,
  modified: false,
};

const NEW = "\0new";

export function HarnessSettings() {
  const harnesses = useHarnessStore((s) => s.harnesses);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Commands may have been installed since the list was last resolved.
  useEffect(() => void useHarnessStore.getState().reload(), []);

  const adding = selectedId === NEW;
  const selected = adding ? blank : (harnesses.find((h) => h.id === selectedId) ?? harnesses[0]);

  return (
    <div className="flex h-full min-h-0">
      <nav aria-label="Harnesses" className="flex w-52 shrink-0 flex-col border-r border-line">
        <ul className="min-h-0 flex-1 overflow-y-auto py-1">
          {harnesses.map((harness) => (
            <li key={harness.id}>
              <button
                type="button"
                aria-current={!adding && selected?.id === harness.id}
                onClick={() => setSelectedId(harness.id)}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-raised aria-[current=true]:bg-raised"
              >
                <span
                  aria-hidden
                  title={harness.resolvedPath ? "Installed" : "Not found on PATH"}
                  className={`size-1.5 shrink-0 rounded-full ${
                    !harness.resolvedPath ? "bg-red-400" : harness.enabled ? "bg-accent" : "bg-line"
                  }`}
                />
                <HarnessIcon
                  id={harness.id}
                  label={harness.label}
                  className={harness.enabled ? "" : "opacity-40"}
                />
                <span
                  className={`truncate ${harness.enabled ? "" : "text-ink-faint line-through"}`}
                >
                  {harness.label}
                </span>
                <span className="ml-auto text-[10px] text-ink-faint">
                  {!harness.builtin ? "custom" : harness.modified ? "modified" : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <div className="border-t border-line p-2">
          <button
            type="button"
            onClick={() => setSelectedId(NEW)}
            className={`${buttonClass} w-full`}
          >
            Add custom harness
          </button>
        </div>
      </nav>

      <div className="min-w-0 flex-1 overflow-y-auto p-5">
        {selected && (
          <HarnessForm
            // A fresh form per harness — and per saved revision, so it restarts from what was saved.
            key={adding ? NEW : `${selected.id}:${JSON.stringify(selected)}`}
            harness={selected}
            isNew={adding}
            onSaved={setSelectedId}
            onRemoved={() => setSelectedId(null)}
          />
        )}
      </div>
    </div>
  );
}

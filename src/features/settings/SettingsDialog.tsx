import { useEffect, useRef, useState } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { buttonClass } from "./fields";
import { AssistSettings } from "./AssistSettings";
import { GeneralSettings } from "./GeneralSettings";
import { HarnessSettings } from "./HarnessSettings";
import { WorkspaceSettings } from "./WorkspaceSettings";

const SECTIONS = [
  ["harnesses", "Harnesses"],
  ["workspaces", "Workspaces"],
  ["assist", "Assist"],
  ["general", "General"],
] as const;
type Section = (typeof SECTIONS)[number][0];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [section, setSection] = useState<Section>("harnesses");
  const dialogRef = useRef<HTMLDivElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // A dialog on top (test launch) closes first; its terminal also needs Escape.
      if (event.key === "Escape" && document.querySelectorAll('[role="dialog"]').length === 1) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-6">
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        className="flex h-full max-h-[46rem] w-full max-w-5xl flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50 outline-none"
      >
        <header className="flex h-11 shrink-0 items-center gap-1 border-b border-line px-3">
          <h2 className="mr-4 font-semibold">Settings</h2>
          <div role="tablist" className="flex gap-1">
            {SECTIONS.map(([id, label]) => (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={section === id}
                onClick={() => setSection(id)}
                className="rounded px-3 py-1 text-ink-muted hover:text-ink aria-selected:bg-raised aria-selected:text-ink"
              >
                {label}
              </button>
            ))}
          </div>
          <span className="flex-1" />
          <button type="button" onClick={onClose} className={buttonClass}>
            Done
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {section === "harnesses" ? (
            <HarnessSettings />
          ) : section === "workspaces" ? (
            <WorkspaceSettings />
          ) : section === "assist" ? (
            <AssistSettings />
          ) : (
            <GeneralSettings />
          )}
        </div>
      </div>
    </div>
  );
}

import { useState } from "react";
import {
  bindingError,
  bindingLabel,
  COMMANDS,
  DEFAULT_BINDINGS,
  eventBinding,
  validBinding,
  type CommandId,
} from "@/lib/shortcuts";
import { usePreferencesStore } from "@/stores/preferences";
import { buttonClass, primaryButtonClass } from "./fields";

export function KeyboardSettings() {
  const loaded = usePreferencesStore((s) => s.loaded);
  const error = usePreferencesStore((s) => s.error);
  if (!loaded)
    return (
      <div className="space-y-3 p-5">
        <p role="status">{error ?? "Loading saved shortcuts…"}</p>
        {error && (
          <button
            type="button"
            className={buttonClass}
            onClick={() => void usePreferencesStore.getState().load()}
          >
            Try again
          </button>
        )}
      </div>
    );
  return <BindingEditor />;
}

function BindingEditor() {
  const stored = usePreferencesStore((s) => s.bindings);
  const notice = usePreferencesStore((s) => s.bindingNotice);
  const loaded = usePreferencesStore((s) => s.loaded);
  const saving = usePreferencesStore((s) => s.saving);
  const error = usePreferencesStore((s) => s.error);
  const [draft, setDraft] = useState(() => ({ ...stored }));
  const [recording, setRecording] = useState<CommandId | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const problem = bindingError(draft);
  const dirty = notice !== null || JSON.stringify(draft) !== JSON.stringify(stored);
  return (
    <section aria-label="Keyboard shortcuts" className="flex h-full flex-col gap-3 p-5">
      <div>
        <h2 className="text-lg font-semibold">Keyboard shortcuts</h2>
        <p className="mt-1 text-ink-muted">
          Mod is ⌘ on macOS, Ctrl+Shift on Linux and Windows. Plain Ctrl keys belong to your
          terminal.
        </p>
        <p className="mt-1 text-ink-faint">
          Choose a binding, then press Mod with a letter, arrow, comma or slash. Alt is optional.
          Escape cancels recording. Copy, paste and save are reserved.
        </p>
      </div>
      {notice && (
        <p role="status" className="text-accent">
          {notice}
        </p>
      )}
      <input
        aria-label="Find shortcut"
        placeholder="Find a command…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        className="w-full rounded border border-line bg-canvas px-3 py-2"
      />
      <div className="min-h-0 flex-1 divide-y divide-line overflow-y-auto rounded border border-line">
        {COMMANDS.filter((c) => c.label.toLowerCase().includes(query.toLowerCase())).map(
          (command) => (
            <div key={command.id} className="flex items-center gap-3 px-3 py-2">
              <span className="flex-1">{command.label}</span>
              <button
                type="button"
                className={`${buttonClass} min-w-36 font-mono`}
                disabled={saving || !loaded}
                aria-label={`Binding for ${command.label}`}
                aria-pressed={recording === command.id}
                onClick={() => {
                  setRecording(command.id);
                  setMessage(null);
                }}
                onBlur={() => {
                  if (recording === command.id) setRecording(null);
                }}
                onKeyDown={(event) => {
                  if (recording !== command.id) return;
                  if (event.key === "Tab") {
                    setRecording(null);
                    return;
                  }
                  event.preventDefault();
                  event.stopPropagation();
                  if (event.key === "Escape") {
                    setRecording(null);
                    return;
                  }
                  if (["Control", "Shift", "Meta", "Alt"].includes(event.key)) return;
                  const key = eventBinding(event.nativeEvent);
                  if (!key || !validBinding(key)) {
                    setMessage(
                      "Use Mod with a letter, arrow, comma or slash. C, V and S are reserved.",
                    );
                    return;
                  }
                  setDraft({ ...draft, [command.id]: key });
                  setRecording(null);
                  setMessage(null);
                }}
              >
                {recording === command.id ? "Press shortcut…" : bindingLabel(draft[command.id])}
              </button>
              <button
                type="button"
                className={buttonClass}
                disabled={saving || !loaded || !draft[command.id]}
                aria-label={`Clear ${command.label}`}
                onClick={() => setDraft({ ...draft, [command.id]: null })}
              >
                Clear
              </button>
            </div>
          ),
        )}
      </div>
      <p className="text-ink-muted">
        Tab / Shift+Tab moves between controls. Enter or Space activates a button. Escape closes
        dialogs. The command palette supports ↑ / ↓ and Enter. In the composer and agent terminals,
        Shift+Enter adds a line.
      </p>
      {(problem || error) && (
        <p role="alert" className="text-red-400">
          {problem ?? error}
        </p>
      )}
      {message && <p role="status">{message}</p>}
      {!loaded && <p role="status">{error ?? "Loading saved shortcuts…"}</p>}
      <div className="flex gap-3">
        <button
          type="button"
          className={primaryButtonClass}
          disabled={!loaded || saving || !dirty || !!problem}
          onClick={async () => {
            if (await usePreferencesStore.getState().saveBindings(draft))
              setMessage("Shortcuts saved.");
          }}
        >
          Save shortcuts
        </button>
        <button
          type="button"
          className={buttonClass}
          disabled={!loaded || saving}
          onClick={() => {
            setDraft({ ...DEFAULT_BINDINGS });
            setMessage("Default bindings restored. Save to apply.");
          }}
        >
          Restore defaults
        </button>
      </div>
    </section>
  );
}

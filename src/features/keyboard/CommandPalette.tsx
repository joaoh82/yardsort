import { useRef, useState } from "react";
import { COMMANDS, bindingLabel } from "@/lib/shortcuts";
import { useModalFocus } from "@/lib/useModalFocus";
import { useLayoutStore } from "@/stores/layout";
import { usePreferencesStore } from "@/stores/preferences";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import { commandEnabled, runCommand, selectWorkspace } from "./commands";

export function CommandPalette() {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(ref);
  const projects = useProjectsStore((s) => s.projects);
  useTerminalStore((s) => s.tabs);
  const bindings = usePreferencesStore((s) => s.bindings);
  const close = () => useLayoutStore.getState().setPaletteOpen(false);
  const items = [
    ...COMMANDS.filter((c) => c.id !== "palette").map((c) => ({
      id: c.id,
      label: c.label,
      detail: bindingLabel(bindings[c.id]),
      disabled: !commandEnabled(c.id),
      run: () => runCommand(c.id),
    })),
    ...projects
      .filter((p) => !p.missing)
      .flatMap((p) =>
        p.workspaces
          .filter((w) => !w.archived && !w.missing)
          .map((w) => ({
            id: w.id,
            label: `${p.name} / ${w.name}`,
            detail: "Workspace",
            disabled: false,
            run: () => selectWorkspace(w.id),
          })),
      ),
  ].filter((item) => item.label.toLowerCase().includes(query.toLowerCase()));
  const active = Math.min(index, Math.max(0, items.length - 1));
  const choose = (i: number) => {
    const item = items[i];
    if (!item || item.disabled) return;
    close();
    // Let the palette restore focus before a command opens another dialog or moves focus.
    requestAnimationFrame(item.run);
  };
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 px-6 pt-[12vh]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label="Commands and workspaces"
        tabIndex={-1}
        className="w-full max-w-xl overflow-hidden rounded-xl border border-line bg-surface shadow-2xl"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            close();
          }
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            const next =
              (active + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
              Math.max(1, items.length);
            setIndex(next);
            document.getElementById(`command-${next}`)?.scrollIntoView?.({ block: "nearest" });
          }
          if (e.key === "Enter") {
            e.preventDefault();
            choose(active);
          }
        }}
      >
        <input
          autoFocus
          role="combobox"
          aria-label="Search commands and workspaces"
          aria-expanded="true"
          aria-controls="command-results"
          aria-activedescendant={items.length ? `command-${active}` : undefined}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          placeholder="Search commands and workspaces…"
          className="w-full border-b border-line bg-transparent p-4 text-base outline-none"
        />
        <ul
          id="command-results"
          role="listbox"
          aria-label="Results"
          className="max-h-[50vh] overflow-y-auto p-2"
        >
          {items.map((item, i) => (
            <li
              id={`command-${i}`}
              key={item.id}
              role="option"
              aria-selected={i === active}
              aria-disabled={item.disabled}
              onMouseMove={() => setIndex(i)}
              onClick={() => choose(i)}
              className={`flex cursor-pointer items-center justify-between gap-3 rounded px-3 py-2 ${i === active ? "bg-raised text-accent" : "text-ink-muted"} ${item.disabled ? "opacity-40" : ""}`}
            >
              <span>{item.label}</span>
              <kbd className="shrink-0 text-[11px]">{item.detail}</kbd>
            </li>
          ))}
          {!items.length && (
            <li className="p-4 text-ink-faint">No matching commands or workspaces.</li>
          )}
        </ul>
        <p className="border-t border-line px-4 py-2 text-[11px] text-ink-faint">
          ↑ ↓ to navigate · Enter to select · Esc to close
        </p>
      </div>
    </div>
  );
}

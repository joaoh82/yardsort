import { useRef, useState } from "react";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import type { Project } from "@/lib/ipc";
import { REQUESTED_BY, VERDICTS, type Filters } from "./filters";

const control =
  "h-7 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent";
const picker = `${control} max-w-48`;

/**
 * Project, workspace, and — on the Tests tab — the last verdict and whether a test needs
 * something the machine lacks; on the Runs tab, who asked. Plus a search. The two tabs sit in
 * the view's header.
 */
export function TestsFilters({
  filters,
  search,
  projects,
  workspaces,
  active,
  onChange,
  onSearch,
  onClear,
}: {
  filters: Filters;
  search: string;
  projects: Project[];
  /** Every workspace that has a test or a run, for the workspace picker. */
  workspaces: { id: string; name: string }[];
  active: boolean;
  onChange: (next: Partial<Filters>) => void;
  onSearch: (text: string) => void;
  onClear: () => void;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const projectButton = useRef<HTMLButtonElement>(null);
  const chosen = projects.filter((project) => filters.projects.includes(project.id));
  const projectLabel =
    chosen.length === 0
      ? "All projects"
      : chosen.length === 1
        ? chosen[0]!.name
        : `${chosen.length} projects`;
  // A remembered workspace nothing in the list matches is still shown, so it can be undone.
  const strayWorkspace =
    filters.workspace !== null && !workspaces.some((w) => w.id === filters.workspace)
      ? filters.workspace
      : null;

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-3 py-2">
      <button
        ref={projectButton}
        type="button"
        aria-label={`Projects: ${projectLabel}`}
        aria-haspopup="menu"
        aria-expanded={!!menu}
        onClick={(event) => {
          const box = event.currentTarget.getBoundingClientRect();
          setMenu(menu ? null : { x: box.left, y: box.bottom + 4 });
        }}
        className={`${picker} truncate text-left`}
      >
        {projectLabel} <span aria-hidden>▾</span>
      </button>
      {menu && (
        <ContextMenu
          at={menu}
          onClose={() => {
            setMenu(null);
            projectButton.current?.focus();
          }}
          items={projects.map((project) => {
            const on = filters.projects.includes(project.id);
            return {
              label: project.name,
              checked: on,
              onSelect: () =>
                onChange({
                  projects: on
                    ? filters.projects.filter((id) => id !== project.id)
                    : [...filters.projects, project.id],
                }),
            };
          })}
        />
      )}

      <select
        aria-label="Workspace"
        value={filters.workspace ?? ""}
        onChange={(event) => onChange({ workspace: event.target.value || null })}
        className={picker}
      >
        <option value="">Any workspace</option>
        {strayWorkspace && <option value={strayWorkspace}>{strayWorkspace}</option>}
        {workspaces.map((w) => (
          <option key={w.id} value={w.id}>
            {w.name}
          </option>
        ))}
      </select>

      {filters.tab === "tests" ? (
        <>
          <select
            aria-label="Verdict"
            value={filters.verdict ?? ""}
            onChange={(event) =>
              onChange({ verdict: (event.target.value || null) as Filters["verdict"] })
            }
            className={picker}
          >
            <option value="">Any verdict</option>
            {VERDICTS.map((verdict) => (
              <option key={verdict.id} value={verdict.id}>
                {verdict.label}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1.5 text-ink-muted">
            <input
              type="checkbox"
              checked={filters.needs}
              onChange={(event) => onChange({ needs: event.target.checked })}
              className="accent-accent"
            />
            Needs attention
          </label>
        </>
      ) : (
        <select
          aria-label="Who asked"
          value={filters.requestedBy ?? ""}
          onChange={(event) => onChange({ requestedBy: event.target.value || null })}
          className={picker}
        >
          <option value="">Anyone</option>
          {REQUESTED_BY.map((who) => (
            <option key={who.id} value={who.id}>
              {who.label}
            </option>
          ))}
        </select>
      )}

      <input
        type="search"
        aria-label="Search"
        placeholder="Search…"
        value={search}
        onChange={(event) => onSearch(event.target.value)}
        className={`${control} min-w-32 flex-1`}
      />
      {active && (
        <button
          type="button"
          onClick={onClear}
          className="rounded px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink"
        >
          Clear filters
        </button>
      )}
    </div>
  );
}

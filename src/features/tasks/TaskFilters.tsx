import { useRef, useState } from "react";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import type { Project } from "@/lib/ipc";
import { ME, NOBODY, type Filters } from "./filters";

const control =
  "h-7 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent";
/** A picker is as wide as what is picked, up to a point; the search takes what is left. */
const picker = `${control} max-w-44`;

/**
 * Project, label, assignee, author, whether an answer is owed, and search. The state tabs sit
 * in the view's header.
 *
 * Everything here narrows the rows that are *loaded*. That is why the view says when the source
 * has more than the list holds, rather than letting a search come up empty without a word.
 */
export function TaskFilters({
  filters,
  search,
  projects,
  labels,
  assignees,
  authors,
  viewer,
  active,
  onChange,
  onSearch,
  onClear,
}: {
  filters: Filters;
  search: string;
  projects: Project[];
  labels: string[];
  assignees: string[];
  authors: string[];
  /** Who `gh` is logged in as, or `null` while nobody has said. */
  viewer: string | null;
  /** Something is narrowing the list. */
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
  // A remembered choice nothing in the list matches is still shown, so it can be seen and undone.
  const unlisted = (value: string | null, known: string[]) =>
    value !== null && value !== ME && value !== NOBODY && !known.includes(value) ? value : null;
  const strayLabel = unlisted(filters.label, labels);
  const strayAssignee = unlisted(filters.assignee, assignees);
  const strayAuthor = unlisted(filters.author, authors);

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
        aria-label="Label"
        value={filters.label ?? ""}
        onChange={(event) => onChange({ label: event.target.value || null })}
        className={picker}
      >
        <option value="">Any label</option>
        {strayLabel && <option value={strayLabel}>{strayLabel}</option>}
        {labels.map((name) => (
          <option key={name} value={name}>
            {name}
          </option>
        ))}
      </select>

      <select
        aria-label="Assignee"
        value={filters.assignee ?? ""}
        onChange={(event) => onChange({ assignee: event.target.value || null })}
        className={picker}
      >
        <option value="">Any assignee</option>
        <option value={NOBODY}>Assigned to no one</option>
        {viewer && <option value={ME}>Me ({viewer})</option>}
        {strayAssignee && <option value={strayAssignee}>{strayAssignee}</option>}
        {assignees.map((login) => (
          <option key={login} value={login}>
            {login}
          </option>
        ))}
      </select>

      <select
        aria-label="Author"
        value={filters.author ?? ""}
        onChange={(event) => onChange({ author: event.target.value || null })}
        className={picker}
      >
        <option value="">Any author</option>
        {viewer && <option value={ME}>Me ({viewer})</option>}
        {strayAuthor && <option value={strayAuthor}>{strayAuthor}</option>}
        {authors.map((login) => (
          <option key={login} value={login}>
            {login}
          </option>
        ))}
      </select>

      <label
        title="Open, and the last person to speak was not an owner, a member or a collaborator"
        className="flex h-7 items-center gap-1.5 whitespace-nowrap text-ink-muted"
      >
        <input
          type="checkbox"
          checked={filters.needsAnswer}
          onChange={(event) => onChange({ needsAnswer: event.target.checked })}
          className="accent-accent"
        />
        Needs an answer
      </label>

      <input
        type="search"
        aria-label="Search tasks"
        placeholder="Search title or number…"
        value={search}
        onChange={(event) => onSearch(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && search !== "") {
            event.stopPropagation();
            onSearch("");
          }
        }}
        className={`${control} max-w-md min-w-32 flex-1 placeholder:text-ink-faint`}
      />

      {active && (
        <button
          type="button"
          onClick={onClear}
          className="h-7 rounded px-2 text-ink-muted hover:bg-raised hover:text-ink"
        >
          Clear filters
        </button>
      )}
    </div>
  );
}

import { useRef, useState } from "react";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import type { Project } from "@/lib/ipc";
import { ME, REVIEW_FILTERS, type Filters, type ReviewFilter } from "./filters";

const control =
  "h-7 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent";
/** A picker is as wide as what is picked, up to a point; the search takes what is left. */
const picker = `${control} max-w-48`;

/**
 * Project, author, review status and search. The state tabs sit in the view's header.
 *
 * Everything here narrows the rows that are *loaded*. That is why the view says when the forge
 * has more than the list holds, rather than letting a search come up empty without a word.
 */
export function PullRequestFilters({
  filters,
  search,
  projects,
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
  // A remembered author nobody in the list matches is still shown, so it can be seen and undone.
  const author = filters.author;
  const unlisted = author !== null && author !== ME && !authors.includes(author);

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
        aria-label="Author"
        value={author ?? ""}
        onChange={(event) => onChange({ author: event.target.value || null })}
        className={picker}
      >
        <option value="">Any author</option>
        {viewer && <option value={ME}>Me ({viewer})</option>}
        {unlisted && <option value={author}>{author}</option>}
        {authors.map((login) => (
          <option key={login} value={login}>
            {login}
          </option>
        ))}
      </select>

      <select
        aria-label="Reviews"
        value={filters.reviews}
        title={viewer ? undefined : "The filters about you need gh to have said who you are."}
        onChange={(event) => onChange({ reviews: event.target.value as ReviewFilter })}
        className={picker}
      >
        {REVIEW_FILTERS.map((filter) => (
          // One that is already chosen stays choosable, or the box could not show it.
          <option
            key={filter.id}
            value={filter.id}
            disabled={filter.you && !viewer && filters.reviews !== filter.id}
          >
            {filter.label}
          </option>
        ))}
      </select>

      <input
        type="search"
        aria-label="Search pull requests"
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

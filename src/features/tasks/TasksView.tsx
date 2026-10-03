import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Group, Panel, Separator, useDefaultLayout, usePanelRef } from "react-resizable-panels";
import { separator, useNarrowerThan } from "@/features/pull-requests/layout";
import { Link, Notice } from "@/features/pull-requests/PullRequestsView";
import type { Project, ProjectTasks } from "@/lib/ipc";
import { recall, useProjectsStore } from "@/stores/projects";
import { useTasksStore } from "@/stores/tasks";
import {
  assigneesOf,
  authorsOf,
  DEFAULT_FILTERS,
  FILTERS_KEY,
  filtering,
  labelsOf,
  readFilters,
  STATE_TABS,
  visible,
  type Filters,
} from "./filters";
import { NewTaskDialog } from "./NewTaskDialog";
import { TaskFilters } from "./TaskFilters";
import { TaskList } from "./TaskList";
import { TaskPane } from "./TaskPane";
import { moreOpenThanListed, openIn, rowsOf } from "./rows";

/** Narrower than this, the list and the details stack instead of sitting side by side. */
const STACK_BELOW_PX = 720;

/**
 * Every project's tasks, in the center panel: a list to filter, and one of them in full beside
 * it. Opened from the sidebar or the command palette.
 *
 * It holds nothing of its own. The rows are what the core last said about each project, and the
 * filters live in the core's `ui_state`. See `docs/design/23-tasks.md`.
 */
export function TasksView() {
  const projects = useProjectsStore((s) => s.projects);
  const ui = useProjectsStore((s) => s.ui);
  const byProject = useTasksStore((s) => s.byProject);
  const selected = useTasksStore((s) => s.selected);
  const [search, setSearch] = useState("");
  const [listHidden, setListHidden] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [creating, setCreating] = useState(false);
  const frame = useRef<HTMLDivElement>(null);
  const narrow = useNarrowerThan(frame, STACK_BELOW_PX);

  // Ages are from "now", and now moves: once a minute is as often as a row could change.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  const listed = useMemo(() => projects.filter((project) => !project.missing), [projects]);
  const rows = useMemo(() => rowsOf(listed, byProject), [listed, byProject]);
  const filters = useMemo(
    () =>
      readFilters(
        recall<unknown>(ui, FILTERS_KEY, null),
        listed.map((project) => project.id),
      ),
    [ui, listed],
  );
  const shown = useMemo(() => visible(rows, filters, search), [rows, filters, search]);
  const labels = useMemo(() => labelsOf(rows), [rows]);
  const assignees = useMemo(() => assigneesOf(rows), [rows]);
  const authors = useMemo(() => authorsOf(rows), [rows]);
  const current = rows.find((row) => row.key === selected) ?? null;

  // Closed tasks are a second question to the source, asked only while a tab that shows them
  // is the one open. The polling in the sidebar reads this and asks.
  const closedWanted = filters.state !== "open";
  useEffect(() => {
    useTasksStore.getState().wantClosed(closedWanted);
    return () => useTasksStore.getState().wantClosed(false);
  }, [closedWanted]);

  // The list alone, or the list and the details: each remembers its own sizes.
  const detailed = current !== null;
  const panelIds = useMemo(() => (detailed ? ["list", "details"] : ["list"]), [detailed]);
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: "yardsort.tasks",
    panelIds,
    storage: localStorage,
  });

  // Hiding the list collapses its panel rather than taking it out of the tree, as the Pull
  // requests view does. The store of truth is `listHidden`; the panel follows it…
  const listPanel = usePanelRef();
  const hideList = listHidden && detailed;
  useEffect(() => {
    const panel = listPanel.current;
    if (!panel || panel.isCollapsed() === hideList) return;
    if (hideList) panel.collapse();
    else panel.expand();
  }, [hideList, listPanel]);
  // …and `listHidden` follows the panel when it is dragged shut or open again.
  const syncListHidden = () => {
    const panel = listPanel.current;
    if (panel && detailed) setListHidden(panel.isCollapsed());
  };
  const viewer = listed.map((project) => byProject[project.id]?.viewer).find(Boolean) ?? null;
  // Where a task can be opened: on GitHub, with issues on, and somebody logged in.
  const writable = useMemo(
    () =>
      listed.filter((project) => {
        const found = byProject[project.id];
        return covered(found) && !found?.disabled && !found?.loggedOut;
      }),
    [listed, byProject],
  );
  const active = filtering(filters, search);
  const loading =
    listed.length > 0 &&
    (listed.every((project) => !byProject[project.id]) ||
      // The closed ones have been asked for and have not arrived.
      (closedWanted &&
        rows.length === 0 &&
        listed.some(
          (project) => covered(byProject[project.id]) && !byProject[project.id]?.closed,
        )));

  const setFilters = (next: Partial<Filters>) =>
    useProjectsStore.getState().remember(FILTERS_KEY, { ...filters, ...next });
  const clear = () => {
    setSearch("");
    // The state tab is not a filter to clear: it is where you are.
    useProjectsStore.getState().remember(FILTERS_KEY, { ...DEFAULT_FILTERS, state: filters.state });
  };
  const refresh = async (ids: string[]) => {
    setRefreshing(true);
    try {
      const { loadProject } = useTasksStore.getState();
      await Promise.all(ids.map((id) => loadProject(id, true, closedWanted)));
    } finally {
      setRefreshing(false);
    }
  };
  const select = (key: string | null) => {
    useTasksStore.getState().select(key);
    if (key === null) setListHidden(false);
  };
  const close = () => {
    const key = selected;
    select(null);
    // Back to where it was opened from, so Up and Down carry on from there.
    requestAnimationFrame(() => {
      const row = key && frame.current?.querySelector<HTMLElement>(`[data-task-row="${key}"]`);
      if (row) row.focus();
    });
  };

  // Escape closes the details from the list or from inside them — but not out from under a
  // menu, which closes itself.
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Escape" || !current) return;
    if ((event.target as HTMLElement).closest('[role="menu"]')) return;
    event.preventDefault();
    close();
  };

  const list =
    shown.length > 0 ? (
      <TaskList rows={shown} selected={selected} now={now} onSelect={select} />
    ) : (
      <Empty
        loading={loading}
        projects={listed.length}
        rows={rows.length}
        active={active}
        state={filters.state}
        onClear={clear}
      />
    );
  const pane = current && (
    <TaskPane
      row={current}
      now={now}
      listHidden={listHidden}
      onToggleList={() => setListHidden((hidden) => !hidden)}
      onClose={close}
    />
  );

  return (
    <section aria-label="Tasks" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-4 py-2">
        <h1 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">Tasks</h1>
        <div role="tablist" aria-label="State" className="flex gap-1">
          {STATE_TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={filters.state === tab.id}
              onClick={() => setFilters({ state: tab.id })}
              className={`rounded-full px-3 py-1 whitespace-nowrap ${
                filters.state === tab.id ? "bg-raised text-ink" : "text-ink-muted hover:text-ink"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          disabled={writable.length === 0}
          title={
            writable.length === 0
              ? "No project here has GitHub issues to add to"
              : "Open a new issue on a project's repository"
          }
          onClick={() => setCreating(true)}
          className="ml-auto rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
        >
          New task
        </button>
        <button
          type="button"
          disabled={refreshing || listed.length === 0}
          title="Ask GitHub again"
          onClick={() => void refresh(listed.map((project) => project.id))}
          className="rounded px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
        <button
          type="button"
          aria-label="Close tasks"
          title="Close"
          onClick={() => useProjectsStore.getState().openTasks(false)}
          className="size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>
      <TaskFilters
        filters={filters}
        search={search}
        projects={listed}
        labels={labels}
        assignees={assignees}
        authors={authors}
        viewer={viewer}
        active={active}
        onChange={setFilters}
        onSearch={setSearch}
        onClear={clear}
      />
      <Notices
        projects={listed}
        byProject={byProject}
        closedShown={closedWanted}
        onRetry={(id) => void refresh([id])}
        retrying={refreshing}
      />
      {creating && writable.length > 0 && (
        <NewTaskDialog
          projects={writable}
          // The one whose task is open, or the only one being looked at, or the first.
          initial={
            writable.find((project) => project.id === current?.project.id)?.id ??
            writable.find((project) => filters.projects.includes(project.id))?.id ??
            writable[0]!.id
          }
          onClose={() => setCreating(false)}
        />
      )}
      <div ref={frame} onKeyDown={onKeyDown} className="min-h-0 flex-1">
        {/* The list keeps its place in the tree whether or not the details are open: a row that
            was just pressed must still be there, focused, for Escape and the arrow keys. */}
        <Group
          // Remounted when it turns: the two orientations do not share sizes.
          key={narrow ? "stacked" : "side-by-side"}
          orientation={narrow ? "vertical" : "horizontal"}
          className="h-full"
          defaultLayout={narrow ? undefined : defaultLayout}
          onLayoutChanged={narrow ? undefined : onLayoutChanged}
        >
          <Panel
            id="list"
            panelRef={listPanel}
            defaultSize="45%"
            minSize={narrow ? 120 : 260}
            collapsible
            collapsedSize={0}
            onResize={syncListHidden}
          >
            {/* Hidden, not only narrow: rows nobody can see must not take focus either. */}
            <div hidden={hideList} className="h-full">
              {list}
            </div>
          </Panel>
          {pane && <Separator className={`${separator} ${narrow ? "h-px" : "w-px"}`} />}
          {pane && (
            <Panel id="details" minSize={narrow ? 160 : 300}>
              {pane}
            </Panel>
          )}
        </Group>
      </div>
    </section>
  );
}

/** Whether a project's tasks can be read at all: on GitHub, with issues on, and `gh` there. */
function covered(found: ProjectTasks | undefined): boolean {
  const kind = found?.repo?.kind;
  return !!found && found.gh && !!found.repo && kind !== "gitlab" && kind !== "bitbucket";
}

/** What the list says when it has no rows, which is never just a blank panel. */
function Empty({
  loading,
  projects,
  rows,
  active,
  state,
  onClear,
}: {
  loading: boolean;
  projects: number;
  rows: number;
  active: boolean;
  state: Filters["state"];
  onClear: () => void;
}) {
  const said =
    projects === 0
      ? "Add a project to see its tasks."
      : loading
        ? "Loading tasks…"
        : rows === 0
          ? "No tasks."
          : active
            ? "No tasks match."
            : state === "all"
              ? "No tasks."
              : `No ${state} tasks.`;
  return (
    <div role="status" className="p-4 text-ink-faint">
      <p>{said}</p>
      {active && rows > 0 && (
        <button
          type="button"
          onClick={onClear}
          className="mt-2 rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink"
        >
          Clear filters
        </button>
      )}
    </div>
  );
}

const FORGE_NAMES = { gitlab: "GitLab", bitbucket: "Bitbucket" } as const;

/**
 * Why a project's tasks are missing or incomplete, one line each, above the rows and never
 * instead of them: one project that cannot be asked says so without hiding the others.
 */
function Notices({
  projects,
  byProject,
  closedShown,
  retrying,
  onRetry,
}: {
  projects: Project[];
  byProject: Record<string, ProjectTasks>;
  /** The list is showing closed tasks, of which only the latest are read. */
  closedShown: boolean;
  retrying: boolean;
  onRetry: (projectId: string) => void;
}) {
  const answers = projects.flatMap((project) => {
    const found = byProject[project.id];
    return found ? [{ project, found }] : [];
  });
  if (answers.length === 0) return null;

  // `gh` is one program and one login for every project: said once, not once per project.
  if (answers.every(({ found }) => !found.gh)) {
    return (
      <Notice>
        Tasks are read with the GitHub CLI, and <code>gh</code> is not installed.{" "}
        <Link url="https://cli.github.com">Get it</Link>, then run{" "}
        <code className="select-text">gh auth login</code>.
      </Notice>
    );
  }
  if (answers.some(({ found }) => found.loggedOut)) {
    return (
      <Notice>
        Nobody is logged in to the GitHub CLI. Run{" "}
        <code className="select-text">gh auth login</code> in a terminal, then refresh.
      </Notice>
    );
  }

  const lines = answers.flatMap(({ project, found }) => {
    const kind = found.repo?.kind;
    if (!found.repo) {
      return [
        <Notice key={project.id}>
          <b>{project.name}</b> has no remote on a forge, so there is nowhere to ask about tasks.
        </Notice>,
      ];
    }
    if (kind === "gitlab" || kind === "bitbucket") {
      return [
        <Notice key={project.id}>
          <b>{project.name}</b> is on {FORGE_NAMES[kind]}. Tasks are read for GitHub only.
        </Notice>,
      ];
    }
    if (found.disabled) {
      return [
        <Notice key={project.id}>
          <b>{project.name}</b>&rsquo;s repository has issues switched off, so it has no tasks.
        </Notice>,
      ];
    }
    const issues = `https://${found.repo.host}/${found.repo.owner}/${found.repo.name}/issues`;
    const said = [];
    if (found.problem) {
      said.push(
        <Notice key={`${project.id}-problem`} alert>
          <b>{project.name}</b>: {found.tasks.length > 0 && "not every task could be read. "}
          <span className="select-text">{found.problem}</span>
          <button
            type="button"
            disabled={retrying}
            onClick={() => onRetry(project.id)}
            className="ml-2 text-ink-muted underline hover:text-ink disabled:opacity-40"
          >
            Retry
          </button>
        </Notice>,
      );
    } else if (moreOpenThanListed(found)) {
      said.push(
        <Notice key={`${project.id}-more`}>
          Showing the {openIn(found)} most recently updated of {found.openTotal} open in{" "}
          <b>{project.name}</b>. <Link url={issues}>See all on GitHub</Link>
        </Notice>,
      );
    }
    return said;
  });
  // Said once for the view rather than per project: it is a fact about the tab.
  const closedNote = closedShown && answers.some(({ found }) => found.closed) && (
    <Notice key="closed">Closed tasks are the 50 most recently updated of each project.</Notice>
  );
  return lines.length > 0 || closedNote ? (
    <>
      {lines}
      {closedNote}
    </>
  ) : null;
}

import { useEffect, useMemo, useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import { statusColour, statusWords } from "@/features/smoke/words";
import type { TestListing } from "@/lib/ipc";
import { recall, useProjectsStore } from "@/stores/projects";
import { scheduleOf, useSchedulesStore } from "@/stores/schedules";
import { triggerOf, useTriggersStore } from "@/stores/triggers";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { DescribeTest } from "./DescribeTest";
import {
  DEFAULT_FILTERS,
  FILTERS_KEY,
  TABS,
  readFilters,
  saveTargets,
  visibleRuns,
  visibleTests,
  type Filters,
} from "./filters";
import { TestDetail } from "./TestDetail";
import { TestsFilters } from "./TestsFilters";

/** A key for one test in one workspace: there is a row per worktree the file sits in. */
const keyOf = (listing: TestListing) => `${listing.workspaceId}\u0000${listing.item.id}`;

/**
 * Every project's smoke tests, in the center panel: a Tests tab listing each test with its last
 * verdict, and a Runs tab of every run. A test opens to its file, its history and **Run in…**;
 * a run opens its tab in its workspace. Read on opening and on Refresh, like Tasks.
 *
 * It holds nothing of its own but what is selected and what is being searched: the tests and
 * runs are the core's, and the filters live in its `ui_state`. See `docs/design/32-smoke-tests.md`.
 */
export function TestsView() {
  const projects = useProjectsStore((s) => s.projects);
  const ui = useProjectsStore((s) => s.ui);
  const catalog = useSmokeTestsStore((s) => s.catalog);
  const catalogLoaded = useSmokeTestsStore((s) => s.catalogLoaded);
  const runs = useSmokeTestsStore((s) => s.runs);
  const error = useSmokeTestsStore((s) => s.error);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [describing, setDescribing] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // Ages are from "now", and now moves: once a minute is as often as a row could change.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  // Read the catalog on opening the view, and whenever the window comes back to the front.
  useEffect(() => {
    const { loadCatalog, load } = useSmokeTestsStore.getState();
    void loadCatalog();
    void load();
    const onFocus = () => void useSmokeTestsStore.getState().loadCatalog();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  const listed = useMemo(() => projects.filter((project) => !project.missing), [projects]);
  // Every live workspace, project-qualified: where Describe it can save a new test, including a
  // workspace that has no smoke tests yet. Not the picker list below, which is history-derived.
  const describeTargets = useMemo(() => saveTargets(projects), [projects]);
  // Every workspace that has a test or a run, for the workspace picker.
  const workspaces = useMemo(() => {
    const seen = new Map<string, string>();
    for (const listing of catalog) seen.set(listing.workspaceId, listing.workspaceName);
    for (const run of runs) if (run.workspaceId) seen.set(run.workspaceId, run.workspaceName);
    return [...seen.entries()]
      .map(([id, name]) => ({ id, name }))
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
  }, [catalog, runs]);

  const filters = useMemo(
    () =>
      readFilters(
        recall<unknown>(ui, FILTERS_KEY, null),
        listed.map((project) => project.id),
        workspaces.map((w) => w.id),
      ),
    [ui, listed, workspaces],
  );
  const shownTests = useMemo(
    () => visibleTests(catalog, filters, search),
    [catalog, filters, search],
  );
  const shownRuns = useMemo(() => visibleRuns(runs, filters, search), [runs, filters, search]);

  const current = catalog.find((listing) => keyOf(listing) === selected) ?? null;

  // A filtered or opened workspace's full (capped) history may reach back past the global cap, so
  // load it on demand — otherwise an older workspace can read "No runs match" while another has
  // filled the cache with newer runs. The global cache stays as it is; these merge into it.
  const focusWorkspace = filters.workspace ?? current?.workspaceId ?? null;
  useEffect(() => {
    if (focusWorkspace) void useSmokeTestsStore.getState().loadWorkspaceRuns(focusWorkspace);
  }, [focusWorkspace]);

  const active =
    filters.projects.length > 0 ||
    filters.workspace !== null ||
    search.trim() !== "" ||
    (filters.tab === "tests"
      ? filters.verdict !== null || filters.needs
      : filters.requestedBy !== null);

  const setFilters = (next: Partial<Filters>) =>
    useProjectsStore.getState().remember(FILTERS_KEY, { ...filters, ...next });
  const clear = () => {
    setSearch("");
    useProjectsStore.getState().remember(FILTERS_KEY, { ...DEFAULT_FILTERS, tab: filters.tab });
  };
  const refresh = async () => {
    setRefreshing(true);
    try {
      const { loadCatalog, load } = useSmokeTestsStore.getState();
      await Promise.all([loadCatalog(), load()]);
    } finally {
      setRefreshing(false);
    }
  };

  /** Open a run's tab in its workspace, which leaves the Tests view for that workspace. */
  const openRun = (workspaceId: string | null, runId: string) => {
    if (!workspaceId) return;
    useProjectsStore.getState().select(workspaceId);
    useSmokeTestsStore.getState().openRun(workspaceId, runId);
  };

  return (
    <section aria-label="Tests" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-4 py-2">
        <h1 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">Tests</h1>
        <div role="tablist" aria-label="Tab" className="flex gap-1">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={filters.tab === tab.id}
              onClick={() => {
                setFilters({ tab: tab.id });
                setSelected(null);
              }}
              className={`rounded-full px-3 py-1 whitespace-nowrap ${
                filters.tab === tab.id ? "bg-raised text-ink" : "text-ink-muted hover:text-ink"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          title="Write a smoke test from a description, with a model"
          onClick={() => setDescribing(true)}
          className="ml-auto rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink"
        >
          Describe it…
        </button>
        <button
          type="button"
          disabled={refreshing}
          title="Read every project's tests again"
          onClick={() => void refresh()}
          className="rounded px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
        <button
          type="button"
          aria-label="Close tests"
          title="Close"
          onClick={() => useProjectsStore.getState().openTests(false)}
          className="size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      <TestsFilters
        filters={filters}
        search={search}
        projects={listed}
        workspaces={workspaces}
        active={active}
        onChange={setFilters}
        onSearch={setSearch}
        onClear={clear}
      />

      {error && (
        <p
          role="alert"
          className="shrink-0 border-b border-line px-4 py-1.5 text-red-400 select-text"
        >
          {error}
        </p>
      )}

      <div className="flex min-h-0 flex-1">
        <div
          className={`min-h-0 overflow-y-auto ${current ? "w-80 shrink-0 border-r border-line" : "flex-1"}`}
        >
          {filters.tab === "tests" ? (
            <TestsList
              listings={shownTests}
              loaded={catalogLoaded}
              total={catalog.length}
              active={active}
              now={now}
              selected={selected}
              onSelect={setSelected}
            />
          ) : (
            <RunsList
              runs={shownRuns}
              total={runs.length}
              active={active}
              now={now}
              onOpen={openRun}
            />
          )}
        </div>
        {current && (
          <div className="min-h-0 flex-1 overflow-y-auto">
            {/* Keyed by the selection so switching tests remounts it: its run-destination state
                starts from the newly selected listing's workspace, not the one before. */}
            <TestDetail
              key={selected ?? undefined}
              listing={current}
              catalog={catalog}
              runs={runs}
              now={now}
              onOpenRun={openRun}
              onClose={() => setSelected(null)}
            />
          </div>
        )}
      </div>

      {describing && (
        <DescribeTest
          workspaces={describeTargets}
          onClose={() => setDescribing(false)}
          onSaved={() => void refresh()}
        />
      )}
    </section>
  );
}

function TestsList({
  listings,
  loaded,
  total,
  active,
  now,
  selected,
  onSelect,
}: {
  listings: TestListing[];
  loaded: boolean;
  total: number;
  active: boolean;
  now: number;
  selected: string | null;
  onSelect: (key: string | null) => void;
}) {
  if (listings.length === 0) {
    const said = !loaded
      ? "Loading tests…"
      : total === 0
        ? "No smoke tests yet. An agent writes one with ys test guide, or press Describe it."
        : active
          ? "No tests match."
          : "No smoke tests yet.";
    return (
      <p role="status" className="p-4 text-ink-faint">
        {said}
      </p>
    );
  }
  return (
    <ul aria-label="Tests">
      {listings.map((listing) => {
        const key = `${listing.workspaceId}\u0000${listing.item.id}`;
        const badges: string[] = [];
        if (!listing.item.runnable) badges.push("won’t run");
        if (listing.item.browserMissing) badges.push("needs a browser");
        if (listing.envMissing.length > 0) badges.push(`unset ${listing.envMissing.join(", ")}`);
        return (
          <li key={key}>
            <button
              type="button"
              aria-current={selected === key ? "true" : undefined}
              onClick={() => onSelect(selected === key ? null : key)}
              className={`flex w-full flex-col gap-0.5 border-b border-line px-4 py-2 text-left hover:bg-raised ${
                selected === key ? "bg-raised" : ""
              }`}
            >
              <div className="flex items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate font-medium text-ink">
                  {listing.item.name}
                </span>
                <span
                  className={`shrink-0 text-[11px] ${statusColour(listing.lastRun?.status ?? "")}`}
                >
                  {listing.lastRun
                    ? statusWords({ status: listing.lastRun.status, stopping: false })
                    : "never run"}
                </span>
              </div>
              <div className="flex items-center gap-2 text-[12px] text-ink-faint">
                <span className="truncate">
                  {listing.projectName} / {listing.workspaceName}
                </span>
                {listing.lastRun && (
                  <span className="shrink-0 tabular-nums">
                    · {timeAgo(listing.lastRun.createdAt, now)}
                  </span>
                )}
              </div>
              {badges.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {badges.map((badge) => (
                    <span
                      key={badge}
                      className="rounded-full border border-amber-300/40 px-1.5 text-[10px] leading-4 text-amber-300"
                    >
                      {badge}
                    </span>
                  ))}
                </div>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function RunsList({
  runs,
  total,
  active,
  now,
  onOpen,
}: {
  runs: ReturnType<typeof useSmokeTestsStore.getState>["runs"];
  total: number;
  active: boolean;
  now: number;
  onOpen: (workspaceId: string | null, runId: string) => void;
}) {
  // The runs a schedule started, to say so on each.
  const started = useSchedulesStore((s) => s.started);
  // And the runs a trigger started.
  const triggered = useTriggersStore((s) => s.started);
  if (runs.length === 0) {
    return (
      <p role="status" className="p-4 text-ink-faint">
        {total === 0 ? "No runs yet." : active ? "No runs match." : "No runs yet."}
      </p>
    );
  }
  return (
    <ul aria-label="Runs">
      {runs.map((run) => (
        <li key={run.id}>
          <button
            type="button"
            onClick={() => onOpen(run.workspaceId, run.id)}
            disabled={!run.workspaceId}
            title={run.workspaceId ? "Open this run in its workspace" : "Its workspace is gone"}
            className="flex w-full flex-col gap-0.5 border-b border-line px-4 py-2 text-left hover:bg-raised disabled:opacity-50"
          >
            <div className="flex items-baseline gap-2">
              <span className="min-w-0 flex-1 truncate font-medium text-ink">{run.testName}</span>
              <span className={`shrink-0 text-[11px] ${statusColour(run.status)}`}>
                {statusWords(run)}
              </span>
            </div>
            <div className="flex items-center gap-2 text-[12px] text-ink-faint">
              <span className="truncate">{run.workspaceName}</span>
              <span className="shrink-0 tabular-nums">· {timeAgo(run.createdAt, now)}</span>
              {scheduleOf(started, run.id) && (
                <span className="truncate">
                  · Scheduled · {scheduleOf(started, run.id)?.scheduleName}
                </span>
              )}
              {triggerOf(triggered, run.id) && (
                <span className="truncate">
                  · Triggered · {triggerOf(triggered, run.id)?.triggerName}
                </span>
              )}
            </div>
          </button>
        </li>
      ))}
    </ul>
  );
}

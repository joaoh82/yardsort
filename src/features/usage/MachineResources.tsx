import { useCallback, useEffect, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { errorMessage, ipc, type Load, type MachineReport, type TerminalLoad } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { AreaChart } from "./AreaChart";
import { formatBytes, formatPercent } from "./format";

/** How often the view samples while it is open. Nothing is sampled while it is closed. */
const MACHINE_POLL_MS = 2000;
const WINDOW_MS = 5 * 60_000;

type SortKey = "memory" | "cpu";

const APP_PARTS: Record<string, string> = {
  main: "Main",
  webview: "Webview",
  host: "Terminal host",
};

/**
 * What Yardsort and the agents it runs use of this machine: totals, the last five minutes, and
 * a breakdown by project, workspace and terminal. A terminal's figures are its whole process
 * tree — the agent and everything it started.
 */
export function MachineResources() {
  const [report, setReport] = useState<MachineReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sort, setSort] = useState<SortKey>("memory");

  const sample = useCallback(
    () =>
      ipc.usageMachine().then(
        (sampled) => {
          setReport(sampled);
          setError(null);
        },
        (reason) => setError(errorMessage(reason)),
      ),
    [],
  );

  useEffect(() => {
    void sample();
    const timer = window.setInterval(() => {
      if (!document.hidden) void sample();
    }, MACHINE_POLL_MS);
    return () => window.clearInterval(timer);
  }, [sample]);

  return (
    <div className="grid gap-4">
      <div className="flex items-center justify-end gap-4 text-ink-muted">
        <span className="flex items-center gap-1.5">
          <span aria-hidden className="size-1.5 rounded-full bg-agent-grok" />
          Live · this machine · every {MACHINE_POLL_MS / 1000} s
        </span>
        <button
          type="button"
          onClick={() => setSort(sort === "memory" ? "cpu" : "memory")}
          title="Change what the processes are sorted by"
          className="rounded px-1.5 py-0.5 hover:bg-raised hover:text-ink"
        >
          Sort: {sort === "memory" ? "Memory" : "CPU"}
        </button>
        <button
          type="button"
          aria-label="Sample now"
          title="Sample now"
          onClick={() => void sample()}
          className="rounded px-1.5 py-0.5 hover:bg-raised hover:text-ink"
        >
          ↻
        </button>
      </div>
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      {!report ? (
        !error && <p className="text-ink-muted">Measuring…</p>
      ) : (
        <Report report={report} sort={sort} />
      )}
    </div>
  );
}

function Report({ report, sort }: { report: MachineReport; sort: SortKey }) {
  const { system, yardsort } = report;
  const other = Math.max(0, system.usedMemory - yardsort.memory);
  const free = Math.max(0, system.totalMemory - system.usedMemory);
  const percent = (part: number) =>
    system.totalMemory > 0 ? (part / system.totalMemory) * 100 : 0;
  const start = report.sampledAt - WINDOW_MS;
  const xs = report.history.map((point) => Math.max(0, (point.at - start) / WINDOW_MS));
  const times = report.history.map((point) =>
    new Date(point.at).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }),
  );

  return (
    <>
      <dl className="grid grid-cols-2 gap-x-6 gap-y-3 border-y border-line py-3 sm:grid-cols-3 xl:grid-cols-6">
        <Stat label="Yardsort CPU" value={formatPercent(yardsort.cpu)} />
        <Stat label="Yardsort memory" value={formatBytes(yardsort.memory)} />
        <Stat label="Share of RAM" value={formatPercent(percent(yardsort.memory))} />
        <Stat
          label="System memory"
          value={`${formatBytes(system.usedMemory)} · ${Math.round(percent(system.usedMemory))}%`}
        />
        <Stat label="CPU cores" value={String(system.cpuCount)} />
        {system.loadOne !== null ? (
          <Stat label="Load (1 m)" value={system.loadOne.toFixed(2)} />
        ) : (
          <Stat label="System CPU" value={formatPercent(system.cpu)} />
        )}
      </dl>

      <div>
        <div
          role="img"
          aria-label={`Memory: Yardsort ${formatBytes(yardsort.memory)}, other apps ${formatBytes(other)}, free ${formatBytes(free)}`}
          className="flex h-1.5 gap-0.5 overflow-hidden rounded-full"
        >
          <span className="bg-ink" style={{ width: `${percent(yardsort.memory)}%` }} />
          <span className="bg-ink-faint" style={{ width: `${percent(other)}%` }} />
          <span className="flex-1 bg-raised" />
        </div>
        <div className="mt-2 flex flex-wrap gap-4 text-ink-muted">
          <Legend swatch="bg-ink" label="Yardsort" value={formatBytes(yardsort.memory)} />
          <Legend swatch="bg-ink-faint" label="Other apps" value={formatBytes(other)} />
          <Legend swatch="bg-raised" label="Free" value={formatBytes(free)} />
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="Yardsort CPU · last 5 min" value={formatPercent(yardsort.cpu)}>
          <AreaChart
            label="Yardsort CPU over the last five minutes"
            points={times}
            xs={xs}
            height={90}
            format={formatPercent}
            series={[
              {
                id: "cpu",
                label: "CPU",
                color: "var(--color-accent)",
                values: report.history.map((p) => p.cpu),
              },
            ]}
          />
        </Card>
        <Card title="Yardsort memory · last 5 min" value={formatBytes(yardsort.memory)}>
          <AreaChart
            label="Yardsort memory over the last five minutes"
            points={times}
            xs={xs}
            height={90}
            format={formatBytes}
            series={[
              {
                id: "memory",
                label: "Memory",
                color: "var(--color-agent-grok)",
                values: report.history.map((p) => p.memory),
              },
            ]}
          />
        </Card>
      </div>

      <Processes report={report} sort={sort} />
    </>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-ink-muted">{label}</dt>
      <dd className="m-0 text-lg text-ink tabular-nums">{value}</dd>
    </div>
  );
}

function Legend({ swatch, label, value }: { swatch: string; label: string; value: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span aria-hidden className={`size-2 rounded-full ${swatch}`} />
      {label} <span className="text-ink tabular-nums">{value}</span>
    </span>
  );
}

function Card(props: { title: string; value: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-line bg-surface p-3">
      <header className="mb-2 flex items-baseline justify-between">
        <h3 className="m-0 text-[12px] font-normal text-ink-muted">{props.title}</h3>
        <span className="text-base text-ink tabular-nums">{props.value}</span>
      </header>
      {props.children}
    </section>
  );
}

function byKey<T extends { load: Load }>(rows: T[], sort: SortKey): T[] {
  return [...rows].sort((a, b) => b.load[sort] - a.load[sort]);
}

function Processes({ report, sort }: { report: MachineReport; sort: SortKey }) {
  const [collapsed, setCollapsed] = useState<string[]>([]);
  const toggle = (id: string) =>
    setCollapsed((ids) => (ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]));
  const whole = report.yardsort.memory;
  const appLoad = report.app.reduce<Load>(
    (sum, part) => ({
      cpu: sum.cpu + part.load.cpu,
      memory: sum.memory + part.load.memory,
      processes: sum.processes + part.load.processes,
    }),
    { cpu: 0, memory: 0, processes: 0 },
  );

  return (
    <table className="w-full border-collapse overflow-hidden rounded-lg border border-line">
      <thead>
        <tr className="border-b border-line text-left text-[11px] tracking-wider text-ink-muted uppercase">
          <th className="px-3 py-2 font-normal">Process</th>
          <th className="w-20 px-3 py-2 text-right font-normal">CPU</th>
          <th className="w-24 px-3 py-2 text-right font-normal">Memory</th>
          <th className="w-40 px-3 py-2 font-normal normal-case tracking-normal">Memory share</th>
        </tr>
      </thead>
      <tbody>
        <Row label="Yardsort app" load={appLoad} whole={whole} depth={0} strong />
        {report.app.map((part) => (
          <Row
            key={part.part}
            label={APP_PARTS[part.part] ?? part.part}
            load={part.load}
            whole={whole}
            depth={1}
            muted
          />
        ))}
        {byKey(report.projects, sort).map((project) => {
          const open = !collapsed.includes(project.projectId);
          return (
            <ProjectRows
              key={project.projectId}
              open={open}
              onToggle={() => toggle(project.projectId)}
              name={project.name}
              load={project.load}
              whole={whole}
            >
              {open &&
                byKey(project.workspaces, sort).flatMap((workspace) => {
                  const wsOpen = !collapsed.includes(workspace.workspaceId);
                  return [
                    <Row
                      key={workspace.workspaceId}
                      label={workspace.name}
                      load={workspace.load}
                      whole={whole}
                      depth={1}
                      expanded={wsOpen}
                      onToggle={() => toggle(workspace.workspaceId)}
                      onOpen={() => useProjectsStore.getState().select(workspace.workspaceId)}
                    />,
                    ...(wsOpen
                      ? byKey(workspace.terminals, sort).map((terminal) => (
                          <TerminalRow key={terminal.sessionId} terminal={terminal} whole={whole} />
                        ))
                      : []),
                  ];
                })}
            </ProjectRows>
          );
        })}
        {report.loose.length > 0 && (
          <>
            <Row
              label="Other terminals"
              load={report.loose.reduce<Load>(
                (sum, t) => ({
                  cpu: sum.cpu + t.load.cpu,
                  memory: sum.memory + t.load.memory,
                  processes: sum.processes + t.load.processes,
                }),
                { cpu: 0, memory: 0, processes: 0 },
              )}
              whole={whole}
              depth={0}
              strong
            />
            {byKey(report.loose, sort).map((terminal) => (
              <TerminalRow key={terminal.sessionId} terminal={terminal} whole={whole} />
            ))}
          </>
        )}
        {report.projects.length === 0 && report.loose.length === 0 && (
          <tr>
            <td colSpan={4} className="px-3 py-3 text-ink-faint">
              No terminals are running.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

function ProjectRows(props: {
  open: boolean;
  onToggle: () => void;
  name: string;
  load: Load;
  whole: number;
  children: React.ReactNode;
}) {
  return (
    <>
      <Row
        label={props.name}
        load={props.load}
        whole={props.whole}
        depth={0}
        heading
        expanded={props.open}
        onToggle={props.onToggle}
      />
      {props.children}
    </>
  );
}

function TerminalRow({ terminal, whole }: { terminal: TerminalLoad; whole: number }) {
  const label = terminal.label ?? (terminal.run ? "Run command" : "Shell");
  return (
    <Row
      label={label}
      icon={terminal.harness ? <HarnessIcon id={terminal.harness} size={12} /> : undefined}
      load={terminal.load}
      whole={whole}
      depth={2}
      muted
      title={`${terminal.load.processes} process${terminal.load.processes === 1 ? "" : "es"}`}
    />
  );
}

function Row(props: {
  label: string;
  load: Load;
  whole: number;
  depth: number;
  strong?: boolean;
  heading?: boolean;
  muted?: boolean;
  icon?: React.ReactNode;
  title?: string;
  expanded?: boolean;
  onToggle?: () => void;
  onOpen?: () => void;
}) {
  const share = props.whole > 0 ? Math.min(100, (props.load.memory / props.whole) * 100) : 0;
  const tone = props.muted ? "text-ink-muted" : "text-ink";
  const name = props.heading ? (
    <span className="text-[11px] font-semibold tracking-wider uppercase">{props.label}</span>
  ) : (
    props.label
  );
  return (
    <tr className={props.depth === 0 ? "border-t border-line" : ""} title={props.title}>
      <td className={`px-3 py-1.5 ${tone}`} style={{ paddingLeft: 12 + props.depth * 20 }}>
        <span className="flex min-w-0 items-center gap-1.5">
          {props.onToggle && (
            <button
              type="button"
              aria-expanded={props.expanded}
              aria-label={`${props.expanded ? "Collapse" : "Expand"} ${props.label}`}
              onClick={props.onToggle}
              className="w-4 shrink-0 text-ink-faint hover:text-ink"
            >
              {props.expanded ? "▾" : "▸"}
            </button>
          )}
          {props.icon}
          {props.onOpen ? (
            <button
              type="button"
              onClick={props.onOpen}
              title={`Open ${props.label}`}
              className="min-w-0 truncate text-left hover:underline"
            >
              {name}
            </button>
          ) : (
            <span className={`min-w-0 truncate ${props.strong ? "font-medium" : ""}`}>{name}</span>
          )}
        </span>
      </td>
      <td className={`px-3 py-1.5 text-right tabular-nums ${tone}`}>
        {formatPercent(props.load.cpu)}
      </td>
      <td className={`px-3 py-1.5 text-right tabular-nums ${tone}`}>
        {formatBytes(props.load.memory)}
      </td>
      <td className="px-3 py-1.5">
        <div className="h-1 rounded-full bg-raised">
          <div className="h-1 rounded-full bg-ink-muted" style={{ width: `${share}%` }} />
        </div>
      </td>
    </tr>
  );
}

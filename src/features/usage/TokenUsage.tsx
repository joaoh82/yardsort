import { useCallback, useEffect, useRef, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { errorMessage, ipc, type AgentLimits, type PlaceUsage, type UsageReport } from "@/lib/ipc";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { DailyBars } from "./DailyBars";
import { formatCost, formatTokens, formatUntil, share, shortDate, windowName } from "./format";

const RANGES = [7, 30, 90] as const;
type Metric = "cost" | "tokens";

const AGENT_NAMES: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex",
  grok: "Grok",
};

const agentColor = (agent: string) => `var(--color-agent-${agent})`;

/** How many rows a table shows before "Show all". */
const SHORT_LIST = 8;

function useAgentName() {
  const harnesses = useHarnessStore((s) => s.harnesses);
  return (agent: string) =>
    harnesses.find((h) => h.id === agent)?.label ?? AGENT_NAMES[agent] ?? agent;
}

/**
 * The tokens the agents spent, from their own session logs on this machine, and what those
 * tokens would cost billed per token at API rates.
 */
export function TokenUsage() {
  const [days, setDays] = useState<(typeof RANGES)[number]>(30);
  const [metric, setMetric] = useState<Metric>("cost");
  // What was read, for which range, and when: the limits' countdowns run from that moment.
  const [read, setRead] = useState<{ days: number; report: UsageReport; at: number } | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The range last asked for. Replies can arrive out of order — a long range is slower to add
  // up — and one for a range no longer chosen must not replace the one that is.
  const wanted = useRef(days);
  const load = useCallback(
    (range: number) =>
      ipc.usageTokens(range).then(
        (report) => {
          if (wanted.current !== range) return;
          setRead({ days: range, report, at: Date.now() });
          setError(null);
        },
        (reason) => {
          if (wanted.current === range) setError(errorMessage(reason));
        },
      ),
    [],
  );

  useEffect(() => {
    wanted.current = days;
    void load(days);
  }, [days, load]);

  const report = read?.report ?? null;
  const loading = refreshing || (!error && read?.days !== days);

  return (
    <div className="grid gap-5">
      <div className="flex items-center justify-end gap-3 text-ink-muted">
        <span>{loading ? "Reading session logs…" : "From session logs on this machine"}</span>
        <button
          type="button"
          aria-label="Read the logs again"
          title="Read the logs again"
          disabled={loading}
          onClick={() => {
            setRefreshing(true);
            void load(days).finally(() => setRefreshing(false));
          }}
          className="rounded px-1.5 py-0.5 hover:bg-raised hover:text-ink disabled:opacity-40"
        >
          ↻
        </button>
      </div>
      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      {report && (
        <>
          <Limits limits={report.limits} now={read!.at} />
          <Sources report={report} />
          <Spending
            report={report}
            metric={metric}
            onMetric={setMetric}
            days={days}
            onDays={setDays}
          />
        </>
      )}
    </div>
  );
}

function Toggle<T extends string | number>(props: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div role="group" aria-label={props.label} className="flex rounded-md bg-raised p-0.5">
      {props.options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={option.value === props.value}
          onClick={() => props.onChange(option.value)}
          className={`rounded px-2 py-0.5 ${
            option.value === props.value ? "bg-canvas text-ink" : "text-ink-muted hover:text-ink"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** Rate limits, as each agent last heard them from its vendor. Only Codex writes them down. */
function Limits({ limits, now }: { limits: AgentLimits[]; now: number }) {
  const name = useAgentName();
  if (limits.length === 0) return null;
  return (
    <section aria-label="Plan limits" className="grid gap-3 lg:grid-cols-2">
      {limits.map((agent) => (
        <div key={agent.agent} className="rounded-lg border border-line bg-surface p-3">
          <header className="mb-2 flex items-center gap-2">
            <HarnessIcon id={agent.agent} />
            <span className="text-ink">{name(agent.agent)}</span>
            {agent.plan && (
              <span className="rounded border border-line px-1.5 text-[10px] tracking-wider text-ink-muted uppercase">
                {agent.plan}
              </span>
            )}
            <span
              className="ml-auto text-[11px] text-ink-faint"
              title={new Date(agent.observedAt).toLocaleString()}
            >
              as of {formatUntil(now - agent.observedAt)} ago
            </span>
          </header>
          {agent.windows.map((window) => {
            const reset = window.resetsAt !== null && window.resetsAt <= now;
            return (
              <div
                key={window.minutes ?? "limit"}
                className="grid grid-cols-[7rem_1fr_3rem_5rem] items-center gap-3 py-0.5"
              >
                <span className="text-ink-muted">{windowName(window.minutes)}</span>
                <div
                  role="meter"
                  aria-label={`${windowName(window.minutes)} limit used`}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={reset ? 0 : window.usedPercent}
                  className="h-1 rounded-full bg-raised"
                >
                  <div
                    className="h-1 rounded-full bg-ink"
                    style={{ width: `${reset ? 0 : Math.min(100, window.usedPercent)}%` }}
                  />
                </div>
                <span className="text-right text-ink tabular-nums">
                  {reset ? "—" : `${Math.round(window.usedPercent)}%`}
                </span>
                <span className="text-right text-ink-muted tabular-nums">
                  {window.resetsAt === null
                    ? ""
                    : reset
                      ? "reset since"
                      : `↻ ${formatUntil(window.resetsAt - now)}`}
                </span>
              </div>
            );
          })}
        </div>
      ))}
    </section>
  );
}

function Sources({ report }: { report: UsageReport }) {
  const name = useAgentName();
  if (report.agents.length === 0) {
    return (
      <p className="rounded-lg border border-line bg-surface p-3 text-ink-muted">
        No session logs found. Yardsort reads the logs Claude Code, Codex and Grok keep on this
        machine — in <code>~/.claude</code>, <code>~/.codex</code> and <code>~/.grok</code>, or
        wherever <code>CLAUDE_CONFIG_DIR</code>, <code>CODEX_HOME</code> and <code>GROK_HOME</code>{" "}
        point.
      </p>
    );
  }
  return (
    <ul aria-label="Where the figures come from" className="flex flex-wrap gap-x-5 gap-y-1">
      {report.agents.map((agent) => (
        <li key={agent.agent} className="flex items-center gap-1.5 text-ink-muted">
          <HarnessIcon id={agent.agent} size={12} />
          <span className="text-ink">{name(agent.agent)}</span>
          <span className="text-ink-faint">
            {agent.location ?? "?"} · {agent.files} log{agent.files === 1 ? "" : "s"}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Spending(props: {
  report: UsageReport;
  metric: Metric;
  onMetric: (metric: Metric) => void;
  days: number;
  onDays: (days: (typeof RANGES)[number]) => void;
}) {
  const { report, metric } = props;
  const name = useAgentName();
  const value = (cost: number, tokens: number) => (metric === "cost" ? cost : tokens);
  const format = metric === "cost" ? formatCost : formatTokens;
  const total = value(report.cost, report.totals.processed);
  const points = report.daily.map((day) => shortDate(day.date));
  const last = points.length - 1;
  const ticks = last > 0 ? [0, Math.round(last / 2), last] : [0];
  const input = report.totals.cacheRead + report.totals.cacheWrite + report.totals.uncachedInput;

  return (
    <section aria-label="Token usage" className="grid gap-4 border-t border-line pt-4">
      <header className="flex flex-wrap items-center gap-3">
        <h3 className="m-0 text-[12px] font-semibold tracking-wider text-ink-muted uppercase">
          Token usage
        </h3>
        <span className="text-ink-faint">
          {shortDate(report.from)} – {shortDate(report.to)} · estimated at API rates
        </span>
        <div className="ml-auto flex gap-2">
          <Toggle
            label="Show"
            value={metric}
            onChange={props.onMetric}
            options={[
              { value: "cost", label: "Cost" },
              { value: "tokens", label: "Tokens" },
            ]}
          />
          <Toggle
            label="Range"
            value={props.days as (typeof RANGES)[number]}
            onChange={props.onDays}
            options={RANGES.map((days) => ({ value: days, label: `${days}d` }))}
          />
        </div>
      </header>

      <div className="grid gap-6 lg:grid-cols-[16rem_1fr]">
        <div className="grid content-start gap-3">
          <div>
            <div className="text-3xl text-ink tabular-nums">
              {format(total)}
              {metric === "cost" && "*"}
            </div>
            {metric === "cost" ? (
              <p className="m-0 text-[11px] text-ink-faint">
                * if billed per token at API rates. A subscription is not billed this way.
                {report.unpricedModels.length > 0 &&
                  ` Not priced: ${report.unpricedModels.join(", ")}.`}
              </p>
            ) : (
              <p className="m-0 text-[11px] text-ink-faint">tokens processed</p>
            )}
          </div>
          <ul aria-label="By agent" className="grid gap-1.5">
            {report.agents.map((agent) => (
              <li key={agent.agent} className="flex items-center gap-2">
                <span
                  aria-hidden
                  className="size-2 rounded-sm"
                  style={{ background: agentColor(agent.agent) }}
                />
                <HarnessIcon id={agent.agent} size={12} />
                <span className="flex-1 text-ink">{name(agent.agent)}</span>
                <span className="text-ink tabular-nums">
                  {format(value(agent.cost, agent.tokens))}
                </span>
                <span className="w-9 text-right text-ink-muted tabular-nums">
                  {share(value(agent.cost, agent.tokens), total)}
                </span>
              </li>
            ))}
          </ul>
        </div>
        <DailyBars
          label={`${metric === "cost" ? "Cost" : "Tokens"} per day, by agent`}
          days={points}
          ticks={ticks}
          format={format}
          height={220}
          series={report.agents.map((agent, i) => ({
            id: agent.agent,
            label: name(agent.agent),
            color: agentColor(agent.agent),
            values: report.daily.map((day) => value(day.cost[i] ?? 0, day.tokens[i] ?? 0)),
          }))}
        />
      </div>

      <dl className="grid grid-cols-2 gap-x-6 gap-y-3 border-y border-line py-3 sm:grid-cols-3 xl:grid-cols-6">
        <Stat label="Processed tokens" value={formatTokens(report.totals.processed)} />
        <Stat
          label="Cached input"
          value={`${formatTokens(report.totals.cacheRead)} · ${share(report.totals.cacheRead, input)}`}
        />
        <Stat label="Cache writes" value={formatTokens(report.totals.cacheWrite)} />
        <Stat label="Uncached input" value={formatTokens(report.totals.uncachedInput)} />
        <Stat label="Output" value={formatTokens(report.totals.output)} />
        <Stat label="Cache savings" value={formatCost(report.totals.cacheSavings)} />
      </dl>

      <div className="grid gap-6 xl:grid-cols-2">
        <Models report={report} />
        <Places report={report} metric={metric} />
      </div>
    </section>
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

function ShowAll(props: { count: number; all: boolean; onToggle: () => void }) {
  if (props.count <= SHORT_LIST) return null;
  return (
    <button type="button" onClick={props.onToggle} className="text-ink-muted hover:text-ink">
      {props.all ? "Show fewer" : `All ${props.count} →`}
    </button>
  );
}

function Models({ report }: { report: UsageReport }) {
  const [all, setAll] = useState(false);
  const rows = all ? report.models : report.models.slice(0, SHORT_LIST);
  return (
    <table className="w-full border-collapse self-start">
      <caption className="sr-only">By model</caption>
      <thead>
        <tr className="border-b border-line text-left text-ink-muted">
          <th className="py-1.5 font-normal">Model</th>
          <th className="py-1.5 text-right font-normal">Cost</th>
          <th className="py-1.5 text-right font-normal">Share</th>
          <th className="py-1.5 text-right font-normal">Tokens</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((model) => (
          <tr key={`${model.agent}:${model.model}`}>
            <td className="py-1 text-ink">
              <span className="flex items-center gap-2">
                <span
                  aria-hidden
                  className="size-1.5 rounded-full"
                  style={{ background: agentColor(model.agent) }}
                />
                <span className="truncate select-text">{model.model}</span>
              </span>
            </td>
            <td className="py-1 text-right text-ink tabular-nums">
              {model.knownCost === null ? (
                <span title="No known price for this model" className="text-ink-faint">
                  —
                </span>
              ) : (
                formatCost(model.knownCost)
              )}
            </td>
            <td className="py-1 text-right text-ink-muted tabular-nums">
              {model.knownCost === null ? "" : share(model.knownCost, report.cost)}
            </td>
            <td className="py-1 text-right text-ink-muted tabular-nums">
              {formatTokens(model.tokens)}
            </td>
          </tr>
        ))}
        {report.models.length === 0 && (
          <tr>
            <td colSpan={4} className="py-2 text-ink-faint">
              Nothing in this range.
            </td>
          </tr>
        )}
      </tbody>
      <tfoot>
        <tr className="border-t border-line">
          <td className="py-1.5 text-ink">
            Total <ShowAll count={report.models.length} all={all} onToggle={() => setAll(!all)} />
          </td>
          <td className="py-1.5 text-right text-ink tabular-nums">~{formatCost(report.cost)}</td>
          <td />
          <td className="py-1.5 text-right text-ink tabular-nums">
            {formatTokens(report.totals.processed)}
          </td>
        </tr>
      </tfoot>
    </table>
  );
}

function Places({ report, metric }: { report: UsageReport; metric: Metric }) {
  const [all, setAll] = useState(false);
  const select = useProjectsStore((s) => s.select);
  // One unit for every row, the one the tab is showing: an unpriced place has no cost, and its
  // token count beside other places' dollars would dwarf them.
  const measure = (place: PlaceUsage) => (metric === "cost" ? place.cost : place.tokens);
  const sorted = [...report.places].sort((a, b) => measure(b) - measure(a));
  const rows = all ? sorted : sorted.slice(0, SHORT_LIST);
  const top = Math.max(0, ...sorted.map(measure));
  return (
    <section aria-label="By workspace" className="self-start">
      <header className="flex items-center border-b border-line py-1.5 text-ink-muted">
        <span className="flex-1">Workspace</span>
        <ShowAll count={report.places.length} all={all} onToggle={() => setAll(!all)} />
        <span className="ml-4">{metric === "cost" ? "Cost" : "Tokens"}</span>
      </header>
      <ul className="grid">
        {rows.map((place) => {
          const workspaceId = place.workspaceId;
          const key = workspaceId ?? place.folder ?? place.label;
          const label = place.project ? `${place.project} · ${place.label}` : place.label;
          const size = top > 0 ? (measure(place) / top) * 100 : 0;
          return (
            <li key={key} className="py-1">
              <div className="flex items-center gap-3">
                {workspaceId ? (
                  <button
                    type="button"
                    onClick={() => select(workspaceId)}
                    title={`Open ${label}`}
                    className="min-w-0 flex-1 truncate text-left text-ink hover:underline"
                  >
                    {label}
                  </button>
                ) : (
                  <span
                    className="min-w-0 flex-1 truncate text-ink-muted"
                    title={place.folder ?? undefined}
                  >
                    {label}
                  </span>
                )}
                <span className="text-ink-muted tabular-nums">{formatTokens(place.tokens)}</span>
                <span className="w-16 text-right text-ink tabular-nums">
                  {formatCost(place.cost)}
                </span>
              </div>
              <div className="mt-1 h-0.5 rounded-full bg-raised">
                <div className="h-0.5 rounded-full bg-ink-muted" style={{ width: `${size}%` }} />
              </div>
            </li>
          );
        })}
        {report.places.length === 0 && (
          <li className="py-2 text-ink-faint">Nothing in this range.</li>
        )}
      </ul>
    </section>
  );
}

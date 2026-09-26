import type { ChangeSet, FileReports } from "@/lib/ipc";
import { eventTime } from "@/features/activity/describe";
import { useProvenanceStore } from "@/stores/provenance";
import { CAVEAT, describeReports, useAnyReporting, useReports, who } from "./reported";

/**
 * What the workspace's agents said they wrote, beside what git shows: a badge on a changed
 * file, a line above the list, a word in the viewer's header. The words are in `reported.ts`.
 */

/** One badge per reporting agent on a changed file, or nothing when no agent reported it. */
export function ReportedBadges({ file }: { file: FileReports | undefined }) {
  if (!file || file.reports.length === 0) return null;
  const names = [...new Set(file.reports.map(who))];
  return (
    <span
      className="flex shrink-0 gap-1"
      title={`${describeReports(file.reports)} ${CAVEAT}`}
      data-testid="reported"
    >
      {names.map((name) => (
        <span
          key={name}
          className="rounded border border-accent/40 px-1 text-[10px] leading-4 text-accent"
        >
          {name}
        </span>
      ))}
    </span>
  );
}

/** The line above the change list: how many changed files an agent reported writing. */
export function ProvenanceNote({ changes }: { changes: ChangeSet }) {
  const provenance = useProvenanceStore((s) => s.provenance);
  const reports = useReports();
  if (!provenance || !reports) return null;
  const reporting = provenance.runs.filter((run) => run.capture !== null).length;
  if (reporting === 0) return null;

  const paths = new Set([...changes.uncommitted, ...changes.committed].map((c) => c.path));
  if (paths.size === 0) return null;
  const reported = [...paths].filter((path) => (reports.get(path)?.reports.length ?? 0) > 0).length;
  // A run is silent only when that is known: its start event said so, or nothing of the
  // agent's ever came through it. A run with neither is unknown, and not counted either way.
  const silent = provenance.runs.filter((run) => run.capture === null && run.captureKnown).length;

  // The alternatives are named every time, and a command the agent ran is one of them: a file
  // made by `echo > hello.txt` is the agent's doing and carries no report, since a command
  // names no file.
  const alternatives = `you, a script, a command the agent ran${
    silent > 0
      ? `, or one of the ${silent} agent run${silent === 1 ? "" : "s"} here that was not reporting`
      : ""
  }`;
  const summary =
    reported === paths.size
      ? `Every changed file was reported written by an agent.`
      : reported === 0
        ? `No changed file was reported written by an agent — the changes came from ${alternatives}.`
        : `Agents reported writing ${reported} of ${paths.size} changed files. The rest changed with no report — ${alternatives}.`;
  return (
    <p className="px-3 py-1 text-[11px] text-ink-faint" title={CAVEAT}>
      {summary}
    </p>
  );
}

/** The viewer header's word on the open diff: who reported writing it, if anyone did. */
export function ReportedLine({ path }: { path: string }) {
  const reports = useReports();
  const reporting = useAnyReporting();
  if (!reports || !reporting) return null;
  const file = reports.get(path);
  if (!file || file.reports.length === 0) {
    return (
      <span className="shrink-0 text-[11px] text-ink-faint" title={CAVEAT}>
        no agent reported writing this
      </span>
    );
  }
  // The binding types a float as nullable; a timestamp the core wrote is never absent.
  const last = Math.max(...file.reports.map((report) => report.lastAt ?? 0));
  const writes = file.reports.reduce((sum, report) => sum + report.writes, 0);
  return (
    <span
      className="shrink-0 text-[11px] text-ink-faint"
      title={`${describeReports(file.reports)} ${CAVEAT}`}
    >
      reported by {[...new Set(file.reports.map(who))].join(", ")} · {writes}{" "}
      {writes === 1 ? "write" : "writes"} · {eventTime(last)}
    </span>
  );
}

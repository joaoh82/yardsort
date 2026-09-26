import { useMemo } from "react";
import type { FileReports, ObservedWrite, WriteReport } from "@/lib/ipc";
import { eventTime } from "@/features/activity/describe";
import { useProvenanceStore } from "@/stores/provenance";

/**
 * What the workspace's agents said they wrote, beside what git shows. A badge on a changed
 * file names the agent that reported writing it; a line above the list counts the files with
 * such a report and the files without one. Nothing here claims which lines came from whom:
 * git shows the sum of every change since the last commit, and no agent reports a line.
 */

/** The reports for each changed path, or `null` while nothing is known about this workspace. */
export function useReports(): Map<string, FileReports> | null {
  const provenance = useProvenanceStore((s) => s.provenance);
  // One map per answer; a selector that built it would rebuild it on every render.
  return useMemo(
    () => (provenance ? new Map(provenance.files.map((file) => [file.path, file])) : null),
    [provenance],
  );
}

/** Whether any agent run in this workspace was reporting at all. Without one, there is nothing
 *  to say about any file, and the list reads as it always did. */
export function useAnyReporting(): boolean {
  const provenance = useProvenanceStore((s) => s.provenance);
  return provenance?.runs.some((run) => run.capture !== null) ?? false;
}

export const who = (report: WriteReport) => report.harnessId ?? report.producer;

/** The words for one file's reports, oldest run first. */
export function describeReports(reports: WriteReport[]): string {
  return reports
    .map((report) => {
      const times = report.writes === 1 ? "once" : `${report.writes} times`;
      return `${who(report)} reported writing this file ${times}, last at ${eventTime(report.lastAt ?? 0)} (${report.producer}/${report.method}).`;
    })
    .join(" ");
}

export const CAVEAT =
  "Git shows every change since the last commit; which of those lines came from that report is not known.";

/** The agents an observation names: one, or "2 agents" when two were running commands at once. */
export function whoObserved(observed: ObservedWrite): string {
  const names = [...new Set(observed.matches.map((m) => m.harnessId ?? "an agent"))];
  return names.length === 1 ? names[0]! : `${names.length} agents`;
}

/**
 * The words for an observation. It says what Yardsort saw — the time on the file, the tool
 * that was running — and what it did not: who wrote the file.
 */
export function describeObserved(observed: ObservedWrite): string {
  const at = observed.at ?? 0;
  const during = observed.matches
    .map((m) => {
      const who = m.harnessId ?? "an agent";
      const tool = m.tool ? `was running ${m.tool}` : "was running a tool";
      return `${who} ${tool} (${eventTime(m.from ?? 0)} to ${eventTime(m.to ?? 0)})`;
    })
    .join(" and ");
  return `This file was last written at ${eventTime(at)}, while ${during}. Yardsort read the time on the file, not who wrote it: you or a script could have written it in that window, and no agent reported it.`;
}

/** The tool an observation names, when every match agrees on one. */
export function observedTool(observed: ObservedWrite): string | null {
  const tools = [...new Set(observed.matches.map((m) => m.tool ?? ""))];
  return tools.length === 1 && tools[0] ? tools[0] : null;
}

import { useMemo } from "react";
import type { FileReports, WriteReport } from "@/lib/ipc";
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

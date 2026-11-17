import type { NotificationItem } from "@/lib/ipc";
import type { Grouping } from "@/stores/notifications";

/** Which rows the view shows. */
export type Filter = "all" | "needs" | "finished" | "earlier";

/** One headed run of rows in the view. */
export interface Section {
  id: string;
  title: string;
  /** What the heading says beside its count: when its rows leave, or what they are. */
  note: string;
  /** Rows under a project's name do not each repeat it. */
  namesProject: boolean;
  items: NotificationItem[];
}

/** Who the row is about, as its tab calls it. */
export function agentName(item: NotificationItem): string {
  return item.harnessId ?? "The agent";
}

/** "12 min", "1 h 5 min": how long an agent worked, to the minute. Under a minute is not said. */
export function workedWords(ms: number): string | null {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return null;
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** What a row is about: which mark it is drawn with, and where pressing it goes. */
export type About = "agent" | "workflow" | "test" | "schedule" | "trigger";

export function about(item: Pick<NotificationItem, "kind">): About {
  return item.kind.slice(0, item.kind.indexOf(".")) as About;
}

/** What happened, in the row's words. */
export function what(item: NotificationItem): string {
  const agent = agentName(item);
  const waits = item.part === "needs";
  switch (item.kind) {
    case "agent.asks":
      return item.part === "needs"
        ? `${agent} asks for a permission`
        : `${agent} asked for a permission`;
    case "agent.finished": {
      const worked = item.workedMs === null ? null : workedWords(item.workedMs);
      return worked ? `${agent} finished after ${worked}` : `${agent} finished`;
    }
    case "agent.failed":
      return `${agent}'s turn failed`;
    case "agent.stopped":
      return `${agent} stopped with an error`;
    case "workflow.approval":
      return waits ? "Workflow waits for your approval" : "Workflow waited for an approval";
    case "workflow.question":
      return waits ? "Workflow asks you a question" : "Workflow asked a question";
    case "workflow.succeeded":
      return "Workflow finished";
    case "workflow.failed":
      return "Workflow failed";
    case "test.waits":
      return waits
        ? "Smoke test waits for you to approve its file"
        : "Smoke test waited for its file to be approved";
    case "schedule.refused":
      return waits ? "Schedule could not start its run" : "Schedule could not start a run";
    case "schedule.noWorkspace":
      return waits ? "Schedule has no workspace to run in" : "Schedule had no workspace to run in";
    case "trigger.refused":
      return waits ? "Trigger could not start its run" : "Trigger could not start a run";
    case "trigger.capped":
      return waits ? "Trigger reached its limit of runs" : "Trigger had reached its limit of runs";
    case "trigger.noWorkspace":
      return waits ? "Trigger has no workspace to run in" : "Trigger had no workspace to run in";
  }
}

/** What pressing the row opens, for its label and its hint. */
export function opens(item: NotificationItem): string {
  switch (about(item)) {
    case "agent":
      return item.terminalId ? "the agent's terminal" : "its workspace";
    case "workflow":
      return "the run";
    case "test":
      return "its workspace, where the file is approved";
    case "schedule":
      return "Schedules";
    case "trigger":
      return "Triggers";
  }
}

/** The short code-like fact beside the words: the command asked for, or the exit code. */
export function chip(item: NotificationItem): string | null {
  if (item.kind === "agent.asks") return item.part === "needs" ? item.command : null;
  if (item.kind === "agent.stopped" && item.exitCode !== null) return `exit ${item.exitCode}`;
  return null;
}

/** Whether a row went badly: drawn red rather than amber. */
export function wentBadly(item: NotificationItem): boolean {
  return (
    item.kind === "agent.failed" || item.kind === "agent.stopped" || item.kind === "workflow.failed"
  );
}

/** The name a row leads with: its workspace, or for a schedule or a trigger that has none, its
 *  own name. */
export function leadName(item: NotificationItem): string {
  return item.workspaceId !== null && item.workspaceName ? item.workspaceName : item.title;
}

/** Beside the lead name, after the branch: what the row is about within its workspace. A
 *  schedule or a trigger with no workspace leads with its own name, and says here which
 *  workspace it lost. */
export function detailName(item: NotificationItem): string | null {
  if (item.workspaceId !== null && item.workspaceName) return item.title || null;
  return item.workspaceName ? `was in ${item.workspaceName}` : null;
}

const newestFirst = (a: NotificationItem, b: NotificationItem) => b.at - a.at;
const lastLeftFirst = (a: NotificationItem, b: NotificationItem) =>
  (b.settledAt ?? 0) - (a.settledAt ?? 0);

function countWords(items: NotificationItem[]): string {
  const needs = items.filter((item) => item.part === "needs").length;
  const finished = items.filter((item) => item.part === "finished").length;
  const said = [];
  if (needs > 0) said.push(`${needs} ${needs === 1 ? "needs" : "need"} you`);
  if (finished > 0) said.push(`${finished} finished`);
  return said.join(" · ");
}

/** The view's sections: by what a row is, or by its project. Earlier is always its own, last.
 *  A section with nothing in it is left out. */
export function sections(items: NotificationItem[], grouping: Grouping, filter: Filter): Section[] {
  const shown = items.filter((item) => filter === "all" || item.part === filter);
  const waiting = shown.filter((item) => item.part !== "earlier");
  const earlier: Section = {
    id: "earlier",
    title: "Earlier",
    note: "Kept for a week",
    namesProject: true,
    items: shown.filter((item) => item.part === "earlier").sort(lastLeftFirst),
  };
  const out: Section[] = [];
  if (grouping === "kind") {
    out.push({
      id: "needs",
      title: "Needs you",
      note: "Leaves when it is answered or put right",
      namesProject: true,
      items: waiting.filter((item) => item.part === "needs").sort(newestFirst),
    });
    out.push({
      id: "finished",
      title: "Finished, not seen",
      note: "Leaves when you look at it",
      namesProject: true,
      items: waiting.filter((item) => item.part === "finished").sort(newestFirst),
    });
  } else {
    const byProject = new Map<string, NotificationItem[]>();
    for (const item of waiting) {
      const list = byProject.get(item.projectId);
      if (list) list.push(item);
      else byProject.set(item.projectId, [item]);
    }
    const projects = [...byProject.entries()].sort(([, a], [, b]) =>
      (a[0]?.projectName ?? "").localeCompare(b[0]?.projectName ?? ""),
    );
    for (const [projectId, rows] of projects) {
      // What needs an answer comes before what only finished.
      const ordered = [...rows].sort((a, b) =>
        a.part === b.part ? newestFirst(a, b) : a.part === "needs" ? -1 : 1,
      );
      out.push({
        id: `project:${projectId}`,
        title: rows[0]?.projectName ?? "",
        note: countWords(rows),
        namesProject: false,
        items: ordered,
      });
    }
  }
  out.push(earlier);
  return out.filter((section) => section.items.length > 0);
}

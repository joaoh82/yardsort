import type {
  ScheduleTarget,
  TriggerEvent,
  TriggerFilters,
  TriggerFire,
  TriggerItem,
} from "@/lib/ipc";

/** What a trigger's runs are counted for, and so which events can share one. */
export type Subject = "session" | "pull request" | "workspace";

/** Every event, in the dialog's order, with its group and its label there. */
export const EVENTS: { id: TriggerEvent; group: string; label: string }[] = [
  { id: "workspace.done", group: "The agent", label: "Says its work is done" },
  { id: "agent.settled", group: "The agent", label: "Finishes a turn" },
  { id: "agent.exited", group: "The agent", label: "Its program ends" },
  { id: "test.finished", group: "A run", label: "A smoke test run ends" },
  { id: "workflow.finished", group: "A run", label: "A workflow run ends" },
  { id: "pr.opened", group: "The pull request", label: "Opens" },
  { id: "pr.review", group: "The pull request", label: "A review comes in" },
  { id: "pr.comment", group: "The pull request", label: "Someone comments or replies" },
  { id: "pr.checks_failed", group: "The pull request", label: "Its checks fail" },
  { id: "pr.checks_passed", group: "The pull request", label: "Its checks pass" },
  { id: "pr.conflicted", group: "The pull request", label: "It conflicts with its base" },
  { id: "pr.merged", group: "The pull request", label: "It is merged" },
  { id: "pr.closed", group: "The pull request", label: "It is closed without merging" },
  { id: "custom", group: "Something you name", label: "A custom event" },
];

/** What a custom event's name is written after: `custom.migration-written`. */
export const CUSTOM_PREFIX = "custom.";

/** The longest a custom event's name may be: the core's own rule. */
const MOST_CUSTOM_NAME = 40;

/** The names typed for a custom event, as the core keeps them: split at commas and spaces,
 *  without `custom.` in front, each once. */
export function customNames(text: string): string[] {
  const names = text
    .split(/[\s,]+/)
    .map((name) => (name.startsWith(CUSTOM_PREFIX) ? name.slice(CUSTOM_PREFIX.length) : name))
    .filter(Boolean);
  return [...new Set(names)];
}

/** Why `name` cannot be a custom event's name, when it cannot: lowercase letters, digits, `-`
 *  and `_`, starting with a letter or a digit. The core says the same when it is saved. */
export function customNameProblem(name: string): string | null {
  if (name.length > MOST_CUSTOM_NAME) {
    return `${name} is longer than ${MOST_CUSTOM_NAME} characters.`;
  }
  if (/^[a-z0-9][a-z0-9_-]*$/.test(name)) return null;
  const lower = name.toLowerCase();
  const hint = lower !== name && customNameProblem(lower) === null ? ` ${lower} would do.` : "";
  return `${name} cannot be an event's name: lowercase letters, digits, - and _, starting with a letter or a digit.${hint}`;
}

export function subjectOf(event: TriggerEvent): Subject {
  if (event === "agent.settled" || event === "agent.exited") return "session";
  if (event.startsWith("pr.")) return "pull request";
  return "workspace";
}

/** The quiet time a trigger on these events starts with: two minutes for reviews and comments,
 *  which come in bursts, and none for the rest. The core's own rule. */
export function defaultQuietMs(events: TriggerEvent[]): number {
  return events.some((event) => event === "pr.review" || event === "pr.comment") ? 120_000 : 0;
}

/** A length of time in words: `2 minutes`, `30 seconds`, `1 hour`. */
export function durationWords(ms: number): string {
  const unit = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;
  if (ms % 3_600_000 === 0) return unit(ms / 3_600_000, "hour");
  if (ms % 60_000 === 0) return unit(ms / 60_000, "minute");
  return unit(Math.round(ms / 1000), "second");
}

/** Where a trigger stands, in a word or two, for its badge. One that listens to an agent that
 *  has ended has no one to listen to. */
export function standingWords(
  item: Pick<TriggerItem, "standing" | "sessionId" | "sessionRunning">,
) {
  switch (item.standing) {
    case "listening":
      return item.sessionId && !item.sessionRunning ? "ended" : "on";
    case "running":
      return "running";
    case "awaiting_approval":
      return "waiting for approval";
    case "waiting":
      return "waiting";
    case "capped":
      return "capped";
    case "refused":
      return "refused";
    case "no_workspace":
      return "no workspace";
    case "off":
      return "off";
  }
}

/** The badge's colours: amber for what needs the user, red for what was stopped or refused. */
export function standingColour(
  item: Pick<TriggerItem, "standing" | "sessionId" | "sessionRunning">,
) {
  switch (standingWords(item)) {
    case "on":
      return "bg-green-500/20 text-green-400";
    case "running":
    case "waiting":
    case "waiting for approval":
      return "bg-amber-500/20 text-amber-300";
    case "capped":
    case "refused":
    case "no workspace":
      return "bg-red-500/20 text-red-400";
    default:
      return "bg-raised text-ink-faint";
  }
}

type Line = Pick<TriggerFire, "outcome" | "result" | "awaitingApproval">;

/** What became of a fire, in a word or two: the run's ending once it has one. */
export function fireWords(fire: Line): string {
  if (fire.outcome !== "started") return fire.outcome;
  if (fire.result) return fire.result.replace(/_/g, " ");
  return fire.awaitingApproval ? "waiting for approval" : "running";
}

/** A line's colours, by what it came to. */
export function fireColour(fire: Line): string {
  switch (fireWords(fire)) {
    case "passed":
    case "succeeded":
      return "bg-green-500/20 text-green-400";
    case "failed":
    case "errored":
    case "refused":
    case "capped":
      return "bg-red-500/20 text-red-400";
    case "running":
    case "waiting":
    case "gathering":
    case "waiting for approval":
      return "bg-amber-500/20 text-amber-300";
    default:
      return "bg-raised text-ink-faint";
  }
}

/** What a trigger runs, in words: `workflow address-review`, `smoke test login`. */
export function targetWords(item: { target: ScheduleTarget; targetId: string }): string {
  return `${item.target === "test" ? "smoke test" : "workflow"} ${item.targetId}`;
}

/** What the dialog is putting together, for [`sentence`]. */
export interface Asking {
  events: TriggerEvent[];
  filters: TriggerFilters;
  target: ScheduleTarget;
  targetId: string;
  inputs: Record<string, string>;
  quietMs: number;
  cap: number;
  /** It listens to one agent session only. */
  oneAgent: boolean;
}

/**
 * What a trigger will do, in a sentence or two, as the dialog shows it: _When someone other than
 * you reviews or comments on this workspace's pull request, 2 minutes after the last one, tell
 * the agent to address it. At most 3 times per pull request._ Empty until there is an event and
 * something to run.
 */
export function sentence(asking: Asking): string {
  const { events, filters } = asking;
  if (events.length === 0 || !asking.targetId) return "";
  const agent = asking.oneAgent ? "this agent" : "an agent";
  const pr = "this workspace's pull request";
  const who = filters.from === "anyone" ? "anyone" : "someone other than you";
  const has = (event: TriggerEvent) => events.includes(event);

  const whens: string[] = [];
  if (has("workspace.done")) whens.push(`${agent} says its work is done`);
  if (has("agent.settled")) {
    const only = [filters.newCommits && "with new commits", filters.clean && "nothing uncommitted"]
      .filter(Boolean)
      .join(" and ");
    whens.push(`${agent} finishes a turn${only ? `, ${only}` : ""}`);
  }
  if (has("agent.exited")) {
    whens.push(filters.exit === "failed" ? `${agent}'s program fails` : `${agent}'s program ends`);
  }
  if (has("test.finished")) {
    const which = filters.test ? `the smoke test ${filters.test}` : "a smoke test";
    const how =
      filters.verdict === "passed" ? "passes" : filters.verdict === "failed" ? "fails" : "ends";
    whens.push(`${which} ${how} here`);
  }
  if (has("workflow.finished")) {
    const which = filters.workflow ? `the workflow ${filters.workflow}` : "a workflow run";
    const how =
      filters.outcome === "succeeded"
        ? "succeeds"
        : filters.outcome === "failed"
          ? "fails"
          : "ends";
    whens.push(`${which} ${how} here`);
  }
  if (has("pr.opened")) {
    whens.push(filters.drafts ? `${pr} opens, drafts too` : `${pr} opens or is marked ready`);
  }
  const reviews = has("pr.review");
  const comments = has("pr.comment");
  if (reviews || comments) {
    const bots = filters.noBots ? ", leaving out bots" : "";
    const states = filters.states?.length
      ? ` (${filters.states.map((state) => state.replace("_", " ")).join(" or ")})`
      : "";
    const verb =
      reviews && comments ? "reviews or comments on" : reviews ? "reviews" : "comments on";
    whens.push(`${who} ${verb} ${pr}${reviews ? states : ""}${bots}`);
  }
  if (has("pr.checks_failed")) whens.push(`the checks on ${pr} fail`);
  if (has("pr.checks_passed")) whens.push(`the checks on ${pr} pass`);
  if (has("pr.conflicted")) whens.push(`${pr} conflicts with its base`);
  if (has("pr.merged")) whens.push(`${pr} is merged`);
  if (has("pr.closed")) whens.push(`${pr} is closed without merging`);
  if (has("custom")) {
    const names = (filters.names ?? []).map((name) => `${CUSTOM_PREFIX}${name}`);
    whens.push(
      names.length === 0
        ? "a custom event is raised here"
        : `${names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} or ${names.at(-1)}`} is raised here`,
    );
  }

  const when =
    whens.length === 1 ? whens[0] : `${whens.slice(0, -1).join(", ")} or ${whens.at(-1)}`;
  const after = asking.quietMs > 0 ? `, ${durationWords(asking.quietMs)} after the last one` : "";
  const subject = subjectOf(events[0] ?? "workspace.done");
  const per = subject === "session" ? "agent session" : subject;
  const times = asking.cap === 1 ? "once" : `${asking.cap} times`;
  return `When ${when}${after}, ${doing(asking)}. At most ${times} per ${per}.`;
}

/** What running the target does, in words: the built-in workflows say what they are for. */
function doing({ target, targetId, inputs }: Asking): string {
  if (target === "test") return `run the smoke test ${targetId}`;
  const reviewer = inputs.reviewer ? `${inputs.reviewer}` : "a second agent";
  switch (targetId) {
    case "address-review":
      return "tell the agent to address it";
    case "fix-checks":
      return "tell the agent which checks failed, to fix them";
    case "review-branch":
      return `have ${reviewer} review the branch, and tell the agent what it found`;
    case "code-review":
      return `have ${reviewer} review the pull request on GitHub`;
    case "smoke-then-tell":
      return inputs.suite
        ? `run the smoke test ${inputs.suite} and tell the agent how it went`
        : "run a smoke test and tell the agent how it went";
    default:
      return `run the workflow ${targetId}`;
  }
}

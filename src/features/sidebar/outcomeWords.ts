import type { AgentOutcomes } from "@/lib/ipc";

/**
 * One agent's history, in words that never overstate a small sample: nothing but counts until
 * the core says there are enough outcomes (`enough`), and always the size of the sample.
 */
export function describeHistory(agent: AgentOutcomes): string {
  if (agent.known === 0) {
    return `${agent.attempts} ${agent.attempts === 1 ? "attempt" : "attempts"}, none judged yet`;
  }
  if (!agent.enough) {
    return `too few to say yet — ${agent.known} ${agent.known === 1 ? "outcome" : "outcomes"} so far`;
  }
  const merged = agent.keptByMerge > 0 ? ` (${agent.keptByMerge} by merge)` : "";
  return `kept ${agent.kept} of ${agent.known}${merged} · partly ${agent.partly} · discarded ${agent.discarded}`;
}

import { Graph, layout } from "@dagrejs/dagre";

export const NODE_WIDTH = 226;
export const NODE_HEIGHT = 62;

export interface Placed {
  id: string;
  /** Top-left corner. */
  x: number;
  y: number;
}

export interface Link {
  id: string;
  /** The step that must succeed first. */
  source: string;
  target: string;
}

/**
 * Where each step goes in the chart: top to bottom in the order the graph allows, a step below
 * every step it needs, and steps that need the same ones side by side. `needs` naming a step
 * that is not there draws nothing: the editor says what is wrong with the file.
 *
 * A route to a later step is drawn too, when `needs` does not already. A route back to an
 * earlier step is drawn, and is not given to the layout, which cannot place a cycle.
 */
export function layoutSteps(steps: { id: string; needs: string[]; routes?: { to: string }[] }[]): {
  nodes: Placed[];
  links: Link[];
} {
  const graph = new Graph();
  graph.setGraph({ rankdir: "TB", nodesep: 28, ranksep: 44, marginx: 16, marginy: 16 });
  graph.setDefaultEdgeLabel(() => ({}));
  const ids = new Set(steps.map((step) => step.id));
  const before = ancestorIds(steps);
  for (const step of steps) graph.setNode(step.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  const links: Link[] = [];
  const seen = new Set<string>();
  function add(source: string, target: string, laidOut: boolean) {
    const id = `${source}->${target}`;
    if (seen.has(id)) return;
    seen.add(id);
    if (laidOut) graph.setEdge(source, target);
    links.push({ id, source, target });
  }
  for (const step of steps) {
    for (const need of step.needs) {
      if (!ids.has(need) || need === step.id) continue;
      add(need, step.id, true);
    }
    for (const route of step.routes ?? []) {
      if (route.to === "stop" || !ids.has(route.to) || route.to === step.id) continue;
      const forward = before.get(route.to)?.has(step.id) ?? false;
      add(step.id, route.to, forward);
    }
  }
  layout(graph);
  const nodes = steps.map((step) => {
    const placed = graph.node(step.id) as { x: number; y: number };
    return { id: step.id, x: placed.x - NODE_WIDTH / 2, y: placed.y - NODE_HEIGHT / 2 };
  });
  return { nodes, links };
}

/** What each step needs, including through other steps. */
function ancestorIds(steps: { id: string; needs: string[] }[]): Map<string, Set<string>> {
  const needs = new Map(steps.map((step) => [step.id, step.needs]));
  const found = new Map<string, Set<string>>();
  function visit(id: string, stack: Set<string>): Set<string> {
    const cached = found.get(id);
    if (cached) return cached;
    const set = new Set<string>();
    if (stack.has(id)) return set;
    stack.add(id);
    for (const need of needs.get(id) ?? []) {
      set.add(need);
      for (const older of visit(need, stack)) set.add(older);
    }
    stack.delete(id);
    found.set(id, set);
    return set;
  }
  for (const step of steps) visit(step.id, new Set());
  return found;
}

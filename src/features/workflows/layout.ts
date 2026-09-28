import { Graph, layout } from "@dagrejs/dagre";

export const NODE_WIDTH = 210;
export const NODE_HEIGHT = 58;

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
 */
export function layoutSteps(steps: { id: string; needs: string[] }[]): {
  nodes: Placed[];
  links: Link[];
} {
  const graph = new Graph();
  graph.setGraph({ rankdir: "TB", nodesep: 28, ranksep: 44, marginx: 16, marginy: 16 });
  graph.setDefaultEdgeLabel(() => ({}));
  const ids = new Set(steps.map((step) => step.id));
  for (const step of steps) graph.setNode(step.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  const links: Link[] = [];
  for (const step of steps) {
    for (const need of step.needs) {
      if (!ids.has(need) || need === step.id) continue;
      graph.setEdge(need, step.id);
      links.push({ id: `${need}->${step.id}`, source: need, target: step.id });
    }
  }
  layout(graph);
  const nodes = steps.map((step) => {
    const placed = graph.node(step.id) as { x: number; y: number };
    return { id: step.id, x: placed.x - NODE_WIDTH / 2, y: placed.y - NODE_HEIGHT / 2 };
  });
  return { nodes, links };
}

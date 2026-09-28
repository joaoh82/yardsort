import {
  Background,
  Controls,
  Handle,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useMemo } from "react";
import type { Step, WorkflowStepRun } from "@/lib/ipc";
import { statusColour } from "@/stores/workflows";
import { layoutSteps, NODE_HEIGHT, NODE_WIDTH } from "./layout";
import { ACTION_WORDS } from "./words";

type StepNode = Node<{ step: Step; status: string | null; note: string | null }, "step">;

function StepBox({ data }: NodeProps<StepNode>) {
  const { step, status, note } = data;
  return (
    <div
      title={note ?? undefined}
      className="flex flex-col justify-center gap-0.5 rounded-md border border-line bg-surface px-3 text-left shadow-sm"
      style={{ width: NODE_WIDTH, height: NODE_HEIGHT }}
    >
      <Handle type="target" position={Position.Top} className="!bg-line" isConnectable={false} />
      <div className="flex items-center justify-between gap-2">
        <span className="truncate text-[12px] font-medium text-ink">
          {ACTION_WORDS[step.action]}
        </span>
        {status && (
          <span className={`shrink-0 rounded-full px-1.5 text-[10px] ${statusColour(status)}`}>
            {status}
          </span>
        )}
      </div>
      <span className="truncate font-mono text-[11px] text-ink-faint">{step.id}</span>
      <Handle type="source" position={Position.Bottom} className="!bg-line" isConnectable={false} />
    </div>
  );
}

const nodeTypes = { step: StepBox };

/**
 * The workflow's steps as a graph, top to bottom. A view of the file, never an editor of it:
 * the YAML is what runs. With a run's steps, each node shows how that step went.
 */
export function WorkflowChart({ steps, run }: { steps: Step[]; run?: WorkflowStepRun[] }) {
  const { nodes, edges } = useMemo(() => {
    const placed = layoutSteps(steps);
    const byId = new Map(run?.map((s) => [s.stepId, s]));
    const nodes: StepNode[] = placed.nodes.map((node) => {
      const step = steps.find((s) => s.id === node.id)!;
      const ran = byId.get(node.id);
      return {
        id: node.id,
        type: "step",
        position: { x: node.x, y: node.y },
        data: { step, status: ran?.status ?? null, note: ran?.note ?? null },
        draggable: false,
        connectable: false,
        selectable: false,
      };
    });
    const edges: Edge[] = placed.links.map((link) => ({
      id: link.id,
      source: link.source,
      target: link.target,
      animated: byId.get(link.target)?.status === "waiting",
    }));
    return { nodes, edges };
  }, [steps, run]);

  return (
    <div className="h-full w-full" aria-label="Steps" role="figure">
      <ReactFlow
        key={steps.map((s) => s.id).join(",")}
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
        colorMode="system"
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        minZoom={0.3}
      >
        <Background gap={16} size={1} />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

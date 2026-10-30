import { errorMessage, ipc } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";

/**
 * Open the composer here with the handoff packet as the first message, to read and edit — with
 * `harness` picked, when it was one agent's button that asked.
 */
export async function handOff(workspaceId: string, harness?: string) {
  const projects = useProjectsStore.getState();
  const workspace = projects.projects
    .flatMap((project) => project.workspaces)
    .find((candidate) => candidate.id === workspaceId);
  if (!workspace) return;
  try {
    const packet = await ipc.workspaceHandoff(workspaceId);
    useProjectsStore.getState().composeIn(workspace, packet.text, harness);
  } catch (error) {
    useProjectsStore.setState({ error: errorMessage(error) });
  }
}

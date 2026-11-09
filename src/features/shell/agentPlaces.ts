/** Running agents named for a person: what each is called, and where it is. */
export function agentPlaces(
  agents: string[],
  tabs: { id: string; title: string; workspaceId: string }[],
  projects: { name: string; workspaces: { id: string; name: string }[] }[],
  /** The Floor Manager's session: an agent with no tab and no workspace, called by its name. */
  floorManager: string | null = null,
) {
  return agents.map((id) => {
    if (id === floorManager) return { id, title: "Floor Manager", place: null };
    const tab = tabs.find((t) => t.id === id);
    const project = projects.find((p) => p.workspaces.some((w) => w.id === tab?.workspaceId));
    const workspace = project?.workspaces.find((w) => w.id === tab?.workspaceId);
    return {
      id,
      title: tab?.title ?? "agent",
      place: project && workspace ? `${project.name} / ${workspace.name}` : null,
    };
  });
}

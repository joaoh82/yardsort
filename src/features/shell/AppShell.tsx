import { useEffect } from "react";
import { Group, Panel, Separator, useDefaultLayout, usePanelRef } from "react-resizable-panels";
import { ChangesPanel } from "@/features/changes/ChangesPanel";
import { composeInCurrentProject, openProjectFromDisk } from "@/features/sidebar/actions";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { UpdateDialog } from "@/features/updates/UpdateDialog";
import { useUpdateChecks } from "@/features/updates/useUpdateChecks";
import { Sidebar } from "@/features/sidebar/Sidebar";
import { QuitDialog } from "@/features/shell/QuitDialog";
import { WorkspacePanel } from "@/features/workspace/WorkspacePanel";
import { hasCore } from "@/lib/ipc";
import { isModKey, shortcutKey } from "@/lib/platform";
import { useAppStore } from "@/stores/app";
import { useHarnessStore } from "@/stores/harnesses";
import { useLayoutStore, type SidePanel } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { listenForQuitRequests } from "@/stores/quit";
import { useTerminalStore } from "@/stores/terminals";
import { useUpdatesStore } from "@/stores/updates";
import { StatusBar } from "./StatusBar";

const separatorClass =
  "w-px bg-line outline-none transition-colors hover:bg-accent focus-visible:bg-accent data-[separator=active]:bg-accent";

/** The three-panel frame: projects · work · files & changes. */
export function AppShell() {
  const leftRef = usePanelRef();
  const rightRef = usePanelRef();
  const collapsed = useLayoutStore((s) => s.collapsed);
  const toggle = useLayoutStore((s) => s.toggle);
  const setCollapsed = useLayoutStore((s) => s.setCollapsed);
  const settingsOpen = useLayoutStore((s) => s.settingsOpen);
  const updateOpen = useUpdatesStore((s) => s.open);
  useUpdateChecks();
  const setSettingsOpen = useLayoutStore((s) => s.setSettingsOpen);

  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: "yardsort.shell",
    storage: localStorage,
  });

  // Drive the panels from the store…
  useEffect(() => {
    for (const [side, ref] of [
      ["left", leftRef],
      ["right", rightRef],
    ] as const) {
      const panel = ref.current;
      if (!panel || panel.isCollapsed() === collapsed[side]) continue;
      if (collapsed[side]) panel.collapse();
      else panel.expand();
    }
  }, [collapsed, leftRef, rightRef]);

  // …and keep the store honest when the user collapses one by dragging.
  const syncFromPanel = (side: SidePanel) => () => {
    const panel = (side === "left" ? leftRef : rightRef).current;
    if (panel) setCollapsed(side, panel.isCollapsed());
  };

  useEffect(() => {
    void useAppStore.getState().load().catch(console.error);
    if (hasCore()) void useHarnessStore.getState().load();
    const listening = listenForQuitRequests();
    return () => void listening.then((stop) => stop());
  }, []);

  // Mod+B / Mod+Alt+B toggle the side panels, Mod+O opens a project, Mod+N composes a new
  // workspace, Mod+T and Mod+W open and
  // close terminal tabs in the selected workspace. Always behind Mod (see `isModKey`), so the
  // program in the terminal never loses a key.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isModKey(event)) return;
      const key = shortcutKey(event);
      const workspaceId = useProjectsStore.getState().selectedWorkspaceId;
      const terminals = useTerminalStore.getState();
      if (key === "b") toggle(event.altKey ? "right" : "left");
      else if (event.altKey) return;
      else if (key === "," || key === "<") setSettingsOpen(true);
      else if (key === "o") void openProjectFromDisk();
      else if (key === "n") composeInCurrentProject();
      else if (key === "t" && workspaceId) void terminals.open(workspaceId);
      else if (key === "w" && workspaceId) {
        const active = terminals.active[workspaceId];
        if (active) void terminals.close(active);
      } else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggle, setSettingsOpen]);

  return (
    <div className="flex h-full flex-col">
      <Group
        orientation="horizontal"
        className="min-h-0 flex-1"
        defaultLayout={defaultLayout}
        onLayoutChanged={onLayoutChanged}
      >
        <Panel
          id="left"
          panelRef={leftRef}
          defaultSize={260}
          minSize={200}
          maxSize={480}
          collapsible
          collapsedSize={0}
          onResize={syncFromPanel("left")}
        >
          <Sidebar />
        </Panel>
        <Separator className={separatorClass} />
        <Panel id="center" minSize={360}>
          <WorkspacePanel />
        </Panel>
        <Separator className={separatorClass} />
        <Panel
          id="right"
          panelRef={rightRef}
          defaultSize={340}
          minSize={240}
          maxSize={720}
          collapsible
          collapsedSize={0}
          onResize={syncFromPanel("right")}
        >
          <ChangesPanel />
        </Panel>
      </Group>
      <StatusBar />
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      {updateOpen && <UpdateDialog />}
      <QuitDialog />
    </div>
  );
}

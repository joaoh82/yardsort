import { useEffect } from "react";
import { Group, Panel, Separator, useDefaultLayout, usePanelRef } from "react-resizable-panels";
import { ChangesPanel } from "@/features/changes/ChangesPanel";
import { CommandPalette } from "@/features/keyboard/CommandPalette";
import { StartFromLinkDialog } from "@/features/sidebar/StartFromLinkDialog";
import { useAppShortcuts } from "@/features/keyboard/useAppShortcuts";
import { WelcomeTour } from "@/features/onboarding/WelcomeTour";
import { usePreferencesStore } from "@/stores/preferences";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { UpdateDialog } from "@/features/updates/UpdateDialog";
import { useUpdateChecks } from "@/features/updates/useUpdateChecks";
import { Sidebar } from "@/features/sidebar/Sidebar";
import { QuitDialog } from "@/features/shell/QuitDialog";
import { WorkspacePanel } from "@/features/workspace/WorkspacePanel";
import { hasCore } from "@/lib/ipc";
import { useAppStore } from "@/stores/app";
import { useHarnessStore } from "@/stores/harnesses";
import { useLayoutStore, type SidePanel } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { listenForQuitRequests } from "@/stores/quit";
import { listenForWorkflowRuns } from "@/stores/workflows";
import { useUpdatesStore } from "@/stores/updates";
import { StatusBar } from "./StatusBar";

const separatorClass =
  "w-px bg-line outline-none transition-colors hover:bg-accent focus-visible:bg-accent data-[separator=active]:bg-accent";

/** The three-panel frame: projects · work · files & changes. */
export function AppShell() {
  const leftRef = usePanelRef();
  const rightRef = usePanelRef();
  const collapsed = useLayoutStore((s) => s.collapsed);
  useAppShortcuts();
  const paletteOpen = useLayoutStore((s) => s.paletteOpen);
  const linkOpen = useProjectsStore((s) => s.linkOpen);
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
    void usePreferencesStore.getState().load();
    if (hasCore()) void useHarnessStore.getState().load();
    const listening = listenForQuitRequests();
    const runs = listenForWorkflowRuns();
    return () => {
      void listening.then((stop) => stop());
      void runs.then((stop) => stop());
    };
  }, []);

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
          <div data-navigation="Projects" tabIndex={-1} className="h-full">
            <Sidebar />
          </div>
        </Panel>
        <Separator className={separatorClass} />
        <Panel id="center" minSize={360}>
          <div data-navigation="Workspace" tabIndex={-1} className="h-full">
            <WorkspacePanel />
          </div>
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
          <div data-navigation="Changes" tabIndex={-1} className="h-full">
            <ChangesPanel />
          </div>
        </Panel>
      </Group>
      <StatusBar />
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      {updateOpen && <UpdateDialog />}
      {paletteOpen && <CommandPalette />}
      {linkOpen && (
        <StartFromLinkDialog onClose={() => useProjectsStore.getState().openLink(false)} />
      )}
      <WelcomeTour />
      <QuitDialog />
    </div>
  );
}

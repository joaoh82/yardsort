import { useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage, ipc } from "@/lib/ipc";
import { useChangesStore } from "@/stores/changes";
import { ContextMenu } from "@/features/sidebar/ContextMenu";

/** Share the same native-file action across Files and Changes. */
export function useFileContextMenu(workspaceId: string | null) {
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    path: string;
    workspaceId: string;
  } | null>(null);
  return {
    onContextMenu(event: React.MouseEvent, path: string) {
      event.preventDefault();
      event.stopPropagation();
      if (workspaceId) setMenu({ x: event.clientX, y: event.clientY, path, workspaceId });
    },
    menu:
      menu && workspaceId && menu.workspaceId === workspaceId
        ? createPortal(
            <ContextMenu
              at={menu}
              onClose={() => setMenu(null)}
              items={[
                {
                  label: "Show in file explorer",
                  onSelect: () => {
                    void ipc.workspaceRevealFile(workspaceId, menu.path).catch((error) => {
                      if (useChangesStore.getState().workspaceId === workspaceId) {
                        useChangesStore.setState({ error: errorMessage(error) });
                      }
                    });
                  },
                },
              ]}
            />,
            document.body,
          )
        : null,
  };
}

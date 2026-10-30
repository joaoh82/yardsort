import { create } from "zustand";
import { hasCore, ipc, type RemoteRequest } from "@/lib/ipc";
import { native } from "@/lib/native";

interface RemoteState {
  /** Devices asking to be let in that the window knows of, until answered or lapsed. */
  asking: RemoteRequest[];
  /** The relay told of one: show it, and say so on the desktop if nobody is looking. */
  told: (request: RemoteRequest) => void;
  /** Settings → Remote read who is waiting now: that list stands. */
  settled: (requests: RemoteRequest[]) => void;
}

/** When each request lapses, so it goes from the status bar on its own. */
const lapses = new Map<string, ReturnType<typeof setTimeout>>();

function lapseLater(request: RemoteRequest) {
  clearTimeout(lapses.get(request.grantId));
  lapses.set(
    request.grantId,
    setTimeout(() => {
      lapses.delete(request.grantId);
      useRemoteStore.setState((state) => ({
        asking: state.asking.filter((r) => r.grantId !== request.grantId),
      }));
    }, request.expiresInMs),
  );
}

/**
 * Devices asking to pair with this computer, which the window shows wherever it is. Answering
 * happens in Settings → Remote, and only there.
 */
export const useRemoteStore = create<RemoteState>((set, get) => ({
  asking: [],

  told(request) {
    if (get().asking.some((r) => r.grantId === request.grantId)) return;
    set((state) => ({ asking: [...state.asking, request] }));
    lapseLater(request);
    if (!document.hasFocus()) {
      void native
        .notify(`${request.deviceName} asks to reach this computer`, "Answer in Settings → Remote.")
        .catch(console.error);
    }
  },

  settled(requests) {
    for (const [grantId, timer] of lapses) {
      if (!requests.some((r) => r.grantId === grantId)) {
        clearTimeout(timer);
        lapses.delete(grantId);
      }
    }
    requests.forEach(lapseLater);
    set({ asking: requests });
  },
}));

/** Listen for devices asking. Mounted once, with the rest of the shell. */
export function listenForPairingRequests(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  return ipc.onRemotePairingRequested((request) => useRemoteStore.getState().told(request));
}

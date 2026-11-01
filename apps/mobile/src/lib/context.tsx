// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { createContext, type ReactNode, useContext, useSyncExternalStore } from "react";

import type { Accounts, Browser, Snapshot } from "./accounts";
import type { Incoming } from "./pairing";

interface App {
  accounts: Accounts;
  browser: Browser;
  /** What to call this phone until the person names it. */
  deviceName: string;
  /** A pairing link on its way to being used. */
  incoming: Incoming;
}

const AppContext = createContext<App | null>(null);

export function AppProvider({ value, children }: { value: App; children: ReactNode }) {
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): App {
  const app = useContext(AppContext);
  if (!app) throw new Error("useApp needs an AppProvider.");
  return app;
}

/** Where the profiles stand, re-rendering when that changes. */
export function useSnapshot(): Snapshot {
  const { accounts } = useApp();
  return useSyncExternalStore(accounts.subscribe, accounts.getSnapshot);
}

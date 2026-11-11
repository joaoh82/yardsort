// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { jest } from "@jest/globals";

jest.mock(
  "react-native-safe-area-context",
  () =>
    jest.requireActual<{ default: unknown }>("react-native-safe-area-context/jest/mock").default,
);

// A WebView, as far as a screen sees it: a view a test fires the page's messages at, and that keeps
// what it was told. The page itself has its own tests, in a real browser.
jest.mock("react-native-webview", () => {
  const { createElement, useImperativeHandle } =
    jest.requireActual<typeof import("react")>("react");
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  const told: string[] = [];
  return {
    told,
    WebView: ({ ref, ...props }: { ref?: unknown }) => {
      useImperativeHandle(ref as never, () => ({ postMessage: (text: string) => told.push(text) }));
      return createElement(View, props);
    },
  };
});

// The camera, as far as a screen sees it: permission given, and a view a test fires scans at.
jest.mock("expo-camera", () => {
  const { createElement } = jest.requireActual<typeof import("react")>("react");
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  return {
    useCameraPermissions: () => [{ granted: true, canAskAgain: true }, async () => {}],
    CameraView: (props: object) => createElement(View, { testID: "camera", ...props }),
  };
});

// The native view that hears a hardware keyboard: a test presses a key with its `onKey`.
jest.mock("../../modules/hardware-keys", () => {
  const { createElement } = jest.requireActual<typeof import("react")>("react");
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  return {
    HardwareKeys: (props: object) => createElement(View, { testID: "hardware-keys", ...props }),
  };
});

// The network, as the platform tells of it: a test says what it now is with `networkIs`.
const mockNetworkListeners = new Set<(state: object) => void>();
jest.mock("expo-network", () => ({
  addNetworkStateListener: (listener: (state: object) => void) => {
    mockNetworkListeners.add(listener);
    return { remove: () => mockNetworkListeners.delete(listener) };
  },
}));
(globalThis as { networkIs?: (state: object) => void }).networkIs = (state) => {
  for (const listener of mockNetworkListeners) listener(state);
};

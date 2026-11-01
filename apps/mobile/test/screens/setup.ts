// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { jest } from "@jest/globals";

jest.mock(
  "react-native-safe-area-context",
  () =>
    jest.requireActual<{ default: unknown }>("react-native-safe-area-context/jest/mock").default,
);

// The camera, as far as a screen sees it: permission given, and a view a test fires scans at.
jest.mock("expo-camera", () => {
  const { createElement } = jest.requireActual<typeof import("react")>("react");
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  return {
    useCameraPermissions: () => [{ granted: true, canAskAgain: true }, async () => {}],
    CameraView: (props: object) => createElement(View, { testID: "camera", ...props }),
  };
});

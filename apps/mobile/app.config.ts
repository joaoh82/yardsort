// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The app's identity. A development build is its own app on the phone, beside a release one:
 * its own name and bundle identifier, the same `yardsort` scheme (31 §5). The system's sign-in
 * session hands its answer to the app that opened it, so two apps with one scheme do not mix.
 * `APP_VARIANT=release` builds the release identity, which `just mobile-testflight` does.
 */

import type { ConfigContext, ExpoConfig } from "expo/config";

const release = process.env.APP_VARIANT === "release";
const identifier = release ? "dev.yardsort.mobile" : "dev.yardsort.mobile.development";

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: release ? "Yardsort" : "Yardsort Dev",
  slug: "yardsort",
  version: "0.1.0",
  scheme: "yardsort",
  orientation: "default",
  userInterfaceStyle: "automatic",
  icon: "./assets/icon.png",
  ios: {
    bundleIdentifier: identifier,
    // TestFlight refuses a build number it has seen; `just mobile-testflight` gives each a new one.
    buildNumber: process.env.YARDSORT_BUILD_NUMBER ?? "1",
    ...(process.env.APPLE_TEAM_ID ? { appleTeamId: process.env.APPLE_TEAM_ID } : {}),
    supportsTablet: true,
    config: { usesNonExemptEncryption: false },
  },
  android: {
    package: identifier,
    adaptiveIcon: { foregroundImage: "./assets/icon-foreground.png", backgroundColor: "#14171c" },
  },
  plugins: [
    "expo-router",
    "expo-secure-store",
    "expo-web-browser",
    "expo-status-bar",
    [
      "expo-font",
      {
        // Each named as its PostScript name, which is how iOS finds it, and Android by its file's.
        fonts: [
          "./assets/fonts/Inter-Regular.ttf",
          "./assets/fonts/Inter-Medium.ttf",
          "./assets/fonts/Inter-SemiBold.ttf",
          "./assets/fonts/JetBrainsMono-Regular.ttf",
          "./assets/fonts/JetBrainsMono-Medium.ttf",
        ],
      },
    ],
    [
      "expo-splash-screen",
      {
        image: "./assets/splash.png",
        imageWidth: 160,
        backgroundColor: "#0f1115",
      },
    ],
    [
      "expo-camera",
      {
        cameraPermission: "Yardsort uses the camera to scan the code your computer shows to pair.",
        // Never asked: scanning records nothing. The library's code refers to the microphone, and
        // Apple refuses an upload whose code does and whose Info.plist has no words for it.
        microphonePermission: "Yardsort does not use the microphone.",
        recordAudioAndroid: false,
      },
    ],
  ],
});

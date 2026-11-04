// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The root layout as the app has it, for what it gives every screen.

import { describe, expect, jest, test } from "@jest/globals";
import { DarkTheme, DefaultTheme, useTheme } from "expo-router";
import { act, renderRouter, screen } from "expo-router/testing-library";
import { Text } from "react-native";

import Layout from "../../src/app/_layout";

jest.mock("react-native/Libraries/Utilities/useColorScheme", () => ({
  __esModule: true,
  default: jest.fn(),
}));

function phoneIs(scheme: "dark" | "light") {
  const { default: useColorScheme } = jest.requireMock<{ default: jest.Mock<() => string> }>(
    "react-native/Libraries/Utilities/useColorScheme",
  );
  useColorScheme.mockReturnValue(scheme);
}

function Header() {
  const { dark, colors } = useTheme();
  return (
    <Text testID="header">
      {JSON.stringify({ dark, card: colors.card, text: colors.text, primary: colors.primary })}
    </Text>
  );
}

async function header() {
  await act(async () => {});
  await renderRouter({ _layout: Layout, index: Header }, { initialUrl: "/" });
  return JSON.parse((await screen.findByTestId("header")).props.children);
}

describe("the header", () => {
  // Android's is drawn from the theme. A white one on a dark phone hides the status bar's white icons.
  // `primary` is only iOS's back button, which keeps the system's blue.
  test("on a dark phone is dark, with light words", async () => {
    phoneIs("dark");
    expect(await header()).toEqual({
      dark: true,
      card: "#0f1115",
      text: "#e8eaed",
      primary: DarkTheme.colors.primary,
    });
  });

  test("on a light phone is light, with dark words", async () => {
    phoneIs("light");
    expect(await header()).toEqual({
      dark: false,
      card: "#f4f5f7",
      text: "#14171c",
      primary: DefaultTheme.colors.primary,
    });
  });
});

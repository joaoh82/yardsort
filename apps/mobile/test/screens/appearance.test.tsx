// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Choosing light, dark or as the phone is, through the screen, and it lasting.

import { beforeEach, describe, expect, jest, test } from "@jest/globals";
import { useTheme } from "expo-router";
import { fireEvent, renderRouter, screen } from "expo-router/testing-library";
import { Appearance, Text } from "react-native";

import Layout from "../../src/app/_layout";
import Settings from "../../src/app/settings";
import { appearance, AppearanceChoice, type ChoiceFile } from "../../src/lib/appearance";
import { appearanceFile, cacheShelf } from "../../src/lib/native";

jest.mock("react-native/Libraries/Utilities/useColorScheme", () => ({
  __esModule: true,
  default: jest.fn(),
}));

// Files as the phone's are in one respect that matters here: `write` finishes later, on another
// thread, and tells of a failure only through its promise. `writeSync` is done when it returns.
jest.mock("expo-file-system", () => {
  const disk = { files: new Map<string, string>(), full: false };
  class File {
    private readonly path: string;
    constructor(folder: string, name: string) {
      this.path = `${folder}/${name}`;
    }
    get exists() {
      return disk.files.has(this.path);
    }
    create() {
      disk.files.set(this.path, "");
    }
    textSync() {
      return disk.files.get(this.path);
    }
    writeSync(content: string) {
      if (disk.full) throw new Error("the disk is full");
      disk.files.set(this.path, content);
    }
    write(content: string) {
      return new Promise<void>((resolve, reject) =>
        setTimeout(() => {
          if (disk.full) return reject(new Error("the disk is full"));
          disk.files.set(this.path, content);
          resolve();
        }, 1),
      );
    }
    delete() {
      disk.files.delete(this.path);
    }
  }
  return { File, Paths: { document: "documents", cache: "cache" }, disk };
});

const { disk } = jest.requireMock<{ disk: { files: Map<string, string>; full: boolean } }>(
  "expo-file-system",
);

function phoneIs(scheme: "dark" | "light") {
  const { default: useColorScheme } = jest.requireMock<{ default: jest.Mock<() => string> }>(
    "react-native/Libraries/Utilities/useColorScheme",
  );
  useColorScheme.mockReturnValue(scheme);
}

function memoryFile(kept: string | null = null): ChoiceFile & { kept: string | null } {
  return {
    kept,
    read() {
      return this.kept;
    },
    write(choice) {
      this.kept = choice;
    },
  };
}

/** Whether the header, which is drawn from the theme, is dark, beside the screen under test. */
function Top() {
  const { dark } = useTheme();
  return <Text testID="dark">{String(dark)}</Text>;
}

async function settings() {
  await renderRouter(
    {
      _layout: Layout,
      index: () => (
        <>
          <Top />
          <Settings />
        </>
      ),
    },
    { initialUrl: "/" },
  );
}

const isDark = async () => (await screen.findByTestId("dark")).props.children;
const checked = (name: string) => screen.getByRole("radio", { name }).props.accessibilityState;

describe("the appearance", () => {
  beforeEach(() => {
    jest.spyOn(Appearance, "setColorScheme").mockImplementation(() => {});
    appearance.choose("automatic");
    appearance.start(memoryFile());
  });

  test("is as the phone is until someone chooses", async () => {
    phoneIs("dark");
    await settings();
    expect(await isDark()).toBe("true");
    expect(checked("Automatic, As this phone is set")).toMatchObject({ checked: true });
    expect(checked("Dark")).toMatchObject({ checked: false });
  });

  test("dark on a light phone, at once, and telling the system", async () => {
    phoneIs("light");
    await settings();
    expect(await isDark()).toBe("false");
    await fireEvent.press(screen.getByRole("radio", { name: "Dark" }));
    expect(await isDark()).toBe("true");
    expect(checked("Dark")).toMatchObject({ checked: true });
    expect(checked("Automatic, As this phone is set")).toMatchObject({ checked: false });
    expect(Appearance.setColorScheme).toHaveBeenLastCalledWith("dark");
  });

  test("light on a dark phone, and back to following it", async () => {
    phoneIs("dark");
    await settings();
    await fireEvent.press(screen.getByRole("radio", { name: "Light" }));
    expect(await isDark()).toBe("false");
    await fireEvent.press(screen.getByRole("radio", { name: "Automatic, As this phone is set" }));
    expect(await isDark()).toBe("true");
    expect(Appearance.setColorScheme).toHaveBeenLastCalledWith("unspecified");
  });

  test("lasts until the app is next opened", () => {
    const file = memoryFile();
    const before = new AppearanceChoice();
    before.start(file);
    before.choose("dark");
    expect(file.kept).toBe("dark");

    const after = new AppearanceChoice();
    after.start(file);
    expect(after.get()).toBe("dark");
    expect(Appearance.setColorScheme).toHaveBeenLastCalledWith("dark");
  });

  test("that cannot be read, or makes no sense, is as the phone is", () => {
    const nonsense = new AppearanceChoice();
    nonsense.start(memoryFile("purple"));
    expect(nonsense.get()).toBe("automatic");

    const unreadable = new AppearanceChoice();
    unreadable.start({
      read: () => {
        throw new Error("no");
      },
      write: () => {},
    });
    expect(unreadable.get()).toBe("automatic");
  });

  test("that cannot be kept still holds for now", () => {
    const choice = new AppearanceChoice();
    choice.start({
      read: () => null,
      write: () => {
        throw new Error("no");
      },
    });
    choice.choose("light");
    expect(choice.get()).toBe("light");
  });

  describe("in the phone's own file", () => {
    beforeEach(() => {
      disk.files.clear();
      disk.full = false;
    });

    test("is written before choosing returns, and read the next time", () => {
      const before = new AppearanceChoice();
      before.start(appearanceFile);
      before.choose("dark");
      expect(disk.files.get("documents/appearance.txt")).toBe("dark");

      const after = new AppearanceChoice();
      after.start(appearanceFile);
      expect(after.get()).toBe("dark");
    });

    test("that cannot be written still holds for now, and the old one is read next time", () => {
      const before = new AppearanceChoice();
      before.start(appearanceFile);
      before.choose("dark");
      disk.full = true;
      before.choose("light");
      expect(before.get()).toBe("light");

      const after = new AppearanceChoice();
      after.start(appearanceFile);
      expect(after.get()).toBe("dark");
    });
  });
});

describe("what was last seen, in the cache folder", () => {
  beforeEach(() => {
    jest.useRealTimers();
    disk.files.clear();
    disk.full = false;
  });

  // Forgetting waits for the last write. One still on its way would land after the delete, and keep
  // what was last seen after signing out.
  test("is on disk when keeping it is done", async () => {
    await cacheShelf.set("seen", "{}");
    expect(disk.files.get("cache/seen.json")).toBe("{}");
    await cacheShelf.delete("seen");
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(disk.files.has("cache/seen.json")).toBe(false);
  });

  test("that cannot be written says so", async () => {
    disk.full = true;
    await expect(cacheShelf.set("seen", "{}")).rejects.toThrow("the disk is full");
  });
});

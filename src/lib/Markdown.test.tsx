import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { Markdown } from "./Markdown";

beforeEach(() => {
  vi.resetAllMocks();
  opener.openUrl.mockResolvedValue(undefined);
});

describe("Markdown", () => {
  it("renders GitHub's flavour: headings, lists, tables, task lists, code", () => {
    const { container } = render(
      <Markdown
        text={[
          "## What",
          "",
          "Fixes the **redirect** and ~~the old flag~~.",
          "",
          "- [x] the steps",
          "- [ ] the screenshots",
          "",
          "| File | Lines |",
          "| ---- | ----- |",
          "| a.rs | 12    |",
          "",
          "Run `just check`:",
          "",
          "```sh",
          "just check",
          "```",
        ].join("\n")}
      />,
    );
    expect(screen.getByRole("heading", { name: "What" })).toBeVisible();
    expect(container.querySelector("strong")).toHaveTextContent("redirect");
    expect(container.querySelector("del")).toHaveTextContent("the old flag");
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes.map((box) => (box as HTMLInputElement).checked)).toEqual([true, false]);
    // What was ticked is shown; it is not there to be ticked from here.
    for (const box of boxes) expect(box).toBeDisabled();
    expect(within(screen.getByRole("table")).getByRole("cell", { name: "a.rs" })).toBeVisible();
    expect(container.querySelector("pre")).toHaveTextContent("just check");
  });

  it("renders no HTML at all, however it is written", () => {
    const { container } = render(
      <Markdown
        text={[
          "Before.",
          "",
          "<!-- a template comment nobody should see -->",
          "",
          '<img src="https://tracker.example.com/pixel.gif" onerror="alert(1)">',
          "",
          "<script>window.pwned = true</script>",
          "",
          '<iframe src="https://example.com"></iframe>',
          "",
          '<a href="https://example.com" onclick="alert(1)">inline html link</a>',
          "",
          "After.",
        ].join("\n")}
      />,
    );
    expect(screen.getByText("Before.")).toBeVisible();
    expect(screen.getByText("After.")).toBeVisible();
    expect(container.querySelector("img, script, iframe")).toBeNull();
    expect(container.innerHTML).not.toContain("template comment");
    expect(container.innerHTML).not.toContain("onerror");
    expect(container.innerHTML).not.toContain("onclick");
    expect((window as unknown as { pwned?: boolean }).pwned).toBeUndefined();
  });

  it("opens a link in the browser and never navigates the window", () => {
    render(
      <Markdown text="See [the docs](https://example.com/docs) or https://example.com/bare." />,
    );
    const link = screen.getByRole("link", { name: "the docs" });
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    fireEvent(link, click);
    expect(click.defaultPrevented, "the webview stays where it is").toBe(true);
    expect(opener.openUrl).toHaveBeenCalledWith("https://example.com/docs");

    // A bare address is a link too, the way GitHub shows it.
    fireEvent.click(screen.getByRole("link", { name: "https://example.com/bare" }));
    expect(opener.openUrl).toHaveBeenLastCalledWith("https://example.com/bare");

    // A middle click must not ask the webview for a new window either.
    const middle = new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 });
    fireEvent(link, middle);
    expect(middle.defaultPrevented).toBe(true);
  });

  it("makes a link only of an address on the web or a mail address", () => {
    render(
      <Markdown
        text={[
          "[script](javascript:alert(1))",
          "[file](file:///etc/passwd)",
          "[relative](../other.md)",
          "[anchor](#top)",
          "[mail](mailto:hello@example.com)",
        ].join(" · ")}
      />,
    );
    expect(screen.getAllByRole("link").map((link) => link.textContent)).toEqual(["mail"]);
    // Their words are still there, as words.
    for (const text of ["script", "file", "relative", "anchor"]) {
      expect(screen.getByText(text).closest("a")).toBeNull();
    }
  });

  it("loads no image: it shows a link with the image's description instead", () => {
    const { container } = render(
      <Markdown
        text={"![the new dialog](https://example.com/shot.png)\n\n![](data:image/png;base64,AAAA)"}
      />,
    );
    expect(container.querySelector("img")).toBeNull();
    const link = screen.getByRole("link", { name: /image: the new dialog/ });
    fireEvent.click(link);
    expect(opener.openUrl).toHaveBeenCalledWith("https://example.com/shot.png");
    // An image that is not at an address on the web is only its label.
    expect(screen.getByText("[image: image]").closest("a")).toBeNull();
  });
});

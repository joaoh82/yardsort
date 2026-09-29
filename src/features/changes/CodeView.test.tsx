import { act, render } from "@testing-library/react";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { beforeEach, expect, it, vi } from "vitest";
import { CodeView } from "./CodeView";

beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addListener: () => {},
    removeListener: () => {},
  }));
});

it("keeps the editor and undo history while typing, preserving CRLF", () => {
  const changed = vi.fn();
  const props = { path: "plain.txt", text: "old\r\nline\r\n", onChange: changed };
  const view = render(<CodeView {...props} />);
  const editor = EditorView.findFromDOM(view.container.querySelector(".cm-editor")!)!;
  act(() => editor.dispatch({ changes: { from: 0, to: 3, insert: "new" } }));
  expect(changed).toHaveBeenLastCalledWith("new\r\nline\r\n");
  view.rerender(<CodeView {...props} text="new\r\nline\r\n" />);
  expect(EditorView.findFromDOM(view.container.querySelector(".cm-editor")!)).toBe(editor);
  act(() => {
    undo(editor);
  });
  expect(changed).toHaveBeenLastCalledWith("old\r\nline\r\n");
});
it("updates a clean editor from disk without treating the update as typing", () => {
  const changed = vi.fn();
  const view = render(<CodeView path="plain.txt" text="old" onChange={changed} />);
  view.rerender(<CodeView path="plain.txt" text="external edit" onChange={changed} />);
  expect(view.container.querySelector(".cm-content")).toHaveTextContent("external edit");
  expect(changed).not.toHaveBeenCalled();
});

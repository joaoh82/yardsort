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
  view.rerender(<CodeView {...props} text={"new\r\nline\r\n"} />);
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
  const editor = EditorView.findFromDOM(view.container.querySelector(".cm-editor")!)!;
  act(() => {
    expect(undo(editor)).toBe(false);
  });
  expect(view.container.querySelector(".cm-content")).toHaveTextContent("external edit");
  expect(changed).not.toHaveBeenCalled();
});

it("clears old undo entries when an external version replaces the document", () => {
  const changed = vi.fn();
  const view = render(<CodeView path="plain.txt" text="old" onChange={changed} />);
  const editor = EditorView.findFromDOM(view.container.querySelector(".cm-editor")!)!;
  act(() => editor.dispatch({ changes: { from: 0, to: 3, insert: "saved edit" } }));
  view.rerender(<CodeView path="plain.txt" text="saved edit" onChange={changed} />);
  view.rerender(<CodeView path="plain.txt" text="external edit" onChange={changed} />);
  changed.mockClear();
  act(() => {
    expect(undo(editor)).toBe(false);
  });
  expect(editor.state.sliceDoc()).toBe("external edit");
  expect(changed).not.toHaveBeenCalled();
});

it("puts a note under its line, in the pane that shows that side", () => {
  const original = "one\ntwo\nthree\nfour\n";
  const text = "one\n2\nthree\nfour\n";
  const notes = [
    { key: "n4", line: 4, side: "new" as const },
    { key: "o2", line: 2, side: "old" as const },
  ];
  const renderNote = (key: string) => <span data-note={key}>note {key}</span>;
  // One pane: a note on the old text goes where that line went in the new text.
  const view = render(
    <CodeView path="a.txt" text={text} original={original} notes={notes} renderNote={renderNote} />,
  );
  const content = () => view.container.querySelector(".cm-content")!;
  const lineOf = (key: string) => {
    const block = content().querySelector(`[data-note="${key}"]`)!.closest(".cm-note")!;
    const lines = [...content().querySelectorAll(".cm-line")];
    return lines.filter(
      (line) => line.compareDocumentPosition(block) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).length;
  };
  expect(content()).toHaveTextContent("note n4");
  expect(lineOf("n4")).toBe(4);
  // Old line 2 ("two") became "2": the note sits under the changed line.
  expect(lineOf("o2")).toBe(2);

  // Two panes: each note in its own pane.
  view.rerender(
    <CodeView
      path="a.txt"
      text={text}
      original={original}
      split
      notes={notes}
      renderNote={renderNote}
    />,
  );
  const panes = view.container.querySelectorAll(".cm-editor");
  expect(panes).toHaveLength(2);
  expect(panes[0]).toHaveTextContent("note o2");
  expect(panes[0]).not.toHaveTextContent("note n4");
  expect(panes[1]).toHaveTextContent("note n4");

  // A note that goes is gone; one that stays keeps its node.
  const kept = panes[1]!.querySelector('[data-note="n4"]');
  view.rerender(
    <CodeView
      path="a.txt"
      text={text}
      original={original}
      split
      notes={[notes[0]!]}
      renderNote={renderNote}
    />,
  );
  expect(view.container.querySelector('[data-note="o2"]')).toBeNull();
  expect(view.container.querySelector('[data-note="n4"]')).toBe(kept);
});

it("says which lines are selected, whole, and on which side", () => {
  const selected = vi.fn();
  const view = render(
    <CodeView
      path="a.txt"
      text={"one\ntwo\nthree\n"}
      original={"one\ntwo\n"}
      split
      onSelect={selected}
    />,
  );
  const [a, b] = [...view.container.querySelectorAll(".cm-editor")].map((pane) =>
    EditorView.findFromDOM(pane as HTMLElement)!,
  );
  // "wo\nthr" in the new pane: lines 2 and 3, whole.
  act(() => b!.dispatch({ selection: { anchor: 5, head: 11 } }));
  expect(selected).toHaveBeenLastCalledWith({ side: "new", from: 2, to: 3, text: "two\nthree" });
  // A selection that ends at the start of a line does not take that line.
  act(() => b!.dispatch({ selection: { anchor: 0, head: 4 } }));
  expect(selected).toHaveBeenLastCalledWith({ side: "new", from: 1, to: 1, text: "one" });
  // The old pane says so.
  act(() => a!.dispatch({ selection: { anchor: 4, head: 6 } }));
  expect(selected).toHaveBeenLastCalledWith({ side: "old", from: 2, to: 2, text: "two" });
  // A caret is not a selection.
  act(() => a!.dispatch({ selection: { anchor: 2 } }));
  expect(selected).toHaveBeenLastCalledWith(null);
});

import { act, render, screen } from "@testing-library/react";
import { EditorView } from "@codemirror/view";
import { beforeEach, expect, it, vi } from "vitest";
import { WorkflowEditor } from "./WorkflowEditor";

beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {} }));
});

const text = "steps:\n  - id: first\n    action: notify\n  - id: second\n    action: notify\n";

it.each([false, true])(
  "navigates and highlights a step without changing YAML (readOnly=%s)",
  (readOnly) => {
    const onChange = vi.fn();
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const base = { text, problems: [], onChange, readOnly, stepLines: { first: 2, second: 4 } };
    const view = render(<WorkflowEditor {...base} />);
    view.rerender(<WorkflowEditor {...base} selection={{ id: "second" }} />);
    expect(
      screen.getByLabelText("Workflow file").querySelector(".cm-selectedStep"),
    ).toHaveTextContent("id: second");
    expect(scroll).toHaveBeenLastCalledWith(text.indexOf("  - id: second"), { y: "center" });
    expect(onChange).not.toHaveBeenCalled();
    const editor = EditorView.findFromDOM(
      screen.getByLabelText("Workflow file").querySelector(".cm-editor")!,
    )!;
    expect(editor.state.doc.toString()).toBe(text);
    expect(editor.state.selection.main.head).toBe(text.indexOf("  - id: second"));
    act(() => editor.dispatch({ selection: { anchor: 0 } }));
    view.rerender(<WorkflowEditor {...base} selection={{ id: "second" }} />);
    expect(editor.state.selection.main.head).toBe(text.indexOf("  - id: second"));
    view.rerender(<WorkflowEditor {...base} selection={{ id: null }} />);
    expect(screen.getByLabelText("Workflow file").querySelector(".cm-selectedStep")).toBeNull();
    scroll.mockRestore();
  },
);

it("waits for source positions from the edited text and clears a missing step", () => {
  const selection = { id: "second" };
  const edited = `# new line\n${text}`;
  const base = { text: edited, problems: [], selection };
  const view = render(<WorkflowEditor {...base} />);
  const host = screen.getByLabelText("Workflow file");
  expect(host.querySelector(".cm-selectedStep")).toBeNull();
  view.rerender(<WorkflowEditor {...base} stepLines={{ second: 5 }} />);
  expect(host.querySelector(".cm-selectedStep")).toHaveTextContent("id: second");
  view.rerender(<WorkflowEditor {...base} stepLines={{}} />);
  expect(host.querySelector(".cm-selectedStep")).toBeNull();
});

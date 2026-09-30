import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { expect, it } from "vitest";
import { treeNavigation } from "./treeNavigation";

function Tree() {
  const [open, setOpen] = useState(false);
  return (
    <ul role="tree" onKeyDown={treeNavigation}>
      <li role="treeitem" aria-expanded={open}>
        <button onClick={() => setOpen(!open)}>Folder</button>
        {open && (
          <ul role="group">
            <li role="treeitem">
              <button>Child</button>
            </li>
          </ul>
        )}
      </li>
      <li role="treeitem">
        <button>File</button>
      </li>
    </ul>
  );
}
it("navigates rows, expands folders, returns to parents and reaches the ends", () => {
  render(<Tree />);
  const folder = screen.getByRole("button", { name: "Folder" });
  folder.focus();
  fireEvent.keyDown(folder, { key: "ArrowRight" });
  expect(screen.getByRole("button", { name: "Child" })).toBeInTheDocument();
  fireEvent.keyDown(folder, { key: "ArrowDown" });
  const child = screen.getByRole("button", { name: "Child" });
  expect(child).toHaveFocus();
  fireEvent.keyDown(child, { key: "ArrowLeft" });
  expect(folder).toHaveFocus();
  fireEvent.keyDown(folder, { key: "ArrowLeft" });
  expect(screen.queryByRole("button", { name: "Child" })).not.toBeInTheDocument();
  fireEvent.keyDown(folder, { key: "End" });
  const file = screen.getByRole("button", { name: "File" });
  expect(file).toHaveFocus();
  fireEvent.keyDown(file, { key: "Home" });
  expect(folder).toHaveFocus();
});

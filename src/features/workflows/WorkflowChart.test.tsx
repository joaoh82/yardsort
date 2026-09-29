import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { WorkflowChart } from "./WorkflowChart";

it("selects a chart node by click and keyboard, with a visible selected state", async () => {
  const onSelect = vi.fn();
  const steps = [{ id: "tell", action: "notify" as const, needs: [], title: "Hi", body: null }];
  const view = render(<WorkflowChart steps={steps} selectedStep={null} onSelect={onSelect} />);
  const node = screen.getByLabelText("Select step tell");
  await userEvent.setup().click(node);
  expect(onSelect).toHaveBeenLastCalledWith("tell");
  view.rerender(<WorkflowChart steps={steps} selectedStep="tell" onSelect={onSelect} />);
  expect(node).toHaveAttribute("aria-pressed", "true");
  expect(node).toHaveClass("border-accent");
  node.focus();
  await userEvent.setup().keyboard("{Enter}");
  expect(onSelect).toHaveBeenCalledTimes(2);
});

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { layoutSteps } from "./layout";

/** The steps of the shipped factory, read from the yaml the app runs, not a copy of it. */
function softwareFactorySteps(): { id: string; needs: string[]; routes?: { to: string }[] }[] {
  const file = resolve(
    dirname(fileURLToPath(import.meta.url)),
    "../../../crates/core/src/workflow/builtin/software-factory.yaml",
  );
  const body = (readFileSync(file, "utf8").split("\nsteps:\n")[1] ?? "").replace(/^ {2}- id: /, "");
  return body
    .split(/\n {2}- id: /)
    .filter((chunk) => chunk.trim().length > 0)
    .map((chunk) => {
      const id = chunk.split("\n")[0]?.trim() ?? "";
      const listed = chunk.match(/\n {4}needs: \[([^\]]*)\]/)?.[1] ?? "";
      const needs = listed
        .split(",")
        .map((name) => name.trim())
        .filter((name) => name.length > 0);
      const routes = (chunk.match(/\n {4}on:\n((?: {6}.+\n)*)/)?.[1] ?? "")
        .split("\n")
        .map((line) => line.split(":")[1]?.trim() ?? "")
        .filter((to) => to.length > 0)
        .map((to) => ({ to }));
      return routes.length > 0 ? { id, needs, routes } : { id, needs };
    });
}

const at = (placed: { id: string; x: number; y: number }[], id: string) =>
  placed.find((node) => node.id === id)!;

describe("layoutSteps", () => {
  it("puts every step below what it needs, and siblings side by side", () => {
    const { nodes, links } = layoutSteps([
      { id: "review", needs: [] },
      { id: "settled", needs: ["review"] },
      { id: "posted", needs: ["settled"] },
      { id: "tell_user", needs: ["posted"] },
      { id: "tell_author", needs: ["posted"] },
    ]);
    const y = (id: string) => at(nodes, id).y;
    expect(y("review")).toBeLessThan(y("settled"));
    expect(y("settled")).toBeLessThan(y("posted"));
    expect(y("posted")).toBeLessThan(y("tell_user"));
    expect(y("tell_user")).toBe(y("tell_author"));
    expect(at(nodes, "tell_user").x).not.toBe(at(nodes, "tell_author").x);
    expect(links.map((l) => l.id)).toEqual([
      "review->settled",
      "settled->posted",
      "posted->tell_user",
      "posted->tell_author",
    ]);
  });

  it("draws nothing for a need that names no step, or the step itself", () => {
    const { nodes, links } = layoutSteps([
      { id: "a", needs: ["missing", "a"] },
      { id: "b", needs: [] },
    ]);
    expect(nodes).toHaveLength(2);
    expect(links).toEqual([]);
  });

  it("draws a route forward in the layout, and a route back without a cycle", () => {
    const { nodes, links } = layoutSteps([
      { id: "plan", needs: [] },
      {
        id: "sign-off",
        needs: ["plan"],
        routes: [{ to: "implement" }, { to: "plan" }, { to: "stop" }],
      },
      { id: "implement", needs: ["sign-off"] },
    ]);
    expect(at(nodes, "plan").y).toBeLessThan(at(nodes, "sign-off").y);
    expect(at(nodes, "sign-off").y).toBeLessThan(at(nodes, "implement").y);
    expect(links.map((link) => link.id)).toEqual([
      "plan->sign-off",
      "sign-off->implement",
      "sign-off->plan",
    ]);
  });

  it("draws the software factory as one column, with the three loops back", () => {
    const steps = softwareFactorySteps();
    const { nodes, links } = layoutSteps(steps);
    const order = steps.map((step) => step.id);
    expect(order).toEqual([
      "define",
      "brief",
      "gather",
      "context",
      "plan",
      "plan-ready",
      "approve-plan",
      "implement",
      "implement-ready",
      "verify",
      "await-review",
      "accept-review",
      "release",
      "result",
      "ready",
    ]);
    const y = (id: string) => at(nodes, id).y;
    for (let i = 1; i < order.length; i++) {
      const above = order[i - 1];
      const below = order[i];
      if (above === undefined || below === undefined) throw new Error("missing step");
      expect(y(above)).toBeLessThan(y(below));
    }
    expect(links.map((link) => link.id)).toEqual([
      "define->brief",
      "brief->gather",
      "gather->context",
      "context->plan",
      "plan->plan-ready",
      "plan-ready->approve-plan",
      "approve-plan->implement",
      "approve-plan->plan",
      "implement->implement-ready",
      "implement-ready->verify",
      "verify->await-review",
      "verify->implement",
      "await-review->accept-review",
      "accept-review->release",
      "accept-review->implement",
      "release->result",
      "result->ready",
    ]);
  });
});

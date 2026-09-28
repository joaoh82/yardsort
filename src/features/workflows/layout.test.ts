import { describe, expect, it } from "vitest";
import { layoutSteps } from "./layout";

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
});

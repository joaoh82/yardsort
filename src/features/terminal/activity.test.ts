import { describe, expect, it } from "vitest";
import { harnessState } from "./activity";

const ok = { code: 0, success: true, signal: null };
const bad = { code: 1, success: false, signal: null };
const tab = (over: Partial<Parameters<typeof harnessState>[0][number]> = {}) => ({
  busy: false,
  exit: null,
  recordId: "r1",
  ...over,
});

describe("harnessState", () => {
  it("says nothing about a workspace with no harness in it", () => {
    expect(harnessState([])).toEqual({ kind: "none" });
    expect(harnessState([tab({ recordId: null })])).toEqual({ kind: "none" });
    expect(harnessState([tab({ recordId: null, exit: ok })])).toEqual({ kind: "none" });
  });

  it("counts the agents waiting for you, and lets them outrank one still working", () => {
    expect(harnessState([tab()])).toEqual({ kind: "waiting", count: 1 });
    expect(harnessState([tab(), tab({ busy: true }), tab()])).toEqual({
      kind: "waiting",
      count: 2,
    });
  });

  it("is working only while every live agent is printing", () => {
    expect(harnessState([tab({ busy: true })])).toEqual({ kind: "working" });
    expect(harnessState([tab({ busy: true }), tab({ exit: ok })])).toEqual({ kind: "working" });
  });

  it("reports a finished agent, and a failure ahead of a clean finish", () => {
    expect(harnessState([tab({ exit: ok })])).toEqual({ kind: "done" });
    expect(harnessState([tab({ exit: ok }), tab({ exit: bad })])).toEqual({ kind: "failed" });
  });

  it("ignores a shell sitting at its prompt beside an ended agent", () => {
    expect(harnessState([tab({ exit: ok }), tab({ recordId: null })])).toEqual({ kind: "done" });
  });
});

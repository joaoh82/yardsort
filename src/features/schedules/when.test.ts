import { describe, expect, it } from "vitest";
import type { ScheduleSpec } from "@/lib/ipc";
import { DEFAULT_WHEN, fieldsOf, parseTime, specOf, toLocalInput, type WhenFields } from "./when";

const fields = (over: Partial<WhenFields>): WhenFields => ({ ...DEFAULT_WHEN, ...over });

describe("specOf", () => {
  it("reads each kind from its own fields", () => {
    expect(specOf(fields({ kind: "daily", time: "09:05" }))).toEqual({
      kind: "daily",
      time: { hour: 9, minute: 5 },
    });
    expect(specOf(fields({ kind: "every", every: "2", unit: "hours" }))).toEqual({
      kind: "every",
      minutes: 120,
    });
    expect(specOf(fields({ kind: "every", every: "3", unit: "days" }))).toEqual({
      kind: "every",
      minutes: 4320,
    });
    expect(specOf(fields({ kind: "weekly", days: ["sat", "sun"], time: "10:30" }))).toEqual({
      kind: "weekly",
      days: ["sat", "sun"],
      time: { hour: 10, minute: 30 },
    });
    expect(specOf(fields({ kind: "monthly", day: "last", time: "06:00" }))).toEqual({
      kind: "monthly",
      day: null,
      time: { hour: 6, minute: 0 },
    });
    expect(specOf(fields({ kind: "monthly", day: "15", time: "06:00" }))).toEqual({
      kind: "monthly",
      day: 15,
      time: { hour: 6, minute: 0 },
    });
    expect(specOf(fields({ kind: "cron", expr: "  0 3 * * *  " }))).toEqual({
      kind: "cron",
      expr: "0 3 * * *",
    });
  });

  it("reads a once on this machine's clock", () => {
    const spec = specOf(fields({ kind: "once", at: "2026-11-06T18:00" }));
    expect(spec).toEqual({ kind: "once", at: new Date(2026, 10, 6, 18, 0).getTime() });
  });

  it("is nothing while the fields do not mean a time yet", () => {
    expect(specOf(fields({ kind: "once", at: "" }))).toBeNull();
    expect(specOf(fields({ kind: "every", every: "0" }))).toBeNull();
    expect(specOf(fields({ kind: "every", every: "soon" }))).toBeNull();
    expect(specOf(fields({ kind: "daily", time: "" }))).toBeNull();
    expect(specOf(fields({ kind: "weekly", days: [] }))).toBeNull();
    expect(specOf(fields({ kind: "monthly", day: "31" }))).toBeNull();
    expect(specOf(fields({ kind: "cron", expr: "   " }))).toBeNull();
  });
});

describe("fieldsOf", () => {
  it("opens a spec on the fields it was written with", () => {
    const specs: ScheduleSpec[] = [
      { kind: "once", at: new Date(2026, 10, 6, 18, 0).getTime() },
      { kind: "every", minutes: 45 },
      { kind: "every", minutes: 120 },
      { kind: "every", minutes: 2880 },
      { kind: "daily", time: { hour: 9, minute: 5 } },
      { kind: "weekly", days: ["mon", "thu"], time: { hour: 18, minute: 30 } },
      { kind: "monthly", day: null, time: { hour: 0, minute: 0 } },
      { kind: "monthly", day: 28, time: { hour: 23, minute: 59 } },
      { kind: "cron", expr: "*/5 9-17 * * mon-fri" },
    ];
    for (const spec of specs) expect(specOf(fieldsOf(spec))).toEqual(spec);
  });

  it("shows an interval in the largest unit that divides it", () => {
    expect(fieldsOf({ kind: "every", minutes: 2880 })).toMatchObject({ every: "2", unit: "days" });
    expect(fieldsOf({ kind: "every", minutes: 90 })).toMatchObject({
      every: "90",
      unit: "minutes",
    });
  });

  it("starts a new schedule on a weekday morning", () => {
    expect(fieldsOf(null)).toEqual(DEFAULT_WHEN);
  });
});

describe("times", () => {
  it("reads a time of day and refuses what is not one", () => {
    expect(parseTime("9:00")).toEqual({ hour: 9, minute: 0 });
    expect(parseTime("23:59")).toEqual({ hour: 23, minute: 59 });
    for (const bad of ["", "24:00", "9:60", "9", "nine"]) expect(parseTime(bad)).toBeNull();
  });

  it("writes a moment the way a datetime-local input holds it", () => {
    expect(toLocalInput(new Date(2026, 0, 5, 7, 3).getTime())).toBe("2026-01-05T07:03");
  });
});

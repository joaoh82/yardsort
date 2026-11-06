import type { ScheduleSpec, TimeOfDay, Weekday } from "@/lib/ipc";

export type WhenKind = ScheduleSpec["kind"];
export type Unit = "minutes" | "hours" | "days";

/** What the dialog's "when" fields hold. Every kind's fields are kept, so switching kind and
 *  back loses nothing; only the chosen kind's fields make the spec. */
export interface WhenFields {
  kind: WhenKind;
  /** A `once`, as a `datetime-local` input holds it: `2026-11-06T18:00`, or empty. */
  at: string;
  every: string;
  unit: Unit;
  /** The time of day of `daily`, `weekly` and `monthly`: `09:00`. */
  time: string;
  days: Weekday[];
  /** A day of the month, `1` to `28`, or `last`. */
  day: string;
  expr: string;
}

export const WEEKDAYS: { id: Weekday; label: string }[] = [
  { id: "mon", label: "Mon" },
  { id: "tue", label: "Tue" },
  { id: "wed", label: "Wed" },
  { id: "thu", label: "Thu" },
  { id: "fri", label: "Fri" },
  { id: "sat", label: "Sat" },
  { id: "sun", label: "Sun" },
];

export const KINDS: { id: WhenKind; label: string }[] = [
  { id: "once", label: "Once" },
  { id: "every", label: "Every…" },
  { id: "daily", label: "Daily" },
  { id: "weekly", label: "Weekly" },
  { id: "monthly", label: "Monthly" },
  { id: "cron", label: "Advanced (cron)" },
];

export const DEFAULT_WHEN: WhenFields = {
  kind: "daily",
  at: "",
  every: "1",
  unit: "hours",
  time: "09:00",
  days: ["mon", "tue", "wed", "thu", "fri"],
  day: "1",
  expr: "0 9 * * 1-5",
};

const PER: Record<Unit, number> = { minutes: 1, hours: 60, days: 1440 };

/** `09:05` as a time of day; null for anything else. */
export function parseTime(text: string): TimeOfDay | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(text.trim());
  if (!match) return null;
  const [hour, minute] = [Number(match[1]), Number(match[2])];
  return hour < 24 && minute < 60 ? { hour, minute } : null;
}

const two = (n: number) => String(n).padStart(2, "0");

/** A moment as a `datetime-local` input holds it, on this machine's clock. */
export function toLocalInput(at: number): string {
  const d = new Date(at);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}`;
}

/**
 * The spec the fields mean, or null while they do not mean one yet: no date picked, no day
 * ticked, a number that is not one. What is wrong with a spec that is one — a time in the past,
 * a cron expression that does not read — is the core's to say.
 */
export function specOf(fields: WhenFields): ScheduleSpec | null {
  const time = parseTime(fields.time);
  switch (fields.kind) {
    case "once": {
      const at = fields.at ? new Date(fields.at).getTime() : NaN;
      return Number.isFinite(at) ? { kind: "once", at } : null;
    }
    case "every": {
      const count = Number(fields.every);
      if (!/^\d+$/.test(fields.every.trim()) || count < 1) return null;
      return { kind: "every", minutes: count * PER[fields.unit] };
    }
    case "daily":
      return time ? { kind: "daily", time } : null;
    case "weekly":
      return time && fields.days.length > 0 ? { kind: "weekly", days: fields.days, time } : null;
    case "monthly": {
      if (!time) return null;
      if (fields.day === "last") return { kind: "monthly", day: null, time };
      const day = Number(fields.day);
      return Number.isInteger(day) && day >= 1 && day <= 28 ? { kind: "monthly", day, time } : null;
    }
    case "cron":
      return fields.expr.trim() ? { kind: "cron", expr: fields.expr.trim() } : null;
  }
}

/** The fields a spec opens the dialog on, for editing: its own kind's fields from the spec, the
 *  others at their defaults. */
export function fieldsOf(spec: ScheduleSpec | null): WhenFields {
  if (!spec) return DEFAULT_WHEN;
  const clock = (time: TimeOfDay) => `${two(time.hour)}:${two(time.minute)}`;
  switch (spec.kind) {
    case "once":
      return { ...DEFAULT_WHEN, kind: "once", at: toLocalInput(spec.at) };
    case "every": {
      const unit: Unit =
        spec.minutes % 1440 === 0 ? "days" : spec.minutes % 60 === 0 ? "hours" : "minutes";
      return { ...DEFAULT_WHEN, kind: "every", every: String(spec.minutes / PER[unit]), unit };
    }
    case "daily":
      return { ...DEFAULT_WHEN, kind: "daily", time: clock(spec.time) };
    case "weekly":
      return { ...DEFAULT_WHEN, kind: "weekly", days: spec.days, time: clock(spec.time) };
    case "monthly":
      return {
        ...DEFAULT_WHEN,
        kind: "monthly",
        day: spec.day === null ? "last" : String(spec.day),
        time: clock(spec.time),
      };
    case "cron":
      return { ...DEFAULT_WHEN, kind: "cron", expr: spec.expr };
  }
}

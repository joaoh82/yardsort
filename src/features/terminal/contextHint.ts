import { create } from "zustand";

/** Pause between typing the command and pressing Enter; some TUIs drop an Enter sent with it. */
export const SUBMIT_DELAY_MS = 150;

/**
 * How full the context has to be, in steps, for a dismissed hint to come back: once dismissed
 * at 82 %, it stays away until 90 %, then 95 %.
 */
export function band(percent: number): number {
  if (percent >= 95) return 95;
  if (percent >= 90) return 90;
  return percent >= 80 ? 80 : 0;
}

/** The band each conversation's hint was dismissed at. Only how the hint looks, never a fact. */
export const useDismissedContext = create<{
  byRecord: Record<string, number>;
  set: (recordId: string, band: number) => void;
}>((set) => ({
  byRecord: {},
  set: (recordId, value) => set((s) => ({ byRecord: { ...s.byRecord, [recordId]: value } })),
}));

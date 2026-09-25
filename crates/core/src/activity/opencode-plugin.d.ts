// Types for the plugin's pure part, so the frontend's test suite can import it under
// TypeScript; the plugin itself runs inside OpenCode, untyped, as a plain module. The module's
// one export is the plugin function — OpenCode calls every export as a plugin — and `reduce`
// hangs off it.

export interface Reduced {
  hook: string;
  payload: Record<string, unknown>;
  directory?: string;
}

export const YardsortActivity: ((ctx: {
  directory?: string;
}) => Promise<Record<string, unknown>>) & {
  /** The reduced form of one hook call, or null when the call is not one Yardsort records. */
  reduce: (hook: string, payload: unknown, directory?: string) => Reduced | null;
};

// Types for the plugin's pure part, so the frontend's test suite can import it under
// TypeScript; the plugin itself runs inside OpenCode, untyped, as a plain module. The module's
// one export is its default: the definition OpenCode 1 starts through `server` and OpenCode 2
// through `setup`. The pure functions hang off it under `testing`.

export interface Reduced {
  hook: string;
  payload: Record<string, unknown>;
  directory?: string;
  /** Set to 2 on what OpenCode 2's API reported. */
  api?: number;
}

/** The plugin's memory between OpenCode 2 calls: when each tool call began, each step's model. */
export interface Seen {
  started: Map<string, number>;
  models: Map<string, unknown>;
}

declare const plugin: {
  id: string;
  server: (ctx: { directory?: string }) => Promise<Record<string, unknown>>;
  setup: (ctx: unknown) => Promise<() => void> | undefined;
  testing: {
    /** The reduced form of one OpenCode 1 hook call, or null when it is not one Yardsort records. */
    reduce: (hook: string, payload: unknown, directory?: string) => Reduced | null;
    /** The same for an OpenCode 2 hook event or bus event. */
    reduceV2: (hook: string, event: unknown, directory?: string, seen?: Seen) => Reduced | null;
    /** Whether a call may have opened a pull request, so its command and output are handed on. */
    mayOpenPullRequest: (tool: unknown, args: unknown) => boolean;
    /** Whether the process is OpenCode 2's shared background service. */
    inSharedService: (argv?: string[]) => boolean;
    /** `setup`, with the process's arguments given instead of read. */
    start: (ctx: unknown, argv: string[]) => Promise<() => void> | undefined;
  };
};

export default plugin;

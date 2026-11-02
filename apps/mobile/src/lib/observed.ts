// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What this phone last saw of a profile's computers and workspaces (31 §5), so a cold start or an
 * offline computer shows something, each with when it was seen.
 *
 * It holds names and branches and nothing else: never a session title, activity, memory, terminal
 * output or a credential. Losing it costs nothing, so a shelf that fails is the same as an empty
 * one, and what does not read is left out. Each profile's is its own, and goes with the profile.
 */

import type { HostId, ProjectId, WorkspaceId, WorkspacePage } from "@yardsort/remote-client";

/** Where it is kept: the phone's cache folder, a map in tests. */
export interface Shelf {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export interface ObservedComputer {
  host: HostId;
  name: string;
  lastSeenAt: number | null;
  observedAt: number;
}

export interface ObservedProject {
  id: ProjectId;
  name: string;
}

export interface ObservedWorkspace {
  id: WorkspaceId;
  project: ProjectId;
  name: string;
  branch: string | null;
  missing: boolean;
}

export interface ObservedWorkspaces {
  observedAt: number;
  projects: ObservedProject[];
  workspaces: ObservedWorkspace[];
}

interface Kept {
  version: 1;
  computers: ObservedComputer[];
  workspaces: Record<string, ObservedWorkspaces>;
}

const MAX_COMPUTERS = 50;
const MAX_WORKSPACES = 100;

const empty = (): Kept => ({ version: 1, computers: [], workspaces: {} });

export class Observed {
  #kept: Promise<Kept> | null = null;
  /** One after another, so a late write cannot put back what was just taken away. */
  #writing: Promise<unknown> = Promise.resolve();
  #forgotten = false;

  constructor(
    readonly shelf: Shelf,
    readonly key: string,
    readonly clock: () => number = Date.now,
  ) {}

  async computers(): Promise<ObservedComputer[]> {
    return (await this.#read()).computers;
  }

  async workspaces(host: HostId): Promise<ObservedWorkspaces | null> {
    return (await this.#read()).workspaces[host] ?? null;
  }

  /** The computers the server says let this phone in. What was kept of any other goes. */
  keepComputers(computers: readonly Omit<ObservedComputer, "observedAt">[]): Promise<void> {
    return this.#write((kept) => {
      const hosts = computers.slice(0, MAX_COMPUTERS).map((c) => c.host);
      const workspaces: Kept["workspaces"] = {};
      for (const host of hosts) {
        const seen = kept.workspaces[host];
        if (seen) workspaces[host] = seen;
      }
      return {
        version: 1,
        computers: computers.slice(0, MAX_COMPUTERS).map((computer) => ({
          host: computer.host,
          name: computer.name,
          lastSeenAt: computer.lastSeenAt,
          observedAt: this.clock(),
        })),
        workspaces,
      };
    });
  }

  /** A computer's workspaces as it listed them, for a computer still known to let this phone in. */
  keepWorkspaces(host: HostId, page: WorkspacePage): Promise<void> {
    return this.#write((kept) => {
      if (!kept.computers.some((computer) => computer.host === host)) return kept;
      const workspaces = page.workspaces.slice(0, MAX_WORKSPACES);
      const named = new Set(workspaces.map((workspace) => workspace.project));
      return {
        ...kept,
        workspaces: {
          ...kept.workspaces,
          [host]: {
            observedAt: this.clock(),
            projects: page.projects
              .filter((project) => named.has(project.id))
              .map(({ id, name }) => ({ id, name })),
            workspaces: workspaces.map((workspace) => ({
              id: workspace.id,
              project: workspace.project,
              name: workspace.name,
              branch: workspace.branch?.label ?? null,
              missing: workspace.missing,
            })),
          },
        },
      };
    });
  }

  /** Delete it all. Nothing is kept after this, whatever was on its way. */
  forget(): Promise<void> {
    this.#forgotten = true;
    this.#kept = Promise.resolve(empty());
    const gone = this.#writing.catch(() => {}).then(() => this.shelf.delete(this.key));
    this.#writing = gone.catch(() => {});
    return this.#writing.then(() => {});
  }

  #read(): Promise<Kept> {
    this.#kept ??= this.shelf.get(this.key).then(readKept, empty);
    return this.#kept;
  }

  #write(change: (kept: Kept) => Kept): Promise<void> {
    const written = this.#writing.then(async () => {
      if (this.#forgotten) return;
      const next = change(await this.#read());
      this.#kept = Promise.resolve(next);
      await this.shelf.set(this.key, JSON.stringify(next));
    });
    this.#writing = written.catch(() => {});
    return this.#writing.then(() => {});
  }
}

const isString = (value: unknown): value is string => typeof value === "string";
const isTime = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);
const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

function readKept(text: string | null): Kept {
  if (!text) return empty();
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return empty();
  }
  if (!isObject(value) || value.version !== 1) return empty();
  const computers = Array.isArray(value.computers) ? value.computers.filter(isComputer) : [];
  const workspaces: Kept["workspaces"] = {};
  if (isObject(value.workspaces)) {
    for (const computer of computers) {
      const seen = value.workspaces[computer.host];
      if (isWorkspaces(seen)) workspaces[computer.host] = seen;
    }
  }
  return { version: 1, computers, workspaces };
}

function isComputer(value: unknown): value is ObservedComputer {
  return (
    isObject(value) &&
    isString(value.host) &&
    isString(value.name) &&
    (value.lastSeenAt === null || isTime(value.lastSeenAt)) &&
    isTime(value.observedAt)
  );
}

function isWorkspaces(value: unknown): value is ObservedWorkspaces {
  return (
    isObject(value) &&
    isTime(value.observedAt) &&
    Array.isArray(value.projects) &&
    value.projects.every((p) => isObject(p) && isString(p.id) && isString(p.name)) &&
    Array.isArray(value.workspaces) &&
    value.workspaces.every(
      (w) =>
        isObject(w) &&
        isString(w.id) &&
        isString(w.project) &&
        isString(w.name) &&
        (w.branch === null || isString(w.branch)) &&
        typeof w.missing === "boolean",
    )
  );
}

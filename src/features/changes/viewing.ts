import type { Content, FileChange } from "@/lib/ipc";
import type { Viewing } from "@/stores/changes";

export const titleOf = (viewing: Viewing) =>
  viewing.kind === "file" ? viewing.path : viewing.change.path;

/** Identifies what is being viewed across refreshes, which replace the `Viewing` object. */
export const keyOf = (viewing: Viewing) =>
  viewing.kind === "file" ? `file:${viewing.path}` : `${viewing.scope}:${viewing.change.path}`;

/** Inline or two panes, remembered across restarts like the composer's last picks. */
export const DIFF_MODE_KEY = "changes.diffMode";
export type DiffMode = "inline" | "split";

export const asText = (content: Content) => (content.type === "text" ? content.text : "");

/** Why a side cannot be shown as text, if it cannot. */
export function obstacle(content: Content): string | null {
  if (content.type === "notEditable") return content.reason;
  if (content.type === "binary") return "Binary file — not shown.";
  if (content.type === "tooLarge") {
    return `File too large to show (${(content.bytes / 1_048_576).toFixed(1)} MB).`;
  }
  return null;
}

/** How each kind of change is marked in a list of files: a letter, its name and its colour. */
export const CHANGE_KIND: Record<
  FileChange["kind"],
  { letter: string; label: string; colour: string }
> = {
  added: { letter: "A", label: "Added", colour: "text-green-400" },
  modified: { letter: "M", label: "Modified", colour: "text-accent" },
  deleted: { letter: "D", label: "Deleted", colour: "text-red-400" },
  renamed: { letter: "R", label: "Renamed", colour: "text-blue-400" },
  untracked: { letter: "U", label: "Untracked", colour: "text-green-400" },
  conflicted: { letter: "!", label: "Conflicted", colour: "text-red-400" },
};

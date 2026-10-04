import type { LineComment } from "@/lib/ipc";

/** A comment on lines and the replies under it, oldest first. */
export interface Thread {
  root: LineComment;
  replies: LineComment[];
}

/**
 * The comments on lines of one file, as threads: each first comment with its replies. A
 * reply to a comment that is not here — deleted, or on another file — stands as its own
 * thread rather than vanish.
 */
export function threadsOf(comments: LineComment[], path: string): Thread[] {
  const own = comments.filter((comment) => comment.path === path);
  const byId = new Map(own.map((comment) => [comment.id, comment]));
  const threads = new Map<string, Thread>();
  for (const comment of own) {
    const parent = comment.inReplyTo && byId.has(comment.inReplyTo) ? comment.inReplyTo : null;
    if (parent === null) {
      threads.set(comment.id, { root: comment, replies: [] });
    }
  }
  for (const comment of own) {
    const parent = comment.inReplyTo && threads.has(comment.inReplyTo) ? comment.inReplyTo : null;
    if (parent !== null) threads.get(parent)!.replies.push(comment);
  }
  return [...threads.values()].sort((a, b) => (a.root.at ?? 0) - (b.root.at ?? 0));
}

/** The threads that still have a line to sit on: not outdated, not about the whole file. */
export const placed = (threads: Thread[]): Thread[] =>
  threads.filter(
    (thread) => thread.root.line !== null && !thread.root.outdated && !thread.root.wholeFile,
  );

/** The rest: the forge no longer places them on a line, or they never were on one. */
export const unplaced = (threads: Thread[]): Thread[] =>
  threads.filter((thread) => !placed([thread]).length);

/** How many threads each file has, for the list of files. */
export function threadCounts(comments: LineComment[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const comment of comments) {
    if (comment.inReplyTo) continue;
    counts.set(comment.path, (counts.get(comment.path) ?? 0) + 1);
  }
  return counts;
}

/** Where a comment sits, in words: `lines 43–49`, `line 36`, or where it used to. */
export function placeOf(comment: LineComment): string {
  if (comment.wholeFile) return "on the whole file";
  const side = comment.side === "left" ? " of the old text" : "";
  if (comment.line === null) {
    return comment.originalLine === null
      ? "outdated"
      : `outdated — was on line ${comment.originalLine}${side}`;
  }
  const lines =
    comment.startLine !== null && comment.startLine !== comment.line
      ? `lines ${comment.startLine}–${comment.line}`
      : `line ${comment.line}`;
  return `${lines}${side}`;
}

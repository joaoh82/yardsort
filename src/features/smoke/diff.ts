/** One line of a two-version comparison: kept, added or removed. */
export type DiffLine = { kind: "same" | "added" | "removed"; text: string };

/**
 * The most lines the LCS matrix may span. The byte limit on a test file (256 KiB) does not
 * bound the line count — thousands of short lines fit under it — and the matrix is quadratic, so
 * a direct LCS could allocate billions of cells and freeze the window during rendering. Past
 * this many differing lines on either side, the diff falls back to showing the whole of each
 * version, which still lets the reader inspect what changed, without the matrix.
 */
const MAX_DIFF_LINES = 2000;

/**
 * The lines of `after` against `before`, for the approval card's "what changed": identical
 * leading and trailing lines are matched directly, and the differing middle is diffed by the
 * longest common subsequence of its lines. The common prefix/suffix stripping makes a one-line
 * edit in a long file cheap, and a cap on the middle keeps a pathological file from exhausting
 * the window.
 */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  if (a[a.length - 1] === "") a.pop();
  if (b[b.length - 1] === "") b.pop();

  // Identical lines at the start and end are always "same": match them off and diff the middle.
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail++;
  }
  const am = a.slice(head, a.length - tail);
  const bm = b.slice(head, b.length - tail);

  const out: DiffLine[] = [];
  for (let i = 0; i < head; i++) out.push({ kind: "same", text: a[i]! });

  if (am.length > MAX_DIFF_LINES || bm.length > MAX_DIFF_LINES) {
    // Too much differs to diff line by line in bounded memory: show the whole of each version.
    for (const text of am) out.push({ kind: "removed", text });
    for (const text of bm) out.push({ kind: "added", text });
  } else {
    out.push(...lcsDiff(am, bm));
  }

  for (let i = b.length - tail; i < b.length; i++) out.push({ kind: "same", text: b[i]! });
  return out;
}

/** The line-by-line diff of two already-trimmed slices, by longest common subsequence. */
function lcsDiff(a: string[], b: string[]): DiffLine[] {
  // lcs[i][j]: the longest common subsequence of a[i..] and b[j..].
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0),
  );
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i]![j] =
        a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ kind: "removed", text: a[i]! });
      i++;
    } else {
      out.push({ kind: "added", text: b[j]! });
      j++;
    }
  }
  for (; i < a.length; i++) out.push({ kind: "removed", text: a[i]! });
  for (; j < b.length; j++) out.push({ kind: "added", text: b[j]! });
  return out;
}

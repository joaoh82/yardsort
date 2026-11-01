/** One line of a two-version comparison: kept, added or removed. */
export type DiffLine = { kind: "same" | "added" | "removed"; text: string };

/**
 * The lines of `after` against `before`, for the approval card's "what changed": the longest
 * common subsequence of lines, which is all a page of YAML needs. Both texts are small by the
 * core's rule (a test file is at most 256 KiB), and the card shows one at a time.
 */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  if (a[a.length - 1] === "") a.pop();
  if (b[b.length - 1] === "") b.pop();
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

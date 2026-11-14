// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The Markdown agents write, read into blocks a phone can draw at its own width (43). Only what
 * agents use: paragraphs, headings, lists, code, quotes, rules and tables, and inside a line code,
 * bold, italics and links. Anything else is text, as written. Nothing here is ever run or fetched:
 * a link is its words and where it leads, and the screen asks before opening one.
 */

export interface Span {
  text: string;
  code?: true;
  bold?: true;
  italic?: true;
  /** Where a link leads. */
  href?: string;
}

export type Block =
  | { kind: "paragraph"; spans: Span[] }
  | { kind: "heading"; level: number; spans: Span[] }
  | { kind: "list"; ordered: boolean; start: number; items: Span[][] }
  | { kind: "code"; language: string; text: string }
  | { kind: "quote"; spans: Span[] }
  | { kind: "rule" }
  | { kind: "table"; header: Span[][]; rows: Span[][][] };

const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}(\d{1,9})[.)]\s+(.*)$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE_ROW = /^\s*\|.*\|\s*$/;
const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

/** Read `text` into blocks. Never throws: what it does not know is a paragraph. */
export function readMarkdown(text: string): Block[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  const endParagraph = () => {
    if (paragraph.length > 0)
      blocks.push({ kind: "paragraph", spans: inline(paragraph.join(" ")) });
    paragraph = [];
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] as string;
    const fence = FENCE.exec(line);
    if (fence) {
      endParagraph();
      const marker = fence[1] as string;
      const body: string[] = [];
      i++;
      while (i < lines.length && !(lines[i] as string).trimStart().startsWith(marker)) {
        body.push(lines[i] as string);
        i++;
      }
      blocks.push({ kind: "code", language: fence[2] ?? "", text: body.join("\n") });
      continue;
    }
    if (line.trim() === "") {
      endParagraph();
      continue;
    }
    if (RULE.test(line)) {
      endParagraph();
      blocks.push({ kind: "rule" });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      endParagraph();
      blocks.push({
        kind: "heading",
        level: (heading[1] as string).length,
        spans: inline(heading[2] as string),
      });
      continue;
    }
    if (TABLE_ROW.test(line) && TABLE_RULE.test(lines[i + 1] ?? "")) {
      endParagraph();
      const header = cells(line).map(inline);
      const rows: Span[][][] = [];
      i += 2;
      while (i < lines.length && TABLE_ROW.test(lines[i] as string)) {
        rows.push(cells(lines[i] as string).map(inline));
        i++;
      }
      i--;
      blocks.push({ kind: "table", header, rows });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = NUMBERED.exec(line);
    if (bullet || numbered) {
      endParagraph();
      const ordered = !bullet;
      const start = numbered ? Number(numbered[1]) : 1;
      const items: string[] = [];
      for (; i < lines.length; i++) {
        const current = lines[i] as string;
        const item = ordered ? NUMBERED.exec(current) : BULLET.exec(current);
        if (item) {
          items.push((ordered ? item[2] : item[1]) as string);
        } else if (current.trim() !== "" && /^\s{2,}/.test(current) && items.length > 0) {
          // A wrapped line, or a nested item: kept with the item above it.
          const nested = BULLET.exec(current) ?? NUMBERED.exec(current);
          const words = nested ? (nested[2] ?? nested[1]) : current.trim();
          items[items.length - 1] += nested ? ` · ${words}` : ` ${words}`;
        } else {
          break;
        }
      }
      i--;
      blocks.push({ kind: "list", ordered, start, items: items.map(inline) });
      continue;
    }
    const quote = QUOTE.exec(line);
    if (quote) {
      endParagraph();
      const said: string[] = [];
      for (; i < lines.length && QUOTE.test(lines[i] as string); i++) {
        said.push((QUOTE.exec(lines[i] as string) as RegExpExecArray)[1] as string);
      }
      i--;
      blocks.push({ kind: "quote", spans: inline(said.join(" ")) });
      continue;
    }
    paragraph.push(line.trim());
  }
  endParagraph();
  return blocks;
}

function cells(row: string): string[] {
  return row
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

const INLINE =
  /(`+)([\s\S]*?[^`])\1(?!`)|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|\[([^\]]+)\]\(([^)\s]+)\)/g;

/** The spans of one line: code, bold, italics and links; the rest as written. */
export function inline(text: string): Span[] {
  const spans: Span[] = [];
  let at = 0;
  for (const match of text.matchAll(INLINE)) {
    const index = match.index ?? 0;
    if (index > at) spans.push({ text: text.slice(at, index) });
    if (match[1] !== undefined)
      spans.push({ text: (match[2] as string).trim() || (match[2] as string), code: true });
    else if (match[3] !== undefined) spans.push({ text: match[3], bold: true });
    else if (match[4] !== undefined) spans.push({ text: match[4], bold: true });
    else if (match[5] !== undefined) spans.push({ text: match[5], italic: true });
    else if (match[6] !== undefined) spans.push({ text: match[6], href: match[7] as string });
    at = index + match[0].length;
  }
  if (at < text.length) spans.push({ text: text.slice(at) });
  return spans;
}

/** How many lines `text` takes when written out: what a long reply is folded by. */
export function lineCount(text: string): number {
  return text.split("\n").length;
}

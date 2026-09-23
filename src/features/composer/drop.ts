/**
 * How a dropped file is written into the composer's message.
 *
 * The message is the agent's prompt, passed through as text, so a path is copied the way the
 * filesystem spells it. A path with whitespace or a quote is double-quoted so it stays one path.
 * A drop on a terminal is quoted for the shell instead (`droppedPathsText`): that paste is input
 * to a program, and this one is not.
 */
export function droppedPromptPaths(paths: string[]): string {
  return paths.map(quotePromptPath).join(" ") + (paths.length > 0 ? " " : "");
}

function quotePromptPath(path: string): string {
  if (!/[\s"]/.test(path)) return path;
  return `"${path.replace(/"/g, '\\"')}"`;
}

/**
 * Insert dropped paths at a selection in the message. A trailing space is left so typing can
 * continue, and a leading one when the insertion would otherwise run into the word before it.
 */
export function insertDroppedPaths(
  message: string,
  start: number,
  end: number,
  paths: string[],
): { message: string; caret: number } {
  if (paths.length === 0) return { message, caret: end };
  const before = message.slice(0, start);
  const gap = before.length > 0 && !/\s$/.test(before) ? " " : "";
  const text = gap + droppedPromptPaths(paths);
  return {
    message: before + text + message.slice(end),
    caret: before.length + text.length,
  };
}

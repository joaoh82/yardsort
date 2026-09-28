import type { Diagnostic } from "@codemirror/lint";
import type { Text } from "@codemirror/state";
import type { Problem, Step } from "@/lib/ipc";

/** What each action does, in the words the chart uses. */
export const ACTION_WORDS: Record<Step["action"], string> = {
  start_session: "Start an agent",
  wait_session: "Wait for it to settle",
  send_to_session: "Send a message",
  wait_pr_activity: "Wait for the pull request",
  notify: "Notify you",
};

/** Where a problem is, as an offset into the document; a problem with no line marks the first. */
export function diagnostics(doc: Text, problems: Problem[]): Diagnostic[] {
  return problems.map((problem) => {
    const line = doc.line(Math.min(Math.max(problem.line ?? 1, 1), doc.lines));
    const from = Math.min(line.from + Math.max((problem.column ?? 1) - 1, 0), line.to);
    return { from, to: Math.max(from, line.to), severity: "error", message: problem.message };
  });
}

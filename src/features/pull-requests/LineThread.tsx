import { lazy, Suspense } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { age } from "./rows";
import { placeOf, type Thread } from "./threads";

const Markdown = lazy(() => import("@/lib/Markdown").then((m) => ({ default: m.Markdown })));

/** One comment on lines, and what was said under it. Read-only: replying is the forge's, still. */
export function LineThread({ thread, now }: { thread: Thread; now: number }) {
  return (
    <article
      aria-label={`Comment on ${placeOf(thread.root)}`}
      className="my-1 rounded border border-line bg-surface text-[13px] font-sans"
    >
      {[thread.root, ...thread.replies].map((comment, index) => (
        <div key={comment.id} className={`px-3 py-2 ${index > 0 ? "border-t border-line" : ""}`}>
          <p className="flex items-baseline gap-1.5 text-[12px]">
            <span className="min-w-0 truncate font-medium">{comment.author ?? "ghost"}</span>
            {index === 0 && <span className="shrink-0 text-ink-faint">{placeOf(comment)}</span>}
            <span
              className="shrink-0 text-ink-faint"
              title={new Date(comment.at ?? 0).toLocaleString()}
            >
              {age(comment.at ?? 0, now)} ago
            </span>
            {comment.url && (
              <button
                type="button"
                aria-label="Open on GitHub"
                title="Open on GitHub"
                onClick={() => void openUrl(comment.url!).catch(console.error)}
                className="ml-auto shrink-0 text-ink-faint hover:text-ink"
              >
                <span aria-hidden>↗</span>
              </button>
            )}
          </p>
          <div className="mt-1">
            <Suspense
              fallback={<p className="whitespace-pre-wrap text-ink-muted">{comment.body}</p>}
            >
              <Markdown text={comment.body} />
            </Suspense>
          </div>
        </div>
      ))}
    </article>
  );
}

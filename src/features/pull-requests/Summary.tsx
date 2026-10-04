import { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { PullRequest, PullRequestCheck, PullRequestPost } from "@/lib/ipc";
import { usePullRequestsStore, type Target } from "@/stores/pullRequests";
import { PullRequestDetails } from "./PullRequestDetails";
import { checksColour } from "./appearance";
import { age, checksHeadline, type Row } from "./rows";

// The Markdown renderer is a fair amount of code that nobody needs until they open a pull
// request, so it arrives then — as CodeMirror does for the first file.
const Markdown = lazy(() => import("@/lib/Markdown").then((m) => ({ default: m.Markdown })));

const heading = "text-[11px] font-semibold tracking-wider text-ink-muted uppercase";

const checkWord = { none: "no result", running: "running", passing: "passed", failing: "failed" };
/** What needs attention first. */
const checkOrder = { failing: 0, running: 1, none: 2, passing: 3 };

const verdict = {
  approved: "approved",
  changesRequested: "requested changes",
  commented: "commented",
  dismissed: "review dismissed",
} as const;

/**
 * A pull request in full: what it is, what its author says it is, what CI and the reviewers
 * made of it, and what has been said. Read-only; replying is the forge's, for now.
 *
 * The top of it comes from the list, which is already here. The rest is asked for when the row
 * is opened and again whenever the list says the pull request changed.
 */
export function Summary({ row, now }: { row: Row; now: number }) {
  const { pr } = row;
  const state = usePullRequestsStore((s) => s.summaries[row.key]);
  const projectId = row.project.id;
  const target: Target = { key: row.key, projectId, pr, viewer: row.viewer };

  useEffect(() => {
    void usePullRequestsStore.getState().loadSummary({ key: row.key, projectId, pr, viewer: null });
    // `pr` is a new object on every poll; `loadSummary` itself compares what matters in it.
  }, [row.key, projectId, pr]);

  const summary = state?.summary ?? null;
  // The pull request as just read has every check by name and link; until it arrives, the
  // list's own copy has the counts.
  const fresh = summary?.pullRequest ?? pr;
  const waiting = !summary && (state?.loading ?? true);

  return (
    <div className="space-y-5">
      <PullRequestDetails pr={pr} link={false} checks={false} />

      {state?.error && (
        <div role="alert" className="flex items-start gap-2 text-red-400">
          <p className="min-w-0 flex-1 break-words select-text">
            Could not read this pull request: {state.error}
          </p>
          <button
            type="button"
            onClick={() => void usePullRequestsStore.getState().loadSummary(target, true)}
            className="shrink-0 text-ink-muted underline hover:text-ink"
          >
            Retry
          </button>
        </div>
      )}

      <Section label="Description">
        {waiting ? (
          <Faint>Loading…</Faint>
        ) : !summary ? null : summary.body.trim() === "" ? (
          <Faint>No description.</Faint>
        ) : (
          <Words text={summary.body} />
        )}
      </Section>

      <Checks pr={fresh} />
      <Reviewers pr={fresh} />

      <Section label="Conversation">
        {waiting ? (
          <Faint>Loading…</Faint>
        ) : !summary ? null : summary.posts.length === 0 ? (
          <Faint>Nobody has commented or reviewed yet.</Faint>
        ) : (
          <ol className="mt-2 space-y-3">
            {summary.posts.map((post, index) => (
              <Post key={`${post.kind}-${post.at}-${index}`} post={post} now={now} />
            ))}
          </ol>
        )}
        {summary && (
          <p className="mt-3 text-[11px] text-ink-faint">
            Comments on particular lines are under <b>Code</b>, beside the lines they are about.
          </p>
        )}
        {summary && <Reply target={target} />}
      </Section>
    </div>
  );
}

/** A comment on the conversation. Sending is the confirmation: nothing is posted any other way. */
function Reply({ target }: { target: Target }) {
  const [text, setText] = useState("");
  const busy = usePullRequestsStore((s) => s.busy !== null);
  const ready = text.trim() !== "" && !busy;
  const send = async () => {
    if (!ready) return;
    const sent = await usePullRequestsStore.getState().comment(target, text.trim());
    if (sent) setText("");
  };
  return (
    <form
      aria-label="Reply"
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
      className="mt-3"
    >
      <textarea
        aria-label="Your comment"
        placeholder="Write a comment…"
        value={text}
        rows={text.includes("\n") ? 5 : 2}
        disabled={busy}
        spellCheck
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          }
        }}
        className="w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
      />
      <div className="mt-1.5 flex items-center justify-end gap-3 text-[11px] text-ink-faint">
        <span>Markdown, as on GitHub. Ctrl+Enter or ⌘Enter sends.</span>
        <button
          type="submit"
          disabled={!ready}
          className="rounded bg-accent px-3 py-1 font-medium text-canvas disabled:opacity-40"
        >
          Comment
        </button>
      </div>
    </form>
  );
}

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section aria-label={label} className="border-t border-line pt-3">
      <h3 className={heading}>{label}</h3>
      {children}
    </section>
  );
}

const Faint = ({ children }: { children: ReactNode }) => (
  <p className="mt-2 text-ink-faint">{children}</p>
);

/** Markdown, with the plain text in its place for the moment the renderer takes to arrive. */
function Words({ text }: { text: string }) {
  return (
    <div className="mt-2">
      <Suspense fallback={<p className="whitespace-pre-wrap text-ink-muted">{text}</p>}>
        <Markdown text={text} />
      </Suspense>
    </div>
  );
}

/** Every check with its workflow, how it went and its run; failures first. */
function Checks({ pr }: { pr: PullRequest }) {
  const details = pr.details;
  const checks = [...(details?.checks ?? [])].sort(
    (a, b) => checkOrder[a.state] - checkOrder[b.state],
  );
  return (
    <Section label="Checks">
      <p className={`mt-2 ${checksColour[pr.checks]}`}>{checksHeadline(details?.checkCounts)}</p>
      {checks.length > 0 && (
        <ul className="mt-2 space-y-1.5">
          {checks.map((check, index) => (
            <Check key={`${check.workflow ?? ""}/${check.name}/${index}`} check={check} />
          ))}
        </ul>
      )}
    </Section>
  );
}

function Check({ check }: { check: PullRequestCheck }) {
  const name = (
    <>
      {check.workflow && <span className="text-ink-faint">{check.workflow} / </span>}
      {check.name}
    </>
  );
  return (
    <li className="flex items-baseline justify-between gap-3">
      {check.url ? (
        <button
          type="button"
          aria-label={`${check.workflow ? `${check.workflow} / ` : ""}${check.name}: open its run on GitHub`}
          title="Open this run on GitHub"
          onClick={() => void openUrl(check.url!).catch(console.error)}
          className="min-w-0 text-left break-words hover:underline"
        >
          {name} <span aria-hidden>↗</span>
        </button>
      ) : (
        <span className="min-w-0 break-words">{name}</span>
      )}
      <span className={`shrink-0 ${checksColour[check.state]}`}>{checkWord[check.state]}</span>
    </li>
  );
}

/** Who was asked for a review and has not answered, and each reviewer's latest word. */
function Reviewers({ pr }: { pr: PullRequest }) {
  const details = pr.details;
  if (!details) return null;
  const { reviewRequests, reviews } = details;
  return (
    <Section label="Reviewers">
      {reviewRequests.length + reviews.length === 0 ? (
        <Faint>Nobody has been asked, and nobody has reviewed it.</Faint>
      ) : (
        <ul className="mt-2 space-y-1">
          {reviews.map((review) => (
            <li key={`review-${review.login}`} className="flex justify-between gap-3">
              <span className="min-w-0 truncate">{review.login}</span>
              <span
                className={`shrink-0 ${
                  review.state === "approved"
                    ? "text-green-400"
                    : review.state === "changesRequested"
                      ? "text-red-400"
                      : "text-ink-muted"
                }`}
              >
                {verdict[review.state]}
              </span>
            </li>
          ))}
          {reviewRequests.map((request) => (
            <li
              key={`request-${request.team ? "team" : "user"}-${request.name}`}
              className="flex justify-between gap-3"
            >
              <span className="min-w-0 truncate">
                {request.team && <span className="text-ink-faint">team </span>}
                {request.name}
              </span>
              <span className="shrink-0 text-amber-300">review requested</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/** What someone did, in the words a timeline uses. */
function did(post: PullRequestPost): string {
  if (post.kind === "comment") return "commented";
  if (post.review === "approved") return "approved";
  if (post.review === "changesRequested") return "requested changes";
  if (post.review === "dismissed") return "reviewed, since dismissed";
  return "reviewed";
}

function Post({ post, now }: { post: PullRequestPost; now: number }) {
  const colour =
    post.review === "approved"
      ? "text-green-400"
      : post.review === "changesRequested"
        ? "text-red-400"
        : "text-ink-muted";
  const body = post.body.trim();
  // Always a number: the bindings only say "or null" because JSON has no NaN.
  const at = post.at ?? 0;
  return (
    <li className="rounded border border-line">
      <p className="flex items-baseline gap-1.5 border-b border-line bg-raised px-3 py-1.5 text-[12px]">
        <span className="min-w-0 truncate font-medium">{post.author ?? "ghost"}</span>
        <span className={`shrink-0 ${colour}`}>{did(post)}</span>
        <span className="shrink-0 text-ink-faint" title={new Date(at).toLocaleString()}>
          {age(at, now) === "now" ? "just now" : `${age(at, now)} ago`}
        </span>
        {post.url && (
          <button
            type="button"
            aria-label="Open on GitHub"
            title="Open on GitHub"
            onClick={() => void openUrl(post.url!).catch(console.error)}
            className="ml-auto shrink-0 text-ink-faint hover:text-ink"
          >
            <span aria-hidden>↗</span>
          </button>
        )}
      </p>
      <div className="px-3 py-2">
        {post.hidden !== null ? (
          <p className="text-ink-faint italic">
            Hidden on GitHub{post.hidden ? ` as ${post.hidden.replace(/_/g, " ")}` : ""}.
          </p>
        ) : body !== "" ? (
          <Suspense fallback={<p className="whitespace-pre-wrap text-ink-muted">{body}</p>}>
            <Markdown text={body} />
          </Suspense>
        ) : post.kind === "review" && post.review === "commented" ? (
          // A review with nothing in it but "commented" is one whose words are all on lines.
          <p className="text-ink-faint">Left comments on lines of the code.</p>
        ) : (
          <p className="text-ink-faint">Nothing further.</p>
        )}
      </div>
    </li>
  );
}

import Link from "next/link";
import { Shot } from "@/components/shot";
import { docUrl } from "@/lib/docs";

// The badges Assist puts on a changed file, as the guide describes them.
const BADGES = [
  { badge: "off-task", body: "Looks unrelated to what this workspace was asked to do." },
  { badge: "secret", body: "Looks like it adds a literal key, token or password." },
  { badge: "tests", body: "Looks like it deletes, skips or weakens a test." },
  { badge: "checks", body: "Looks like it switches a lint, type check or CI step off." },
  {
    badge: "credentials",
    body: "The file's name says it holds credentials — decided on your machine, contents never sent.",
  },
];

const code = "font-mono text-[13.5px] text-ink";

export function Assist() {
  return (
    <section aria-labelledby="h-assist" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <div className="grid items-start gap-12 md:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <div className="md:order-2">
          <p className="mb-2.5 font-mono text-[12.5px] text-accent">06 · Assist</p>
          <h3 id="h-assist" className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">
            A second pair of eyes, powered by Jev
          </h3>
          <div className="mt-3.5 space-y-3 text-muted">
            <p>
              Yardsort never parses what an agent prints. A diff, though, is text a model can be
              asked about — so Assist asks{" "}
              <a href="https://docs.typesafe.ai" className="link text-ink">
                Jev
              </a>
              , TypeSafe&apos;s judgment model, about each changed file, and badges the ones worth a
              look.
            </p>
            <p>
              Jev never generates text. It answers a <em>typed</em> question about a piece of state
              — a yes/no probability, a choice among named options, a position on ordered levels —
              and Yardsort decides what the number means. The questions stay narrow, one property
              each; the thresholds live in your settings; the model is pinned (
              <code className={code}>jev-1.13.0</code>) so a new version cannot move under them.
            </p>
            <p>
              In the composer it can also suggest a harness and an effort, built on{" "}
              <strong className="font-medium text-ink">your</strong> own &ldquo;Good at&rdquo;
              descriptions of your agents rather than on any opinion of ours. Nothing is ever picked
              for you.
            </p>
            <p>
              Optional and off by default: it does nothing until you enter your own TypeSafe API key
              — kept in your system credential store, never in a file — and tick a feature. What
              leaves the machine is a changed file&apos;s diff and the task, or the message you are
              typing. Never a terminal. With no key, switched off or offline, Yardsort works exactly
              as it does otherwise, minus a few badges.
            </p>
            <p>
              <Link href={`${docUrl("guide/assist")}/`} className="link text-ink">
                Assist
              </Link>{" "}
              covers the thresholds, the costs and what to do when TypeSafe says no.
            </p>
          </div>
        </div>
        <div className="md:order-1">
          <Shot
            name="assist"
            alt="Settings → Assist: the API key, the two features and the three thresholds"
          />
          <dl className="mt-6 grid gap-2.5 text-[15px]">
            {BADGES.map((item) => (
              <div key={item.badge} className="flex gap-3">
                <dt className="shrink-0 rounded-[5px] border border-line bg-raised px-2 py-0.5 font-mono text-[12px] text-ink">
                  {item.badge}
                </dt>
                <dd className="text-muted">{item.body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </div>
    </section>
  );
}

import Link from "next/link";
import type { ReactNode } from "react";
import { GitHubMark, Stars } from "@/components/github-mark";
import { docUrl } from "@/lib/docs";
import { changelogSummary, REPO, REPO_URL, repoFile, starCount } from "@/lib/site";

const h2 = "text-2xl leading-[1.2] font-medium tracking-[-0.015em]";

function Doc({ slug, children }: { slug: string; children: ReactNode }) {
  return <Link href={`${docUrl(slug)}/`}>{children}</Link>;
}

export async function OpenSource() {
  const stars = await starCount();
  return (
    <section aria-labelledby="h-oss" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <div className="grid gap-10 border-t border-line pt-12 md:grid-cols-3">
        <div>
          <h2 id="h-oss" className={h2}>
            Free software
          </h2>
          <p className="mt-3 text-[15px] text-muted">
            GPL-3.0. You may use, study, share and change it, and versions you distribute must stay
            free under the same terms.
          </p>
          <p className="mt-3 text-[15px] text-muted">
            Early, and moving fast. The core loop works on all three platforms and is covered by CI
            on each. Expect rough edges, and please{" "}
            <a href={`${REPO_URL}/issues/new/choose`} className="link text-ink">
              report them
            </a>
            .
          </p>
          <a
            href={REPO_URL}
            className="mt-5 inline-flex items-center gap-2.5 rounded-md border border-line bg-surface px-3.5 py-[9px] text-sm font-medium"
          >
            <GitHubMark />
            {REPO}
            {!!stars && (
              <span className="border-l border-line pl-2.5 font-normal text-muted">
                <Stars count={stars} />
              </span>
            )}
          </a>
        </div>
        <div>
          <h2 className={h2}>Docs</h2>
          <ul className="mt-3 grid gap-2 text-[15px]">
            <li>
              <Doc slug="quick-start">Quick start</Doc>{" "}
              <span className="text-faint">— download to first agent in five minutes</span>
            </li>
            <li>
              <Doc slug="guide/projects">Projects</Doc> ·{" "}
              <Doc slug="guide/workspaces">Workspaces</Doc>
            </li>
            <li>
              <Doc slug="guide/terminals-and-sessions">Terminals &amp; sessions</Doc>
            </li>
            <li>
              <Doc slug="guide/changes-and-files">Changes &amp; files</Doc>
            </li>
            <li>
              <Doc slug="guide/settings">Settings &amp; harnesses</Doc> ·{" "}
              <Doc slug="guide/assist">Assist</Doc>
            </li>
            <li>
              <Doc slug="guide/cli">The ys command line</Doc>
            </li>
            <li>
              <Doc slug="guide/updates">Updates</Doc> ·{" "}
              <Doc slug="guide/shortcuts">Keyboard shortcuts</Doc>
            </li>
            <li>
              <Doc slug="guide/troubleshooting">Troubleshooting</Doc> ·{" "}
              <Doc slug="roadmap">Roadmap</Doc> ·{" "}
              <a href={repoFile("docs/design/README.md")}>Design docs</a>
            </li>
          </ul>
        </div>
        <div>
          <h2 className={h2}>Changelog</h2>
          <dl className="mt-3 grid gap-3.5 text-[15px]">
            {changelogSummary().map((entry) => (
              <div key={entry.version}>
                <dt className="font-mono text-[12.5px] text-muted">{entry.version}</dt>
                <dd className="mt-0.5 text-muted">{entry.summary}</dd>
              </div>
            ))}
          </dl>
          <Link href="/changelog/" className="link mt-3.5 inline-block text-sm">
            Full changelog
          </Link>
        </div>
      </div>
    </section>
  );
}

import Image from "next/image";
import Link from "next/link";
import { CONTACT_EMAIL, REPO_URL, repoFile } from "@/lib/site";

export function SiteFooter() {
  return (
    <footer className="mx-auto max-w-[1120px] px-5 pt-20 pb-10 md:px-8">
      <div className="flex flex-col flex-wrap items-stretch justify-between gap-6 border-t border-line pt-6 text-[13.5px] text-muted md:flex-row md:items-start">
        <div className="flex items-center gap-2">
          <Image src="/icon.svg" alt="" width={18} height={18} className="rounded" />
          <span className="font-medium text-ink">yardsort</span>
          <span className="font-mono text-xs">yardsort.sh</span>
        </div>
        <nav aria-label="Footer" className="flex flex-wrap gap-[18px]">
          <a href={REPO_URL}>GitHub</a>
          <a href={`mailto:${CONTACT_EMAIL}`}>Contact</a>
          <Link href="/docs/">Docs</Link>
          <Link href="/changelog/">Changelog</Link>
          <a href={repoFile("CONTRIBUTING.md")}>Contributing</a>
          <a href={repoFile("SECURITY.md")}>Security</a>
          <a href={repoFile("LICENSE")}>GPL-3.0</a>
        </nav>
      </div>
      <p className="mt-6 text-[13px] text-muted">
        Questions, support or anything else —{" "}
        <a href={`mailto:${CONTACT_EMAIL}`} className="link">
          {CONTACT_EMAIL}
        </a>
        .
      </p>
      <p className="mt-3 max-w-[720px] text-[13px] text-faint">
        Yardsort is an independent project, not affiliated with Anthropic, OpenAI, xAI or any other
        maker of the agents it can launch. Product names belong to their owners.
      </p>
    </footer>
  );
}

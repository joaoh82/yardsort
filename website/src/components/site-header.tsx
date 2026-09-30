import Image from "next/image";
import Link from "next/link";
import { REPO_URL, starCount } from "@/lib/site";
import { GitHubMark, Stars } from "./github-mark";

export async function SiteHeader() {
  const stars = await starCount();
  return (
    <header className="mx-auto flex max-w-[1120px] flex-wrap items-center justify-between gap-x-6 gap-y-4 px-5 py-[18px] md:px-8">
      <Link href="/" aria-label="Yardsort home" className="flex items-center gap-2.5">
        <Image src="/icon.svg" alt="" width={26} height={26} className="block rounded-md" />
        <span className="text-[17px] font-medium tracking-[-0.01em]">yardsort</span>
      </Link>
      <nav
        aria-label="Main"
        className="flex flex-wrap items-center gap-x-5 gap-y-3 text-sm text-muted"
      >
        <Link href="/docs/">Docs</Link>
        <Link href="/tutorials/">Tutorials</Link>
        <Link href="/blog/">Blog</Link>
        <Link href="/changelog/">Changelog</Link>
        <a href={REPO_URL} aria-label="Yardsort on GitHub" className="flex items-center gap-1.5">
          <GitHubMark />
          <Stars count={stars} />
        </a>
        <Link
          href="/#install"
          className="rounded-md bg-accent px-3.5 py-2 text-sm font-medium text-accent-ink hover:text-accent-ink hover:opacity-90"
        >
          Download
        </Link>
      </nav>
    </header>
  );
}

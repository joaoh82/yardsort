import Image from "next/image";
import Link from "next/link";
import { REPO_URL, starCount } from "@/lib/site";
import { GitHubMark, Stars } from "./github-mark";

export async function SiteHeader() {
  const stars = await starCount();
  return (
    <header className="mx-auto flex max-w-[1120px] items-center justify-between gap-6 px-5 py-[18px] md:px-8">
      <Link href="/" aria-label="Yardsort home" className="flex items-center gap-2.5">
        <Image src="/icon.svg" alt="" width={26} height={26} className="block rounded-md" />
        <span className="text-[17px] font-medium tracking-[-0.01em]">yardsort</span>
      </Link>
      <nav aria-label="Main" className="flex items-center gap-[22px] text-sm text-muted">
        <Link className="max-md:hidden" href="/docs/">
          Docs
        </Link>
        <Link className="max-md:hidden" href="/changelog/">
          Changelog
        </Link>
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

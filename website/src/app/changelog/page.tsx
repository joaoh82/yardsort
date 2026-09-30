import type { Metadata } from "next";
import { mdxComponents } from "@/components/mdx-components";
import { renderMarkdown } from "@/lib/render-doc";
import { readChangelog } from "@/lib/site";
import { pageMetadata } from "@/lib/seo";

export const metadata: Metadata = pageMetadata(
  "Changelog · Yardsort",
  "See what's new in Yardsort: release notes, new features, improvements and fixes for the desktop app and ys command line.",
  "/changelog",
);

// CHANGELOG.md from the repository root, rendered as it is.
export default async function ChangelogPage() {
  const { title, content } = await renderMarkdown(readChangelog(), mdxComponents);
  return (
    <article className="mx-auto max-w-[760px] px-5 py-14 md:px-8">
      <h1 className="text-[36px] leading-[1.1] font-medium tracking-[-0.02em]">{title}</h1>
      <div className="prose mt-6 max-w-none">{content}</div>
    </article>
  );
}

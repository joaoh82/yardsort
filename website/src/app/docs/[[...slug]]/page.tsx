import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { ALL_DOCS, DEFAULT_DOC, docUrl, neighbours, readDoc, REPO_URL } from "@/lib/docs";
import { renderDoc } from "@/lib/render-doc";
import { mdxComponents } from "@/components/mdx-components";

type Props = { params: Promise<{ slug?: string[] }> };

export const dynamicParams = false;

export function generateStaticParams() {
  return ALL_DOCS.map((d) => ({ slug: d.slug === DEFAULT_DOC ? [] : d.slug.split("/") }));
}

function slugOf(parts?: string[]) {
  return parts?.length ? parts.join("/") : DEFAULT_DOC;
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const slug = slugOf((await params).slug);
  const entry = ALL_DOCS.find((d) => d.slug === slug);
  return { title: entry ? `${entry.title} · Docs` : "Docs" };
}

export default async function DocPage({ params }: Props) {
  const slug = slugOf((await params).slug);
  const doc = readDoc(slug);
  if (!doc) notFound();
  const { title, content } = await renderDoc(doc, mdxComponents);
  const { prev, next } = neighbours(slug);
  const editUrl = `${REPO_URL}/edit/main/docs/${doc.path}.${doc.format}`;

  return (
    <article className="min-w-0 max-w-[760px]">
      <h1 className="text-[36px] leading-[1.1] font-medium tracking-[-0.02em]">{title}</h1>
      <div className="prose mt-6 max-w-none">{content}</div>
      <footer className="mt-14 flex flex-wrap items-center justify-between gap-4 border-t border-line pt-6 text-sm text-muted">
        <div className="flex flex-wrap gap-x-6 gap-y-2">
          {prev && <Link href={docUrl(prev.slug)}>← {prev.title}</Link>}
          {next && <Link href={docUrl(next.slug)}>{next.title} →</Link>}
        </div>
        <a href={editUrl} className="link">
          Edit this page on GitHub
        </a>
      </footer>
    </article>
  );
}

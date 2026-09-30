import Link from "next/link";
import {
  articlePath,
  articleSchema,
  ARTICLE_AUTHOR,
  displayDate,
  readArticle,
  SECTIONS,
  type Article,
} from "@/lib/articles";
import { renderMarkdown } from "@/lib/render-doc";
import { Breadcrumbs } from "./breadcrumbs";
import { JsonLd } from "./json-ld";
import { mdxComponents } from "./mdx-components";

export async function ArticlePage({ article }: { article: Article }) {
  const { content } = await renderMarkdown(readArticle(article), mdxComponents);
  return (
    <article className="mx-auto max-w-[824px] px-5 pt-10 md:px-8 md:pt-16">
      <Breadcrumbs
        items={[
          { name: "Home", path: "/" },
          { name: SECTIONS[article.section].title, path: `/${article.section}/` },
          { name: article.title, path: articlePath(article) },
        ]}
      />
      <JsonLd data={articleSchema(article)} />
      <header className="border-b border-line pb-8">
        <p className="text-sm font-medium text-accent">{article.category}</p>
        <h1 className="mt-4 text-4xl leading-[1.12] font-medium tracking-tight md:text-5xl">
          {article.title}
        </h1>
        <p className="mt-5 text-lg text-muted">{article.description}</p>
        <p className="mt-6 text-sm text-muted">
          By{" "}
          <Link className="link" href="/">
            {ARTICLE_AUTHOR.name}
          </Link>{" "}
          · Published <time dateTime={article.published}>{displayDate(article.published)}</time>
          {article.updated !== article.published && (
            <>
              {" "}
              · Updated <time dateTime={article.updated}>{displayDate(article.updated)}</time>
            </>
          )}
        </p>
        {article.verified && (
          <p className="mt-2 text-sm text-muted">
            Last verified: <time dateTime={article.verified}>{displayDate(article.verified)}</time>.
            Written by the Yardsort project using the official sources linked below; not a hands-on
            benchmark.
          </p>
        )}
      </header>
      <div className="prose mt-8 max-w-none break-words">{content}</div>
      <footer className="mt-12 flex flex-wrap justify-between gap-4 border-t border-line pt-6 text-sm">
        <Link className="link" href={`/${article.section}/`}>
          ← All {SECTIONS[article.section].title.toLowerCase()}
        </Link>
        <Link className="link" href="/docs/">
          Get started with Yardsort →
        </Link>
      </footer>
    </article>
  );
}

import Link from "next/link";
import { Breadcrumbs } from "./breadcrumbs";
import {
  articlePath,
  articlesIn,
  displayDate,
  SECTIONS,
  type ArticleSection,
} from "@/lib/articles";

export function ArticleIndex({ section }: { section: ArticleSection }) {
  const info = SECTIONS[section];
  return (
    <div className="mx-auto max-w-[1000px] px-5 pt-10 md:px-8 md:pt-16">
      <Breadcrumbs
        items={[
          { name: "Home", path: "/" },
          { name: info.title, path: `/${section}/` },
        ]}
      />
      <h1 className="text-4xl font-medium tracking-tight md:text-5xl">{info.title}</h1>
      <p className="mt-5 max-w-[620px] text-lg text-muted">{info.description}</p>
      <div className="mt-10 grid gap-6 md:grid-cols-2">
        {articlesIn(section).map((article) => (
          <article
            key={article.slug}
            className="flex flex-col rounded-xl border border-line bg-surface p-6 md:p-8"
          >
            <p className="text-xs font-medium tracking-wide text-accent">{article.category}</p>
            <h2 className="mt-4 text-2xl leading-tight font-medium tracking-tight">
              <Link href={articlePath(article)}>{article.title}</Link>
            </h2>
            <p className="mt-4 flex-1 text-muted">{article.description}</p>
            <p className="mt-6 text-sm text-muted">
              <time dateTime={article.published}>{displayDate(article.published)}</time> · Yardsort
            </p>
          </article>
        ))}
      </div>
      <p className="mt-10 text-muted">
        {section === "blog" ? "Ready to try it? " : "Looking for background and comparisons? "}
        <Link className="link" href={section === "blog" ? "/tutorials/" : "/blog/"}>
          {section === "blog" ? "Follow a tutorial →" : "Read the blog →"}
        </Link>
      </p>
    </div>
  );
}

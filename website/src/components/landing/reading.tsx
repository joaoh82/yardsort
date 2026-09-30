import Link from "next/link";
import { articlePath, ARTICLES } from "@/lib/articles";

export function Reading() {
  const featured = [
    ARTICLES.find((a) => a.section === "tutorials")!,
    ARTICLES.find((a) => a.slug === "why-we-built-yardsort")!,
  ];
  return (
    <section className="mx-auto mt-20 max-w-[1000px] px-5 md:px-8" aria-labelledby="reading-title">
      <h2 id="reading-title" className="text-3xl font-medium tracking-tight">
        Put parallel agents to work
      </h2>
      <div className="mt-6 grid gap-6 md:grid-cols-2">
        {featured.map((article) => (
          <article key={article.slug} className="rounded-xl border border-line p-6">
            <p className="text-xs font-medium text-accent">{article.category}</p>
            <h3 className="mt-3 text-xl font-medium">
              <Link href={articlePath(article)}>{article.title}</Link>
            </h3>
            <p className="mt-3 text-muted">{article.description}</p>
          </article>
        ))}
      </div>
      <p className="mt-6 flex gap-6 text-sm">
        <Link className="link" href="/tutorials/">
          All tutorials →
        </Link>
        <Link className="link" href="/blog/">
          Blog and comparisons →
        </Link>
      </p>
    </section>
  );
}

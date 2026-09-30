import fs from "node:fs";
import path from "node:path";
import type { Metadata } from "next";
import catalog from "../../content/articles.json";
import { pageMetadata, pageUrl, SOCIAL_IMAGE } from "./seo";

export type ArticleSection = "blog" | "tutorials";
export type Article = {
  section: ArticleSection;
  slug: string;
  title: string;
  description: string;
  category: string;
  published: string;
  updated: string;
  verified?: string;
};
export const ARTICLES: Article[] = catalog as Article[];
export const ARTICLE_AUTHOR = { name: "Yardsort", url: pageUrl("/") };
export const SECTIONS = {
  blog: {
    title: "Blog",
    description:
      "Product decisions, practical ideas for parallel coding, and comparisons with Superset and Conductor.",
  },
  tutorials: {
    title: "Tutorials",
    description:
      "Follow complete coding workflows in Yardsort, from your first parallel agents to reviewed pull requests.",
  },
};
export const articlePath = (article: Article) => `/${article.section}/${article.slug}/`;
export const articlesIn = (section: ArticleSection) =>
  ARTICLES.filter((a) => a.section === section);
export const findArticle = (section: ArticleSection, slug: string) =>
  ARTICLES.find((a) => a.section === section && a.slug === slug);
export function readArticle(article: Article) {
  return fs.readFileSync(
    path.join(process.cwd(), "content", article.section, `${article.slug}.md`),
    "utf8",
  );
}
export function articleMetadata(article: Article): Metadata {
  const metadata = pageMetadata(
    `${article.title} · Yardsort`,
    article.description,
    articlePath(article),
  );
  return {
    ...metadata,
    authors: [ARTICLE_AUTHOR],
    openGraph: {
      ...metadata.openGraph,
      type: "article",
      publishedTime: article.published,
      modifiedTime: article.updated,
      authors: [ARTICLE_AUTHOR.url],
    },
  };
}
export function articleSchema(article: Article) {
  return {
    "@context": "https://schema.org",
    "@type": article.section === "blog" ? "BlogPosting" : "TechArticle",
    headline: article.title,
    description: article.description,
    url: pageUrl(articlePath(article)),
    mainEntityOfPage: pageUrl(articlePath(article)),
    datePublished: article.published,
    dateModified: article.updated,
    author: { "@type": "Organization", ...ARTICLE_AUTHOR },
    publisher: { "@type": "Organization", ...ARTICLE_AUTHOR },
    image: SOCIAL_IMAGE.url,
  };
}
export function displayDate(date: string) {
  return new Intl.DateTimeFormat("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  }).format(new Date(date));
}

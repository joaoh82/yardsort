import type { MetadataRoute } from "next";
import { ALL_DOCS, docUrl } from "@/lib/docs";
import { ARTICLES, articlePath } from "@/lib/articles";
import { pageUrl } from "@/lib/seo";

export const dynamic = "force-static";

export default function sitemap(): MetadataRoute.Sitemap {
  return [
    "/",
    "/changelog",
    "/blog",
    "/tutorials",
    ...ARTICLES.map(articlePath),
    ...ALL_DOCS.map((entry) => docUrl(entry.slug)),
  ].map((path) => ({
    url: pageUrl(path),
  }));
}

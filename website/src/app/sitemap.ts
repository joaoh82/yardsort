import type { MetadataRoute } from "next";
import { ALL_DOCS, docUrl } from "@/lib/docs";
import { pageUrl } from "@/lib/seo";

export const dynamic = "force-static";

export default function sitemap(): MetadataRoute.Sitemap {
  return ["/", "/changelog", ...ALL_DOCS.map((entry) => docUrl(entry.slug))].map((path) => ({
    url: pageUrl(path),
  }));
}

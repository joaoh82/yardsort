import { ArticleIndex } from "@/components/article-index";
import { SECTIONS } from "@/lib/articles";
import { pageMetadata } from "@/lib/seo";

export const metadata = pageMetadata(
  `${SECTIONS.blog.title} · Yardsort`,
  SECTIONS.blog.description,
  "/blog/",
);
export default function Page() {
  return <ArticleIndex section="blog" />;
}

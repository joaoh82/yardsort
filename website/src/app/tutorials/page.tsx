import { ArticleIndex } from "@/components/article-index";
import { SECTIONS } from "@/lib/articles";
import { pageMetadata } from "@/lib/seo";

export const metadata = pageMetadata(
  `${SECTIONS.tutorials.title} · Yardsort`,
  SECTIONS.tutorials.description,
  "/tutorials/",
);
export default function Page() {
  return <ArticleIndex section="tutorials" />;
}

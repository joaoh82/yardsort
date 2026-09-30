import { notFound } from "next/navigation";
import { ArticlePage } from "@/components/article-page";
import { articleMetadata, articlesIn, findArticle } from "@/lib/articles";

type Props = { params: Promise<{ slug: string }> };
export const dynamicParams = false;
export function generateStaticParams() {
  return articlesIn("blog").map((a) => ({ slug: a.slug }));
}
export async function generateMetadata({ params }: Props) {
  const article = findArticle("blog", (await params).slug);
  if (!article) notFound();
  return articleMetadata(article);
}
export default async function Page({ params }: Props) {
  const article = findArticle("blog", (await params).slug);
  if (!article) notFound();
  return <ArticlePage article={article} />;
}

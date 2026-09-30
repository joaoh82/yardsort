import Link from "next/link";
import { readDoc } from "@/lib/docs";
import { renderDoc } from "@/lib/render-doc";
import { mdxComponents } from "@/components/mdx-components";

export async function Questions() {
  const doc = readDoc("guide/questions");
  if (!doc) throw new Error("Missing About Yardsort guide");
  const { content } = await renderDoc(doc, {
    ...mdxComponents,
    // The same Markdown uses h2 on its own page, h3 beneath this section's h2.
    h2: (props) => <h3 {...props} />,
  });
  return (
    <section
      id="questions"
      aria-labelledby="h-questions"
      className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8"
    >
      <h2
        id="h-questions"
        className="text-[26px] leading-[1.15] font-medium tracking-[-0.02em] md:text-[32px]"
      >
        Questions about Yardsort
      </h2>
      <div className="prose mt-8 max-w-[760px]">{content}</div>
      <p className="mt-6 text-sm text-muted">
        <Link href="/docs/guide/questions/" className="link">
          Read these answers in the docs →
        </Link>
      </p>
    </section>
  );
}

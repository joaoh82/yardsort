import { evaluate } from "@mdx-js/mdx";
import type { MDXComponents } from "mdx/types";
import type { ReactNode } from "react";
import * as runtime from "react/jsx-runtime";
import rehypeSlug from "rehype-slug";
import remarkGfm from "remark-gfm";
import { visit } from "unist-util-visit";
import { resolveDocLink, type DocSource } from "./docs";

type MdNode = { type: string; url?: string; depth?: number; children?: MdNode[]; value?: string };

function text(node: MdNode): string {
  return node.value ?? (node.children ?? []).map(text).join("");
}

// Rewrites GitHub-style relative links and lifts the first `# Heading` out as the page title, so
// the layout can render it.
function remarkDocs(from: string, out: { title?: string }) {
  return () => (tree: MdNode) => {
    visit(tree as never, (node: MdNode) => {
      if ((node.type === "link" || node.type === "image") && node.url) {
        node.url = resolveDocLink(node.url, from);
      }
    });
    const first = tree.children?.findIndex((n) => n.type === "heading" && n.depth === 1) ?? -1;
    if (first >= 0 && tree.children) {
      out.title = text(tree.children[first]);
      tree.children.splice(first, 1);
    }
  };
}

export async function renderDoc(
  doc: DocSource,
  components: MDXComponents,
): Promise<{ title?: string; content: ReactNode }> {
  const out: { title?: string } = {};
  const { default: Content } = await evaluate(doc.source, {
    ...runtime,
    format: doc.format,
    remarkPlugins: [remarkGfm, remarkDocs(doc.path, out)],
    rehypePlugins: [rehypeSlug],
  });
  return { title: doc.title ?? out.title, content: <Content components={components} /> };
}

// Plain Markdown from the repository root (the changelog). The slug makes its relative links
// resolve from there.
export async function renderMarkdown(
  source: string,
  components: MDXComponents,
): Promise<{ title?: string; content: ReactNode }> {
  return renderDoc({ slug: "", path: "../CHANGELOG", format: "md", source }, components);
}

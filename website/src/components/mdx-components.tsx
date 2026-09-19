import type { MDXComponents } from "mdx/types";
import Link from "next/link";

// What Markdown elements turn into, and the components an .mdx page can use without importing.
export const mdxComponents: MDXComponents = {
  a: ({ href = "", children, ...rest }) =>
    href.startsWith("/") ? (
      <Link href={href}>{children}</Link>
    ) : (
      <a href={href} {...rest}>
        {children}
      </a>
    ),
  // eslint-disable-next-line @next/next/no-img-element -- static export, images are plain files
  img: ({ alt = "", ...rest }) => <img alt={alt} loading="lazy" {...rest} />,
};

import fs from "node:fs";
import path from "node:path";

// The documentation is the repo's own docs/ folder: one source of truth, read on GitHub and
// rendered here. A page can be .md (plain Markdown, exactly as GitHub shows it) or .mdx (Markdown
// plus React components).
export const DOCS_DIR = path.resolve(process.cwd(), "..", "docs");
export const REPO_URL = "https://github.com/joaoh82/yardsort";

export type DocEntry = { slug: string; title: string };
export type DocSection = { title: string; items: DocEntry[] };

// Sidebar order. Add a page here after adding its file under docs/.
export const DOCS_NAV: DocSection[] = [
  {
    title: "Start",
    items: [{ slug: "quick-start", title: "Quick start" }],
  },
  {
    title: "Guide",
    items: [
      { slug: "guide/projects", title: "Projects" },
      { slug: "guide/workspaces", title: "Workspaces" },
      { slug: "guide/terminals-and-sessions", title: "Terminals & sessions" },
      { slug: "guide/changes-and-files", title: "Changes & files" },
      { slug: "guide/settings", title: "Settings & harnesses" },
      { slug: "guide/updates", title: "Updates" },
      { slug: "guide/shortcuts", title: "Shortcuts" },
      { slug: "guide/troubleshooting", title: "Troubleshooting" },
    ],
  },
];

export const ALL_DOCS: DocEntry[] = DOCS_NAV.flatMap((s) => s.items);
export const DEFAULT_DOC = "quick-start";

export function docUrl(slug: string): string {
  return slug === DEFAULT_DOC ? "/docs" : `/docs/${slug}`;
}

export type DocSource = { slug: string; file: string; format: "md" | "mdx"; source: string };

export function readDoc(slug: string): DocSource | null {
  if (!ALL_DOCS.some((d) => d.slug === slug)) return null;
  for (const format of ["mdx", "md"] as const) {
    const file = path.join(DOCS_DIR, `${slug}.${format}`);
    if (fs.existsSync(file)) {
      return { slug, file, format, source: fs.readFileSync(file, "utf8") };
    }
  }
  return null;
}

export function neighbours(slug: string): { prev?: DocEntry; next?: DocEntry } {
  const i = ALL_DOCS.findIndex((d) => d.slug === slug);
  return { prev: ALL_DOCS[i - 1], next: ALL_DOCS[i + 1] };
}

// Links in docs/ are written so they work on GitHub (`workspaces.md`, `../images/x.png`,
// `../CONTRIBUTING.md`). Turn each into the right thing for the site.
export function resolveDocLink(href: string, fromSlug: string): string {
  if (/^([a-z][a-z0-9+.-]*:|\/\/|#|\/)/i.test(href)) return href;
  const [target, hash = ""] = href.split("#");
  const anchor = hash ? `#${hash}` : "";
  // Where the link points, as a path from the repository root.
  const repoPath = path.posix.normalize(
    path.posix.join("docs", path.posix.dirname(fromSlug), target),
  );

  if (repoPath.startsWith("docs/images/")) {
    return `/docs-images/${repoPath.slice("docs/images/".length)}`;
  }
  const slug = repoPath.replace(/^docs\//, "").replace(/\.mdx?$/, "");
  if (repoPath.startsWith("docs/") && ALL_DOCS.some((d) => d.slug === slug)) {
    return `${docUrl(slug)}${anchor}`;
  }
  // Anything else lives in the repository, not on the site.
  return `${REPO_URL}/blob/main/${repoPath}${anchor}`;
}

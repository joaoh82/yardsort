import fs from "node:fs";
import path from "node:path";

// The documentation is the repo's own docs/ folder: one source of truth, read on GitHub and
// rendered here. A page can be .md (plain Markdown, exactly as GitHub shows it) or .mdx (Markdown
// plus React components).
export const DOCS_DIR = path.resolve(process.cwd(), "..", "docs");
export const REPO_URL = "https://github.com/joaoh82/yardsort";

// `slug` is the page's address on the site. `path` is its file under docs/, without the
// extension, when that differs from the slug; the page then takes its title from here too.
export type DocEntry = { slug: string; title: string; description: string; path?: string };
export type DocSection = { title: string; items: DocEntry[] };

// Sidebar order. Add a page here after adding its file under docs/.
export const DOCS_NAV: DocSection[] = [
  {
    title: "Start",
    items: [
      {
        slug: "quick-start",
        title: "Quick start",
        description:
          "Install Yardsort on Linux, macOS or Windows, connect a working agent CLI, open a project and start your first task in its own git worktree.",
      },
    ],
  },
  {
    title: "Guide",
    items: [
      {
        slug: "guide/projects",
        title: "Projects",
        description:
          "Open, create or clone git repositories in Yardsort. Find projects, configure their settings and organize the workspaces inside them.",
      },
      {
        slug: "guide/workspaces",
        title: "Workspaces",
        description:
          "Create an isolated git worktree for each coding task, choose an agent, and manage, archive or delete workspaces in Yardsort.",
      },
      {
        slug: "guide/terminals-and-sessions",
        title: "Terminals & sessions",
        description:
          "Run coding agents in real terminals, resume or fork sessions, hand off work, and keep agents running when you close the Yardsort window.",
      },
      {
        slug: "guide/changes-and-files",
        title: "Changes & files",
        description:
          "Review git changes and diffs, browse files, preview images and edit code inside a Yardsort workspace before committing your work.",
      },
      {
        slug: "guide/commits-and-pull-requests",
        title: "Commits & pull requests",
        description:
          "Commit workspace changes, push your branch and open a GitHub pull request from Yardsort using git and the GitHub CLI.",
      },
      {
        slug: "guide/settings",
        title: "Settings & harnesses",
        description:
          "Configure Yardsort settings and agent harnesses, including commands, argument templates, models and custom terminal coding agents.",
      },
      {
        slug: "guide/assist",
        title: "Assist",
        description:
          "Set up optional Assist with TypeSafe Jev for change checks and composer suggestions. Learn what is sent, how billing works and what stays local.",
      },
      {
        slug: "guide/activity",
        title: "Activity",
        description:
          "Understand the local activity Yardsort records, opt-in agent events and file attribution, and how to inspect or export the record.",
      },
      {
        slug: "guide/memory",
        title: "Memory",
        description:
          "Keep project lessons in Yardsort, review agent proposals and choose which approved notes are shared with agents in future sessions.",
      },
      {
        slug: "guide/outcomes",
        title: "Outcomes",
        description:
          "Record whether workspace results were kept, partly kept or discarded, and review how coding agents perform on your projects in Yardsort.",
      },
      {
        slug: "guide/workflows",
        title: "Workflows",
        description:
          "Create reusable agent workflows in YAML, inspect their steps, run code reviews and follow workflow runs in Yardsort or through ys.",
      },
      {
        slug: "guide/updates",
        title: "Updates",
        description:
          "Check for Yardsort releases, configure automatic update checks and learn how installation and updates differ across Linux, macOS and Windows.",
      },
      {
        slug: "guide/shortcuts",
        title: "Shortcuts",
        description:
          "Find Yardsort keyboard shortcuts for projects, workspaces, terminals and settings on Linux, macOS and Windows.",
      },
      {
        slug: "guide/cli",
        title: "The ys command line",
        description:
          "Install and use the ys command line to manage Yardsort projects and workspaces, start agents and inspect activity from your terminal.",
      },
      {
        slug: "guide/troubleshooting",
        title: "Troubleshooting",
        description:
          "Troubleshoot Yardsort startup, database version mismatches, missing agents and terminal problems with practical checks and recovery steps.",
      },
    ],
  },
  {
    title: "Project",
    items: [
      {
        slug: "roadmap",
        title: "Roadmap",
        description:
          "Explore the Yardsort roadmap: completed milestones, planned features and open work for parallel coding agents, workspaces and workflows.",
        path: "design/05-roadmap",
      },
    ],
  },
];

export const ALL_DOCS: DocEntry[] = DOCS_NAV.flatMap((s) => s.items);
export const DEFAULT_DOC = "quick-start";

export function docUrl(slug: string): string {
  return slug === DEFAULT_DOC ? "/docs" : `/docs/${slug}`;
}

export function docPath(entry: DocEntry): string {
  return entry.path ?? entry.slug;
}

// `path` is the file under docs/ without its extension: what relative links resolve from.
export type DocSource = {
  slug: string;
  path: string;
  title?: string;
  format: "md" | "mdx";
  source: string;
};

export function readDoc(slug: string): DocSource | null {
  const entry = ALL_DOCS.find((d) => d.slug === slug);
  if (!entry) return null;
  for (const format of ["mdx", "md"] as const) {
    const file = path.join(DOCS_DIR, `${docPath(entry)}.${format}`);
    if (fs.existsSync(file)) {
      const title = entry.path ? entry.title : undefined;
      return { slug, path: docPath(entry), title, format, source: fs.readFileSync(file, "utf8") };
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
export function resolveDocLink(href: string, fromPath: string): string {
  if (/^([a-z][a-z0-9+.-]*:|\/\/|#|\/)/i.test(href)) return href;
  const [target, hash = ""] = href.split("#");
  const anchor = hash ? `#${hash}` : "";
  // Where the link points, as a path from the repository root.
  const repoPath = path.posix.normalize(
    path.posix.join("docs", path.posix.dirname(fromPath), target),
  );

  if (repoPath.startsWith("docs/images/")) {
    return `/docs-images/${repoPath.slice("docs/images/".length)}`;
  }
  const docFile = repoPath.replace(/^docs\//, "").replace(/\.mdx?$/, "");
  const entry = ALL_DOCS.find((d) => docPath(d) === docFile);
  if (repoPath.startsWith("docs/") && entry) return `${docUrl(entry.slug)}${anchor}`;
  // Anything else lives in the repository, not on the site.
  return `${REPO_URL}/blob/main/${repoPath}${anchor}`;
}

import type { Metadata } from "next";

// Match the production redirect: yardsort.sh redirects to www.yardsort.sh.
export const SITE_ORIGIN = "https://www.yardsort.sh";
export const HOME_TITLE = "Yardsort — parallel coding tasks, any agent";
export const HOME_DESCRIPTION =
  "Yardsort is an open-source desktop app for Linux, macOS and Windows. Every task gets its own git worktree; every agent gets its own terminal. Run Claude Code, Codex and other agents side by side — in one worktree or across many.";

export const SOCIAL_IMAGE = {
  url: `${SITE_ORIGIN}/social/yardsort-v1.png`,
  width: 1200,
  height: 630,
  type: "image/png",
  alt: "Yardsort — parallel coding tasks, any agent. An app preview shows workspaces, an agent terminal and a code diff.",
};

// Page URLs follow next.config.ts's trailingSlash policy. Asset URLs do not use this helper.
export function pageUrl(path: string): string {
  return `${SITE_ORIGIN}${path.replace(/\/+$/, "")}/`;
}

export function pageMetadata(title: string, description: string, path: string): Metadata {
  const url = pageUrl(path);
  return {
    title: { absolute: title },
    description,
    alternates: { canonical: url },
    openGraph: {
      type: "website",
      siteName: "Yardsort",
      title,
      description,
      url,
      images: [SOCIAL_IMAGE],
    },
    twitter: {
      card: "summary_large_image",
      title,
      description,
      images: [{ url: SOCIAL_IMAGE.url, alt: SOCIAL_IMAGE.alt }],
    },
  };
}

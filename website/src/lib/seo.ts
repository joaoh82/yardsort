import type { Metadata } from "next";

// Match the production redirect: yardsort.sh redirects to www.yardsort.sh.
export const SITE_ORIGIN = "https://www.yardsort.sh";
export const HOME_TITLE = "Yardsort — orchestrate coding agents on any OS";
export const HOME_DESCRIPTION =
  "Open-source desktop for Linux, macOS and Windows. Orchestrate Claude Code, Codex and any CLI agent in parallel — shared worktrees when you want, handoff with context so you do not re-explain the branch.";

export const SOCIAL_IMAGE = {
  url: `${SITE_ORIGIN}/social/yardsort-v1.png`,
  width: 1200,
  height: 630,
  type: "image/png",
  alt: "Yardsort — orchestrate coding agents on any OS. An app preview shows workspaces, an agent terminal and a code diff.",
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

import fs from "node:fs";
import path from "node:path";

export const REPO = "joaoh82/yardsort";
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases/latest`;
export const BREW_COMMAND = "brew install --cask joaoh82/yardsort/yardsort";

const REPO_ROOT = path.resolve(process.cwd(), "..");

export function repoFile(relative: string): string {
  return `${REPO_URL}/blob/main/${relative}`;
}

// The version the site advertises is the app's own, read at build time.
export function appVersion(): string {
  const conf = JSON.parse(
    fs.readFileSync(path.join(REPO_ROOT, "src-tauri", "tauri.conf.json"), "utf8"),
  ) as { version: string };
  return conf.version;
}

// Stars at build time. No number is better than a wrong or invented one, so any failure
// (offline build, rate limit) just leaves it out.
export async function starCount(): Promise<number | null> {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    const data = (await res.json()) as { stargazers_count?: number };
    return typeof data.stargazers_count === "number" ? data.stargazers_count : null;
  } catch {
    return null;
  }
}

export function readChangelog(): string {
  return fs.readFileSync(path.join(REPO_ROOT, "CHANGELOG.md"), "utf8");
}

export type ChangelogEntry = { version: string; summary: string };

function plain(markdown: string): string {
  return markdown
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function firstSentence(text: string): string {
  const end = text.search(/[.:](\s|$)/);
  return (end > 0 ? text.slice(0, end) : text).trim() + ".";
}

// The newest entries of CHANGELOG.md, each boiled down to the opening sentence of its first two
// bullets, for the landing page.
export function changelogSummary(limit = 4): ChangelogEntry[] {
  const sections = readChangelog().split(/^## /m).slice(1);
  return sections.slice(0, limit).map((section) => {
    const [heading, ...rest] = section.split("\n");
    const bullets = rest
      .join("\n")
      .split(/^- /m)
      .slice(1)
      .map((b) => firstSentence(plain(b)));
    const summary = bullets.length
      ? bullets.slice(0, 2).join(" ")
      : firstSentence(plain(rest.join(" ")));
    return { version: heading.trim(), summary };
  });
}

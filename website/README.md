# The Yardsort website

The site at [yardsort.sh](https://yardsort.sh): the landing page, documentation, changelog, blog and tutorials. It is its own application — its own `package.json`, its own lockfile — and shares
nothing with the desktop app's build. It lives in this repository so that one change can update the
app, its docs and the site together.

Next.js (App Router) + React + Tailwind, built as a **fully static export**: `bun run build` writes
plain files to `out/`, which any static host can serve.

```sh
cd website
bun install
bun run dev          # http://localhost:3000
bun run build        # static site in out/
bun run lint && bun run typecheck
```

From the repository root: `just site-dev`, `just site-build`, `just site-check`.

## Where the content comes from

Nothing about the product is written twice. At build time the site reads:

| On the site               | Source                                                                                    |
| ------------------------- | ----------------------------------------------------------------------------------------- |
| `/docs/…`                 | `../docs/quick-start.md`, `../docs/guide/*` and the roadmap — the same files GitHub shows |
| `/changelog/`             | `../CHANGELOG.md`; the landing page shows its newest four entries                         |
| `/blog/…`, `/tutorials/…` | `content/articles.json` and Markdown under `content/blog/` and `content/tutorials/`       |
| Version in the hero       | `../src-tauri/tauri.conf.json`                                                            |
| Screenshots               | `../docs/images/`, copied to `public/docs-images/` before dev and build                   |
| GitHub stars              | The GitHub API, at build time; left out if the request fails or it is 0                   |

The landing page's own copy lives here, in `src/components/landing/`. It came from the README;
when the README's claims change (platforms, install methods, what is pending), change it too.

## Writing documentation

A page is a file under `../docs/`:

- **`.md`** — plain Markdown, rendered exactly as GitHub renders it. Use this by default.
- **`.mdx`** — Markdown plus React components, for pages that need more than text. Components
  listed in `src/components/mdx-components.tsx` can be used without importing. GitHub shows `.mdx`
  as source, so keep pages that people read there as `.md`.

To add a page: create the file, then add one line to `DOCS_NAV` in `src/lib/docs.ts` (the sidebar
and the order of the previous/next links), including a unique `description` for search and link
previews. An entry can give a `path` when the file should not
decide the address: the roadmap is `docs/design/05-roadmap.md`, shown at `/docs/roadmap/`. Write links the way they work on GitHub
(`workspaces.md#anchor`, `../images/x.png`, `../CONTRIBUTING.md`); the site rewrites them, and
sends anything outside the docs to GitHub.

## Writing blog posts and tutorials

Add a plain Markdown file to `content/blog/` or `content/tutorials/`, then register it in
`content/articles.json`. The registry supplies its slug, title, description, category, publication
date and last substantive update. Comparisons also have a `verified` date: update it only after
checking the cited official sources. Set publication dates to the actual launch date before merging.
Keep dates in `YYYY-MM-DD` format. Entries are displayed in registry order; unregistered drafts
are not exported. Only reviewed entries belong in the registry.

Use absolute site paths for internal links (such as `/docs/guide/workspaces/`) and full HTTPS
URLs for external sources. Markdown is compiled at build time with the existing docs renderer;
article routes have no client-side content fetch. The visible title and metadata come from the
registry. The renderer removes the Markdown H1, leaving one page H1.

Both sections have static index and detail pages, breadcrumbs, unique metadata, article social
tags, organizational author attribution and `BlogPosting`/`TechArticle` structured data.
The existing branded share card is shared across them. The sitemap includes both indexes and
all registered articles. Keep article dates, attribution and schema consistent with visible text.

Comparisons disclose that Yardsort is the publisher and distinguish documentation research from
hands-on testing. Link claims to official sources, include competitors' strengths, distinguish
paid plans from provider costs, and label roadmap features as plans. Do not invent performance
results, personal stories, endorsements or availability. The initial product story incorporates
the creator's account; Sync is explicitly described as in development.

## Search and link previews

The homepage's questions render `docs/guide/questions.md`, also published as the About Yardsort
guide. Edit that Markdown to keep both versions in sync. Answers stay visible without JavaScript.

`src/lib/structured-data.ts` supplies the homepage's `WebSite` and `SoftwareApplication` JSON-LD:
the release version comes from the app configuration, and costs, platforms and prerequisites
match visible content. It includes no ratings or reviews and does not claim eligibility for a
software rich result. `Breadcrumbs` renders each documentation trail and its `BreadcrumbList`
from one list, with no duplicate Quick start/Docs destination. `JsonLd` escapes `<` when serializing
data into a script element. After changes, check schema with the Schema.org validator and Google's
Rich Results Test as well as the exported-HTML checks; appearance in search is not guaranteed.

`src/lib/seo.ts` defines the production origin, shared image and page metadata helper. The
canonical origin is `https://www.yardsort.sh`, matching the existing bare-domain redirect on
Vercel. Each page supplies its own canonical, description and Open Graph/Twitter title;
documentation descriptions live alongside their titles in `DOCS_NAV`. Page URLs use trailing
slashes, matching the static export. Keep the hosting redirect and these URLs consistent.

`src/app/sitemap.ts` generates `/sitemap.xml` from the docs and article registries, plus their indexes, the homepage and
changelog. Quick start is `/docs/`, not `/docs/quick-start/`. `src/app/robots.ts` allows public
crawling and points to the sitemap. Both files are generated at build time and need no server.

The 1200 × 630 social card is committed at `public/social/yardsort-v1.png`. Its editable layout
is `scripts/generate-social-image.tsx`, using `next/og` with the existing logo and demo screenshot.
Run `bun run social-image` to regenerate it, then inspect the PNG at full and thumbnail sizes.
It is deliberately separate from normal builds: changing a documentation screenshot does not
silently replace the share card. For a published replacement, use a new versioned filename in
the generator and `SOCIAL_IMAGE` to distinguish it from cached previews.

Every `bun run build` runs `scripts/check-seo.mjs` afterward. It checks exported HTML for unique
titles/descriptions, self-canonicals, matching OG/Twitter fields, image dimensions and alt text,
404 noindex, and sitemap coverage. It also parses JSON-LD, compares breadcrumb names and URLs to
their visible navigation, checks the product version, and compares the homepage answers with the
About Yardsort guide. `just site-check` also runs lint and types. After deployment,
verify live redirects, discovery files and previews in the target sharing services; their caches
can retain older cards. Submit the sitemap in the site's webmaster tools when access is available.

## Design

Colours are the desktop app's own tokens (`src/app/globals.css`): dark by default, light when the
system asks for it. Geist for text, JetBrains Mono for commands, versions and labels. 8px grid,
6px radii on controls and 10px on cards and screenshots, 1px lines, no shadows except under the
hero screenshot. The page reads correctly without JavaScript; scripts only add platform detection,
the copy buttons and analytics.

## Analytics

The site — never the desktop app — reports to PostHog (EU cloud). `src/instrumentation-client.ts`
starts it, and only when `NEXT_PUBLIC_POSTHOG_KEY` is set at build time, so local builds and forks
send nothing and show no banner.

A cookie banner (`src/components/cookie-consent.tsx`) asks first, and nothing is captured until the
visitor answers. **Accept** sets PostHog's cookies and allows pageviews, clicks and session replay
(when replay is switched on in the PostHog project). **Decline** sets no cookies and leaves
anonymous, cookieless pageviews, which PostHog drops unless cookieless tracking is switched on in
the project. PostHog stores the answer itself, in local storage; **Cookie settings** in the footer
asks again. Before an answer PostHog still loads the project's configuration and feature flags,
which session replay needs, but records no event and stores nothing in the browser. In production the browser talks to `yardsort.sh/ingest`, which `vercel.json` rewrites to
PostHog, so blockers of posthog.com don't drop events; a host other than Vercel needs the same
rewrites. They match with regular expressions because a pattern like `/ingest/:path*` does not match
PostHog's paths, which end in a slash (`/e/`, `/flags/`). The `config/` rule removes the slash that
`trailingSlash` adds to `/array/<key>/config`, which PostHog serves only without it. Under `bun run dev` with the key set, it talks to PostHog directly.

## Deploying

Deployed on Vercel at [yardsort.sh](https://yardsort.sh); every push to `main` redeploys. Any static
host works. To set it up on Vercel: import the repository, set **Root Directory** to `website`, and
leave "Include files outside the root directory" on (the default) — the build reads `../docs`,
`../CHANGELOG.md` and `../src-tauri/tauri.conf.json`. Framework, install and build commands are
detected. Set `NEXT_PUBLIC_POSTHOG_KEY` (the PostHog project's `phc_…` key) under Environment
Variables for Production. The site shows the version and changelog as of its last build, so
redeploy after a release (a push to `main` does that).

# The Yardsort website

The site at [yardsort.sh](https://yardsort.sh): the landing page, the documentation and the
changelog. It is its own application — its own `package.json`, its own lockfile — and shares
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

| On the site         | Source                                                                       |
| ------------------- | ---------------------------------------------------------------------------- |
| `/docs/…`           | `../docs/quick-start.md` and `../docs/guide/*` — the same files GitHub shows |
| `/changelog/`       | `../CHANGELOG.md`; the landing page shows its newest four entries            |
| Version in the hero | `../src-tauri/tauri.conf.json`                                               |
| Screenshots         | `../docs/images/`, copied to `public/docs-images/` before dev and build      |
| GitHub stars        | The GitHub API, at build time; left out if the request fails or it is 0      |

Only the landing page's own copy lives here, in `src/components/landing/`. It came from the README;
when the README's claims change (platforms, install methods, what is pending), change it too.

## Writing documentation

A page is a file under `../docs/`:

- **`.md`** — plain Markdown, rendered exactly as GitHub renders it. Use this by default.
- **`.mdx`** — Markdown plus React components, for pages that need more than text. Components
  listed in `src/components/mdx-components.tsx` can be used without importing. GitHub shows `.mdx`
  as source, so keep pages that people read there as `.md`.

To add a page: create the file, then add one line to `DOCS_NAV` in `src/lib/docs.ts` (the sidebar
and the order of the previous/next links). Write links the way they work on GitHub
(`workspaces.md#anchor`, `../images/x.png`, `../CONTRIBUTING.md`); the site rewrites them, and
sends anything outside the docs to GitHub.

## Design

Colours are the desktop app's own tokens (`src/app/globals.css`): dark by default, light when the
system asks for it. Geist for text, JetBrains Mono for commands, versions and labels. 8px grid,
6px radii on controls and 10px on cards and screenshots, 1px lines, no shadows except under the
hero screenshot. The page reads correctly without JavaScript; scripts only add platform detection
and the copy buttons.

## Deploying

Any static host works. On Vercel: import the repository, set **Root Directory** to `website`, and
leave "Include files outside the root directory" on (the default) — the build reads `../docs`,
`../CHANGELOG.md` and `../src-tauri/tauri.conf.json`. Framework, install and build commands are
detected. The site shows the version and changelog as of its last build, so redeploy after a
release (a push to `main` does that).

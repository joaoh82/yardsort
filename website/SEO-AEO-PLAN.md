# Website SEO and AEO review

Reviewed 2026-09-30. Findings below describe the site at audit time. Phase 1 has since been
merged and deployed: page metadata, www canonicals, discovery files, a dedicated share card
and exported-HTML checks. Live metadata, discovery files and the image URL were verified;
individual sharing services may still cache their own previews.
Phase 2 is merged: shared visible product answers, direct
guide introductions, homepage product/site JSON-LD and documentation breadcrumbs. Phase 3 now has an initial blog and tutorial set: a creator story, a worktree explainer,
Superset and Conductor comparisons, and a worked parallel-agent tutorial. Search Console sitemap
submission and a successful live fetch were reported during rollout; indexing and traffic still
need measurement. Performance work and the monthly measurement review remain outstanding. AEO means making the site useful as a source for answer engines.

## Evidence and scope

Inspected the website source, downloaded live HTML for the homepage, workspaces guide and
changelog, checked HTTP responses for discovery files and URL variants, and downloaded and
visually inspected the declared social image. No Search Console, Bing Webmaster Tools,
analytics, real-user performance data or platform-specific cached previews were available.
Ranking, indexing coverage and traffic impact therefore remain unmeasured. Requests used a
normal HTTP client; actual crawler access still needs checking through webmaster tools.

The site has a good foundation: static exported HTML with readable content, descriptive
homepage title and H1, distinct documentation titles, internal navigation, screenshot alt
text, image dimensions and lazy loading below the hero. Keep these properties.

## Findings

| Priority         | Verified finding                                                                                                                                                                | Recommended action                                                                                                                                                                                                                        |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| High             | HTTPS bare-domain requests return 308 to `https://www.yardsort.sh/`; `metadataBase` and image URLs use `https://yardsort.sh`. Sampled pages have no canonical link or `og:url`. | Use one production origin consistently in redirects, canonicals, OG URLs, schema and sitemap. Keeping the currently served www origin is the lowest-change default; changing to bare-domain needs an intentional hosting redirect change. |
| High             | `/robots.txt` and `/sitemap.xml` return 404 after the hostname redirect.                                                                                                        | Publish crawl policy and a generated sitemap containing all public canonical pages. Missing robots.txt is not itself a crawl block; missing sitemap is a discovery and monitoring gap.                                                    |
| High             | Docs metadata only supplies a title. Workspaces and changelog live HTML repeat the homepage description and `og:title=Yardsort`.                                                | Supply a unique description, canonical, OG title/description/URL and Twitter title/description per page.                                                                                                                                  |
| High for sharing | The social image is `/docs-images/overview.png`, HTTP 200, PNG, 1875 × 1175, 792,477 bytes (about 774 KiB). It is a full application screenshot.                                | Create a dedicated share card whose branding and value proposition remain legible at thumbnail size.                                                                                                                                      |
| Medium           | No JSON-LD in sampled HTML or website source.                                                                                                                                   | Add truthful product/site entities and documentation breadcrumbs.                                                                                                                                                                         |
| Medium           | Product facts exist, but there is no dedicated concise question-and-answer section. Some feature headings favor metaphor over explicit task names.                              | Add direct answers and practical tutorials connected to existing guides.                                                                                                                                                                  |
| Investigate      | Screenshots are served as original PNGs; the hero alone is about 774 KiB.                                                                                                       | Measure mobile performance, then generate responsive WebP/AVIF derivatives if beneficial. Do not claim a Core Web Vitals failure without measurements.                                                                                    |

The current homepage title is **Yardsort — run AI coding agents in parallel**. Keep it; it
already states the product and task clearly. The description is accurate but could name the
brand, open-source status and representative agents more directly.

Twitter tags are **present in live HTML**, including `summary_large_image`, even though there
is no explicit Twitter object in the source. Next.js derives them from Open Graph metadata.
The gap is specificity and control, not missing Twitter support.

The OG tags omit `og:url`, `og:site_name`, image width, height and alt text. The
[Open Graph specification](https://ogp.me/) includes URL among its basic required properties
and recommends image alt text. Image dimensions also make the intended asset clearer.

## Social preview brief

Current asset: [overview screenshot](https://www.yardsort.sh/docs-images/overview.png).
The tiny window title is its main branding, and the terminal/diff text will be difficult to
read in a compact card. Its approximately 1.60:1 ratio also leaves presentation decisions to
services using wider cards. Actual crop and caching vary by service.

Create a dedicated 1200 × 630 PNG or JPEG, proposed path `/social/yardsort-v1.png`:

- Prominent Yardsort logo/name and **Run AI coding agents in parallel**.
- Supporting line: **One task. One git worktree. Your choice of agent.**
- A restrained crop of the demo UI, with sufficient space around essential text.
- A small Linux · macOS · Windows label; avoid a release number that rapidly becomes stale.
- Target under 300 KB if image quality permits; inspect at roughly 600 × 315 and 300 × 158.
- Use the sanitized demo profile for any new screenshot, following repository instructions.

Keep this separate from the documentation screenshot so documentation updates do not silently
redesign the social card. Declare its absolute URL, dimensions, type and descriptive alt in
OG metadata, and explicit Twitter large-image metadata. Start with one branded image shared
across pages and unique page titles/descriptions; page-specific cards can follow later.
Use a versioned asset URL when replacing it and recheck platform caches after deployment.

## Phase 1 — technical metadata and discovery

Estimated effort: 1–2 engineering days including the share card and validation.

1. Centralize production origin and shared metadata in a small website utility. Match the
   deployed hostname policy. Give every page its own self-canonical; never inherit a homepage
   canonical across all documentation. Align trailing-slash policy with the exported routes
   and actual host behavior. Google recommends consistent canonical signals across redirects,
   links and sitemaps: [canonical guidance](https://developers.google.com/search/docs/crawling-indexing/consolidate-duplicate-urls).
2. Extend `DocEntry` in `src/lib/docs.ts` with an explicit editorial description and, where
   useful, a search title distinct from the short sidebar label. Apply these in
   `src/app/docs/[[...slug]]/page.tsx`; add dedicated changelog metadata. Preserve shared image
   defaults when composing per-page OG objects.
3. Generate the sitemap from `ALL_DOCS`/`docUrl`, plus home and changelog: currently 19 pages
   (17 docs plus two). Quick start is `/docs/`; `/docs/quick-start/` currently returns 404 and
   must not enter the sitemap. Only add a redirect alias if needed. Use genuine modification
   dates or omit `lastmod`; do not stamp every page with each deployment time.
4. Add static-export-compatible robots and sitemap outputs. Allow public content and reference
   the canonical sitemap. Check hosting/WAF policy as well as robots; decide AI search access
   separately from training policy. Consult each provider's current crawler documentation
   during implementation instead of copying a speculative bot allowlist.
5. Build and wire the dedicated social card. Include `og:site_name=Yardsort`, page URL and
   explicit page-specific Twitter fields.

Example proposed homepage description:

> Yardsort is an open-source desktop app for running Claude Code, Codex and other AI coding agents in parallel git worktrees on Linux, macOS and Windows.

Example workspaces metadata:

- Title: **Git worktrees and workspaces · Yardsort Docs**
- Description: **Create an isolated git worktree for each coding task, choose an agent, and manage, archive or delete workspaces in Yardsort.**

Implementation must first read the installed Next.js metadata/static-export documentation as
required by `website/AGENTS.md`.

## Phase 2 — answer clarity and structured data

Estimated effort: 2–3 days for an initial content pass and schema.

Add a short definition near the beginning of the homepage that names Yardsort explicitly.
Add visible answers with links to the detailed guides:

- What is Yardsort, and who is it for?
- Which coding agents and operating systems does it support?
- Is Yardsort free? Distinguish the app's license from agent/provider costs.
- Does it include the agents, or do I install and authenticate them separately?
- How do separate git worktrees help agents work in parallel?
- Does work continue when I close the window?
- What stays local, and what can be sent to agent providers or optional Assist?

Verify privacy, process-lifecycle and pricing wording against the actual app and docs before
publishing. Begin relevant guides with a concise answer, then prerequisites, executable steps,
screenshots and limitations. Use task-oriented headings; retain the railway metaphor as
supporting copy. Add visible breadcrumbs and related guide links where they improve navigation.

Add `WebSite` and `SoftwareApplication` JSON-LD on the homepage with stable IDs, canonical URL,
name, description, image, operating systems, `DeveloperApplication` category, license and
verified repository reference. Add `BreadcrumbList` where it matches visible navigation.
Avoid manually duplicated release versions; use the existing version source if included.
Only publish prices, ratings or reviews supported by visible truthful content. Google requires
a rating or review for its software-app rich result; basic schema without one must not be
promised a rich result. See [software application guidance](https://developers.google.com/search/docs/appearance/structured-data/software-app).

For Google's AI features, there are no special AEO tags or required AI text files. Prioritize
indexable, linked, useful text and structured data consistent with it. Treat `llms.txt` as an
optional experiment, not a prerequisite or promised ranking improvement. See
[Google's AI search guidance](https://developers.google.com/search/docs/appearance/ai-features).

## Phase 3 — useful search content and measurement

Estimated effort: 1–2 weeks for the first small content set; then monthly review.

Start with these topic hypotheses, refining them using actual query data:

| Intent                                        | First content to improve or create                                                                      |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Run coding agents in parallel                 | Homepage and a practical multi-task tutorial                                                            |
| Use Claude Code and Codex with git worktrees  | One worked example with verified commands and screenshots                                               |
| Manage AI agent workspaces and review changes | Link workspaces, changes and pull-request guides into an end-to-end workflow                            |
| Install Yardsort on each platform             | Improve quick-start sections first; add separate pages only when they have substantial distinct content |

Prefer a few useful pages over many thin agent/platform variations. Add new guides to both
`docs/README.md` and `website/src/lib/docs.ts`. Keep README, landing copy and guides consistent.
Build discoverability through the GitHub README, release notes and accurate ecosystem listings;
external submissions or outreach are separate publishing actions.

Establish a baseline in Google Search Console and Bing Webmaster Tools, submit the sitemap,
and inspect representative URLs. Review indexing, non-brand impressions/clicks, query/page
CTR and download-link clicks after roughly 28 days and again after 8–12 weeks. Download-link
clicks are not confirmed installations. Track identifiable AI referral traffic and a small
repeatable question set as directional evidence; answers vary and attribution is incomplete.

Measure mobile Lighthouse/PageSpeed and available field data before performance work. Preserve
image dimensions and lazy loading, and optimize screenshot delivery based on evidence. No
traffic, rankings or answer citations are guaranteed by these changes.

## Acceptance criteria for implementation

- `just site-check` passes, including static export. Parse all exported pages to check unique
  titles/descriptions, correct self-canonicals, page-specific OG/Twitter fields and valid JSON-LD.
- Sitemap covers every intended public page once, uses the selected host/path convention and
  excludes errors/aliases. Its URLs and the share image resolve successfully after deployment.
- Robots and sitemap return 200 with appropriate content; missing URLs return genuine 404s.
- Redirects converge on one hostname without loops; crawler-visible HTML includes the content
  and metadata without JavaScript execution.
- Inspect the share card at small sizes and test real target services, such as LinkedIn,
  Slack, Discord and X, allowing for independent caches and crops.
- Validate schema with Schema.org Validator and Google's Rich Results Test; distinguish schema
  validity from eligibility for a particular Google result feature.
- Verify published behavior against updated website documentation and add the appropriate
  changelog entry when user-visible changes ship. Run required repository checks before commit.

Recommended order: **hostname/canonicals and page metadata → discovery files and share card →
answer content and schema → targeted tutorials and measurement**.

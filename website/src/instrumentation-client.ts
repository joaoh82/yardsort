import posthog from "posthog-js";

// Analytics for the website only; the desktop app sends nothing. A build without the key (local
// development, forks, previews without it set) loads no analytics at all.
const key = process.env.NEXT_PUBLIC_POSTHOG_KEY;

if (key) {
  posthog.init(key, {
    // Proxied through yardsort.sh by the rewrites in vercel.json, so blockers of posthog.com
    // don't drop events. Only Vercel serves /ingest; `next dev` talks to PostHog directly.
    api_host: process.env.NODE_ENV === "production" ? "/ingest" : "https://eu.i.posthog.com",
    ui_host: "https://eu.posthog.com",
    // Pageviews on client-side navigation, pageleave, autocapture and session replay (switched on
    // in the PostHog project's settings) all follow these defaults.
    defaults: "2026-08-30",
    // Nothing is captured until the visitor answers the cookie banner (cookie-consent.tsx).
    // Declining leaves anonymous, cookieless pageviews, which the PostHog project must have
    // switched on in its settings or they are dropped.
    cookieless_mode: "on_reject",
  });
}

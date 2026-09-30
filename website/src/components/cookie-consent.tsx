"use client";

import posthog from "posthog-js";
import { useSyncExternalStore } from "react";

// PostHog keeps the choice itself (see instrumentation-client.ts): until one is made nothing is
// captured, "Accept" allows cookies and session replay, "Decline" leaves cookieless counts only.
// Without the key there is no analytics, so no banner and no footer link.
export const ANALYTICS_ENABLED = Boolean(process.env.NEXT_PUBLIC_POSTHOG_KEY);

const listeners = new Set<() => void>();
let reopened = false;

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function bannerShown(): boolean {
  if (!ANALYTICS_ENABLED || !posthog.__loaded) return false;
  return reopened || posthog.get_explicit_consent_status() === "pending";
}

function choose(accepted: boolean) {
  if (accepted) posthog.opt_in_capturing();
  else posthog.opt_out_capturing();
  reopened = false;
  listeners.forEach((listener) => listener());
}

export function reopenCookieConsent() {
  reopened = true;
  listeners.forEach((listener) => listener());
}

export function CookieConsent() {
  // The static HTML never has the banner; it appears once the browser knows there is no choice.
  const shown = useSyncExternalStore(subscribe, bannerShown, () => false);
  if (!shown) return null;

  return (
    <section
      aria-label="Cookie consent"
      className="fixed inset-x-4 bottom-4 z-50 mx-auto max-w-[560px] rounded-[10px] border border-line bg-surface p-4 text-[13.5px] text-muted md:p-5"
    >
      <p>
        This site uses cookies to count visits and record sessions, which helps us see what to
        improve. Decline and it counts pageviews anonymously, without cookies. The Yardsort app
        itself sends nothing.
      </p>
      <div className="mt-4 flex flex-wrap justify-end gap-2">
        <button
          type="button"
          onClick={() => choose(false)}
          className="cursor-pointer rounded-md border border-line px-3.5 py-2 text-sm font-medium text-ink hover:bg-raised"
        >
          Decline
        </button>
        <button
          type="button"
          onClick={() => choose(true)}
          className="cursor-pointer rounded-md bg-accent px-3.5 py-2 text-sm font-medium text-accent-ink hover:opacity-90"
        >
          Accept
        </button>
      </div>
    </section>
  );
}

export function CookieSettingsLink() {
  if (!ANALYTICS_ENABLED) return null;
  return (
    <button
      type="button"
      onClick={reopenCookieConsent}
      className="cursor-pointer text-muted hover:text-ink"
    >
      Cookie settings
    </button>
  );
}

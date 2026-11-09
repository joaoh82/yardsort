import type { OtherHost, OtherLink } from "@/lib/ipc";

/** Where a pairing stands, in a few words, with the link when there is one. */
export function pairingText(host: OtherHost): string {
  switch (host.pairing) {
    case "waiting":
      return "waiting for its answer";
    case "refused":
      return "refused";
    case "lapsed":
      return "nobody answered in time";
    case "revoked":
      return "taken back";
    case "paired":
      return host.link ? linkText(host.link) : "watches";
  }
}

/** How a host's link stands, in a few words. */
export function linkText(link: OtherLink): string {
  switch (link.kind) {
    case "connecting":
      return "connecting…";
    case "online":
      return "online";
    case "offline":
      return `offline since ${clock(link.since)}`;
  }
}

/** A time today as hours and minutes, another day as its date too. */
export function clock(at: number | null): string {
  if (at === null) return "";
  const when = new Date(at);
  const today = new Date().toDateString() === when.toDateString();
  return today
    ? when.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : when.toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}

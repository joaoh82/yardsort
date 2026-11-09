import type { OtherHost } from "@/lib/ipc";
import { pairingText } from "./otherComputersText";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";

/**
 * Below the projects: the other computers this one reaches as a client, each by its name and how
 * it stands. Kept apart from the projects, so nothing of another computer is taken for this one's;
 * each opens read-only in the center panel. Nothing shows until this computer is paired with one.
 */
export function OtherComputersList() {
  const status = useOtherComputersStore((s) => s.status);
  const open = useProjectsStore((s) => s.remoteHostId);
  const hosts = status?.client ? status.hosts : [];
  const shown = hosts.filter((h) => h.pairing === "paired" || h.pairing === "waiting");
  if (shown.length === 0) return null;
  return (
    <section aria-label="Other computers" className="border-t border-line py-1">
      <h3 className="px-3 py-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
        Other computers
      </h3>
      <ul>
        {shown.map((host) => {
          const current = open === host.hostId;
          const paired = host.pairing === "paired";
          return (
            <li key={host.grantId}>
              <button
                type="button"
                disabled={!paired}
                aria-current={current ? "page" : undefined}
                title={paired ? `${host.name}, another computer` : pairingText(host)}
                onClick={() =>
                  useProjectsStore.getState().openRemoteHost(current ? null : host.hostId)
                }
                className={`flex h-8 w-full items-center gap-2 px-3 text-left hover:bg-raised disabled:cursor-default disabled:hover:bg-transparent ${
                  current ? "bg-raised text-ink" : "text-ink-muted"
                }`}
              >
                <ComputerIcon />
                <span className="min-w-0 flex-1 truncate">{host.name}</span>
                <span className="shrink-0 text-[11px] text-ink-faint">{shortState(host)}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function shortState(host: OtherHost): string {
  if (host.pairing === "waiting") return "waiting";
  if (!host.link) return "";
  if (host.link.kind === "offline") return "offline";
  return host.link.kind === "online" ? "online" : "…";
}

function ComputerIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-3.5 shrink-0"
    >
      <rect x="2" y="2.5" width="12" height="8.5" rx="1" />
      <path d="M6 13.5h4M8 11v2.5" />
    </svg>
  );
}

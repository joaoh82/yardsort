import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ComputerIcon } from "@/features/sidebar/OtherComputersTree";
import { useLayoutStore } from "@/stores/layout";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { whyNotThere } from "./there";

/**
 * Where a new workspace will run, as the last part of the composer's title: this computer, or one
 * of the computers it is paired with. Pressed, it lists them all with how each stands. One that
 * is away, or lets this computer only watch, is listed and cannot be chosen.
 */
export function RunOn({
  hostId,
  here,
  onChoose,
  disabled,
}: {
  /** The computer chosen now; `null` is this one. */
  hostId: string | null;
  /** Why this computer cannot be chosen from where the composer is, when it cannot. */
  here?: string | null;
  onChoose: (hostId: string | null) => void;
  disabled?: boolean;
}) {
  const status = useOtherComputersStore((s) => s.status);
  const views = useOtherComputersStore((s) => s.views);
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x: 0, y: 0 });

  const hosts = (status?.client ? status.hosts : []).filter((h) => h.pairing === "paired");
  const chosen = hosts.find((h) => h.hostId === hostId);
  const name = hostId === null ? "This computer" : (chosen?.name ?? "Another computer");

  // Under the button, and inside the window.
  useLayoutEffect(() => {
    if (!open || !button.current || !menu.current) return;
    const from = button.current.getBoundingClientRect();
    setAt({
      x: Math.max(4, Math.min(from.left, window.innerWidth - menu.current.offsetWidth - 4)),
      y: Math.max(4, Math.min(from.bottom + 4, window.innerHeight - menu.current.offsetHeight - 4)),
    });
    menu.current.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!menu.current?.contains(target) && !button.current?.contains(target)) close();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // The composer behind it gives up on Escape: this one closes the menu and no more.
      event.stopPropagation();
      close();
      button.current?.focus();
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", close);
    // It is placed once, under the button: when the window changes size it would be left behind.
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", close);
    };
  }, [open]);

  const moveFocus = (event: React.KeyboardEvent) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const rows = [
      ...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
    ];
    const index = rows.indexOf(document.activeElement as HTMLButtonElement);
    rows[(index + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length]?.focus();
  };

  const choose = (target: string | null) => {
    setOpen(false);
    if (target !== hostId) onChoose(target);
  };

  return (
    <>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Run on: ${name}`}
        title="Choose the computer the new workspace runs on"
        disabled={disabled}
        onClick={() => setOpen((shown) => !shown)}
        className={`inline-flex items-center gap-1.5 rounded border px-2 py-0.5 align-middle font-medium text-ink hover:bg-raised disabled:opacity-50 ${
          hostId === null ? "border-line" : "border-accent"
        }`}
      >
        <ComputerIcon />
        {name}
        <span aria-hidden className="text-small text-ink-faint">
          ▾
        </span>
      </button>
      {open && (
        <div
          ref={menu}
          role="menu"
          aria-label="Run on"
          onKeyDown={moveFocus}
          style={{ left: at.x, top: at.y }}
          className="fixed z-50 w-84 max-w-[calc(100vw-0.5rem)] rounded-md border border-line bg-raised py-1 text-left font-normal shadow-xl shadow-black/40"
        >
          <div className="px-3 pt-1.5 pb-1 text-micro font-semibold tracking-wider text-ink-faint uppercase">
            Run on
          </div>
          <Row
            name="This computer"
            checked={hostId === null}
            why={hostId === null ? null : (here ?? null)}
            onSelect={() => choose(null)}
          />
          {hosts.map((host) => {
            const why = whyNotThere(host, views[host.hostId]);
            return (
              <Row
                key={host.grantId}
                name={host.name}
                checked={host.hostId === hostId}
                why={why}
                online={why === null}
                onSelect={() => choose(host.hostId)}
              />
            );
          })}
          <div role="none" className="mt-1 border-t border-line pt-1">
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                useLayoutStore.getState().openSettings("remote");
              }}
              className="block w-full px-3 py-1.5 text-left text-ink-muted outline-none hover:bg-line hover:text-ink focus-visible:bg-line"
            >
              Pair another computer…
            </button>
          </div>
        </div>
      )}
    </>
  );
}

/** One computer in the menu: ticked when chosen, and with why it cannot be when it cannot. */
function Row({
  name,
  checked,
  why,
  online,
  onSelect,
}: {
  name: string;
  checked: boolean;
  why: string | null;
  online?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={checked}
      aria-label={why ? `${name}, ${why}` : online ? `${name}, online` : name}
      disabled={why !== null}
      onClick={onSelect}
      className="flex w-full items-center gap-2 px-3 py-1.5 text-left outline-none hover:bg-line focus-visible:bg-line disabled:hover:bg-transparent"
    >
      <span aria-hidden className="w-3 shrink-0 text-accent">
        {checked ? "✓" : ""}
      </span>
      <span className={`flex min-w-0 items-center gap-2 ${why ? "text-ink-faint" : "text-ink"}`}>
        <ComputerIcon />
        <span className="truncate">{name}</span>
      </span>
      <span className="ml-auto flex shrink-0 items-center gap-1.5 pl-3 text-small text-ink-faint">
        {online && <span aria-hidden className="size-2 rounded-full bg-green-400" />}
        {why ?? (online ? "online" : "")}
      </span>
    </button>
  );
}

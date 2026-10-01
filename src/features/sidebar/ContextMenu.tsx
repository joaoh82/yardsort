import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** One of a set to choose from: shown ticked when true, and read out as checked or not. */
  checked?: boolean;
  /** Draw a line above it, ending the group before. */
  divider?: boolean;
}

interface Props {
  /** Viewport coordinates to open at. */
  at: { x: number; y: number };
  items: MenuItem[];
  onClose: () => void;
}

export function ContextMenu({ at, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(at);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    setPosition({
      x: Math.max(4, Math.min(at.x, window.innerWidth - menu.offsetWidth - 4)),
      y: Math.max(4, Math.min(at.y, window.innerHeight - menu.offsetHeight - 4)),
    });
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [at]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  const moveFocus = (event: React.KeyboardEvent) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const buttons = [
      ...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
    ];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
    buttons[next]?.focus();
  };

  return (
    <div
      ref={ref}
      role="menu"
      onKeyDown={moveFocus}
      style={{ left: position.x, top: position.y }}
      className="fixed z-50 min-w-48 rounded-md border border-line bg-raised py-1 shadow-xl shadow-black/40"
    >
      {items.map((item) => (
        <div
          key={item.label}
          role="none"
          className={item.divider ? "mt-1 border-t border-line pt-1" : ""}
        >
          <button
            type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemradio"}
            aria-checked={item.checked}
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
            className={`block w-full px-3 py-1.5 text-left outline-none hover:bg-line focus-visible:bg-line disabled:opacity-40 disabled:hover:bg-transparent ${
              item.danger ? "text-red-400" : ""
            }`}
          >
            {item.checked !== undefined && (
              <span aria-hidden className="mr-2 inline-block w-3">
                {item.checked ? "✓" : ""}
              </span>
            )}
            {item.label}
          </button>
        </div>
      ))}
    </div>
  );
}

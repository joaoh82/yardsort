import { useEffect, useId, useRef, useState, type ReactNode, type MouseEventHandler } from "react";
import { createPortal } from "react-dom";

/** A hover preview that also opens on keyboard focus and lets you move into its contents. */
export function HoverCard({
  children,
  content,
  label,
  side = "bottom",
  className,
  onContextMenu,
}: {
  children: ReactNode;
  content: ReactNode;
  label: string;
  side?: "right" | "bottom";
  className?: string;
  onContextMenu?: MouseEventHandler<HTMLDivElement>;
}) {
  const id = useId();
  const anchor = useRef<HTMLDivElement>(null);
  const card = useRef<HTMLDivElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pointerDown = useRef(false);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const cancel = () => clearTimeout(timer.current);
  const close = () => {
    cancel();
    setPosition(null);
  };
  const show = () => {
    cancel();
    const box = anchor.current?.getBoundingClientRect();
    if (!box) return;
    setPosition({
      left: Math.max(
        8,
        Math.min(side === "right" ? box.right + 6 : box.right - 340, window.innerWidth - 348),
      ),
      top: Math.max(
        8,
        Math.min(side === "right" ? box.top : box.bottom + 6, window.innerHeight - 420),
      ),
    });
  };
  const leave = () => {
    cancel();
    timer.current = setTimeout(close, 180);
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!position) return;
    const dismiss = () => {
      clearTimeout(timer.current);
      setPosition(null);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (card.current?.contains(document.activeElement))
          anchor.current?.querySelector<HTMLElement>("button, a")?.focus();
        dismiss();
      }
    };
    const pointer = (event: PointerEvent) => {
      if (!card.current?.contains(event.target as Node)) dismiss();
    };
    const scroll = (event: Event) => {
      if (!card.current?.contains(event.target as Node)) dismiss();
    };
    const blur = (event: FocusEvent) => {
      // Focus moving from the document into a card control is still inside this window.
      if (!(event.relatedTarget instanceof Node)) dismiss();
    };
    window.addEventListener("keydown", key);
    window.addEventListener("pointerdown", pointer);
    window.addEventListener("resize", dismiss);
    window.addEventListener("scroll", scroll, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener("pointerdown", pointer);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("scroll", scroll, true);
      window.removeEventListener("blur", blur);
    };
  }, [position]);
  return (
    <div
      ref={anchor}
      data-hover-card="trigger"
      className={className}
      onPointerDownCapture={(event) => {
        if (!anchor.current?.contains(event.target as Node)) return;
        // Cancel even a pending hover before the browser focuses a clicked button.
        pointerDown.current = true;
        close();
      }}
      onPointerUpCapture={() => {
        pointerDown.current = false;
      }}
      onPointerCancel={() => {
        pointerDown.current = false;
      }}
      onContextMenu={(event) => {
        close();
        onContextMenu?.(event);
      }}
      onMouseEnter={(event) => {
        cancel();
        if ((event.target as Element).closest("[data-hover-card]") === anchor.current)
          timer.current = setTimeout(show, 350);
      }}
      onMouseLeave={leave}
      onMouseOver={(event) => {
        if (
          anchor.current?.contains(event.target as Node) &&
          (event.target as Element).closest("[data-hover-card]") !== anchor.current
        )
          close();
      }}
      onFocus={(event) => {
        if (
          anchor.current?.contains(event.target as Node) &&
          (event.target as Element).closest("[data-hover-card]") !== anchor.current
        )
          close();
        else if (
          !pointerDown.current &&
          (event.target as Element).matches(":focus-visible") &&
          !event.currentTarget.contains(event.relatedTarget)
        )
          show();
      }}
      onBlur={(event) => {
        pointerDown.current = false;
        if (
          !anchor.current?.contains(event.relatedTarget) &&
          !card.current?.contains(event.relatedTarget)
        )
          leave();
      }}
      onKeyDown={(event) => {
        if (
          event.key === "ArrowDown" &&
          position &&
          anchor.current?.contains(event.target as Node)
        ) {
          event.preventDefault();
          card.current?.querySelector<HTMLElement>("button, summary, a")?.focus();
        }
      }}
      aria-controls={position ? id : undefined}
    >
      {children}
      {position &&
        createPortal(
          <div
            ref={card}
            data-hover-card="content"
            id={id}
            role="region"
            aria-label={label}
            style={{ ...position, maxHeight: `calc(100vh - ${position.top + 8}px)` }}
            onMouseEnter={cancel}
            onMouseLeave={leave}
            onFocus={cancel}
            className="fixed z-40 max-h-[calc(100vh-16px)] w-[340px] max-w-[calc(100vw-16px)] overflow-y-auto rounded-lg border border-line bg-raised p-4 text-xs text-ink shadow-xl shadow-black/40 select-text"
          >
            {content}
          </div>,
          document.body,
        )}
    </div>
  );
}

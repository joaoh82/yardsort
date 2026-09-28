import type { ReactNode } from "react";

export function PanelHeader({ title, children }: { title: ReactNode; children?: ReactNode }) {
  return (
    <header className="flex h-9 shrink-0 items-center justify-between border-b border-line px-3">
      {typeof title === "string" ? (
        <h2 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          {title}
        </h2>
      ) : (
        title
      )}
      {children}
    </header>
  );
}

import type { ReactNode } from "react";

export function PanelHeader({
  title,
  search,
  children,
}: {
  title: string;
  search?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <header className="flex h-9 shrink-0 items-center justify-between border-b border-line px-3">
      {search ?? (
        <h2 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          {title}
        </h2>
      )}
      {children}
    </header>
  );
}

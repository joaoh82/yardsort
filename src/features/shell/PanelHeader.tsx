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
    <header className="flex h-10 shrink-0 items-center justify-between border-b border-line px-3.5">
      {search ?? (
        <h2 className="text-small font-semibold tracking-wider text-ink-muted uppercase">
          {title}
        </h2>
      )}
      {children}
    </header>
  );
}

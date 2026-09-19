"use client";

import { useRef, useState } from "react";

type Props = { command: string; size?: "hero" | "card" };

export function CopyCommand({ command, size = "card" }: Props) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  function copy() {
    void navigator.clipboard?.writeText(command);
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1500);
  }

  const hero = size === "hero";
  return (
    <div
      className={`flex min-w-0 max-w-full items-stretch overflow-hidden rounded-md border border-line font-mono ${
        hero ? "bg-surface text-[13px] max-md:w-full" : "bg-bg text-[12.5px]"
      }`}
    >
      <code
        className={`min-w-0 flex-1 text-left max-md:break-all md:whitespace-nowrap ${
          hero ? "px-3.5 py-[11px]" : "overflow-x-auto px-3 py-[9px]"
        }`}
      >
        <span className="text-faint select-none">$ </span>
        {command}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label="Copy install command"
        className="flex cursor-pointer items-center border-l border-line px-3 text-xs text-muted hover:text-ink"
      >
        <span aria-live="polite">{copied ? "Copied" : "Copy"}</span>
      </button>
    </div>
  );
}

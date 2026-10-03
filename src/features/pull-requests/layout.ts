import { useEffect, useState, type RefObject } from "react";

// What the Pull requests view and the Tasks view lay themselves out with: a list and a detail.

export const separator =
  "bg-line outline-none transition-colors hover:bg-accent focus-visible:bg-accent data-[separator=active]:bg-accent";

/** Whether `ref`'s element is narrower than `px`. False until it has been measured. */
export function useNarrowerThan(ref: RefObject<HTMLElement | null>, px: number): boolean {
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setNarrow(entry.contentRect.width < px);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, px]);
  return narrow;
}

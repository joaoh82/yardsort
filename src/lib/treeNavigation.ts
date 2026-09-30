import type { KeyboardEvent } from "react";

/** Navigate visible tree rows while leaving Tab available for each row's action buttons. */
export function treeNavigation(event: KeyboardEvent<HTMLElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if (!(event.target instanceof HTMLElement)) return;
  const tree = event.currentTarget;
  const row = event.target.closest<HTMLElement>('[role="treeitem"]');
  if (!row) return;
  const button = (item: Element) => item.querySelector<HTMLButtonElement>("button:not(:disabled)");
  const rows = Array.from(tree.querySelectorAll<HTMLElement>('[role="treeitem"]')).filter((item) =>
    button(item),
  );
  const index = rows.indexOf(row);
  let target: HTMLElement | undefined;
  switch (event.key) {
    case "ArrowDown":
      target = rows[Math.min(index + 1, rows.length - 1)];
      break;
    case "ArrowUp":
      target = rows[Math.max(index - 1, 0)];
      break;
    case "Home":
      target = rows[0];
      break;
    case "End":
      target = rows[rows.length - 1];
      break;
    case "ArrowRight":
      if (row.getAttribute("aria-expanded") === "false") button(row)?.click();
      else target = row.querySelector<HTMLElement>('[role="treeitem"]') ?? undefined;
      break;
    case "ArrowLeft":
      if (row.getAttribute("aria-expanded") === "true") button(row)?.click();
      else target = row.parentElement?.closest<HTMLElement>('[role="treeitem"]') ?? undefined;
      break;
    default:
      return;
  }
  event.preventDefault();
  if (target) button(target)?.focus();
}

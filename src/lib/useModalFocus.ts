import { useEffect, useRef, type RefObject } from "react";

/**
 * Give a dialog the keyboard while it is open, and hand it back afterwards.
 *
 * Without this a dialog opened by a shortcut leaves focus where it was — in the terminal, which
 * swallows every key: Escape and Tab would go to the agent behind the dialog instead of to it.
 * The dialog element needs `tabIndex={-1}` so it can hold focus itself.
 */
export function useModalFocus(dialog: RefObject<HTMLElement | null>) {
  const previous = useRef(
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  useEffect(() => {
    const element = dialog.current;
    const restore = previous.current;
    // Respect a field that focused itself (autoFocus, or an effect that ran before this one).
    if (element && !element.contains(document.activeElement)) element.focus();
    const isTop = () => {
      const dialogs = document.querySelectorAll('[role="dialog"], [role="alertdialog"]');
      return dialogs[dialogs.length - 1] === element;
    };
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !element || !isTop()) return;
      const controls = Array.from(
        element.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]',
        ),
      ).filter((control) => !control.closest('[hidden], [aria-hidden="true"]'));
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (!first || !last) {
        event.preventDefault();
        element.focus();
        return;
      }
      if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === element)
      ) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === element)
      ) {
        event.preventDefault();
        first.focus();
      }
    };
    const keepFocus = (event: FocusEvent) => {
      if (element && isTop() && event.target instanceof Node && !element.contains(event.target))
        element.focus();
    };
    document.addEventListener("keydown", trap);
    document.addEventListener("focusin", keepFocus);
    return () => {
      document.removeEventListener("keydown", trap);
      document.removeEventListener("focusin", keepFocus);
      if (restore?.isConnected) restore.focus();
    };
  }, [dialog]);
}

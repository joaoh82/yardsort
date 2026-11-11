import type { ReactNode } from "react";

export const inputClass =
  "h-9 w-full rounded border border-line bg-canvas px-2 text-ink outline-none select-text focus:border-accent disabled:opacity-50";

export const buttonClass =
  "h-9 rounded border border-line px-3 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40 disabled:hover:border-line disabled:hover:text-ink-muted";

export const primaryButtonClass =
  "h-9 rounded bg-accent px-4 font-medium text-canvas disabled:opacity-40";

/**
 * A labelled form row. Only the label text and the control live inside `<label>`, so the
 * control's accessible name is exactly the label: hints, errors and side buttons sit outside.
 */
export function Field(props: {
  label: string;
  hint?: ReactNode;
  error?: string | null;
  /** Rendered beside the control, e.g. a "Browse…" button. */
  trailing?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="grid content-start gap-1">
      <div className="flex items-end gap-2">
        <label className="grid min-w-0 flex-1 gap-1">
          <span className="text-ink-muted">{props.label}</span>
          {props.children}
        </label>
        {props.trailing}
      </div>
      {props.error ? (
        <span role="alert" className="text-small text-red-400">
          {props.error}
        </span>
      ) : (
        props.hint && <span className="text-small text-ink-faint">{props.hint}</span>
      )}
    </div>
  );
}

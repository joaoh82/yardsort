import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { yaml } from "@codemirror/lang-yaml";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { lintGutter, setDiagnostics } from "@codemirror/lint";
import { EditorState } from "@codemirror/state";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";
import type { Problem } from "@/lib/ipc";
import { diagnostics } from "./words";

interface Props {
  /** The text it starts with. Give the editor a new `key` to start over with other text. */
  text: string;
  /** Called with the whole text after every change. */
  onChange?: (text: string) => void;
  /** What the core found wrong, shown where each one is. */
  problems: Problem[];
  readOnly?: boolean;
}

/** Colours come from the app's CSS variables, so the editor follows light and dark by itself. */
const theme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "12px",
    backgroundColor: "var(--color-canvas)",
    color: "var(--color-ink)",
  },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.5" },
  ".cm-gutters": {
    backgroundColor: "var(--color-canvas)",
    color: "var(--color-ink-faint)",
    border: "none",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-tooltip": {
    backgroundColor: "var(--color-surface)",
    color: "var(--color-ink)",
    border: "1px solid var(--color-line)",
  },
});

/**
 * A workflow file in an editor. It owns a CodeMirror instance and nothing else: checking,
 * saving and everything around it live in the view, so they can be tested without it.
 */
export function WorkflowEditor({ text, onChange, problems, readOnly = false }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const dark = !window.matchMedia("(prefers-color-scheme: light)").matches;
    const view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: text,
        extensions: [
          lineNumbers(),
          lintGutter(),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          yaml(),
          syntaxHighlighting(dark ? oneDarkHighlightStyle : defaultHighlightStyle),
          EditorView.editable.of(!readOnly),
          EditorState.readOnly.of(readOnly),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChangeRef.current?.(update.state.doc.toString());
          }),
          theme,
        ],
      }),
    });
    viewRef.current = view;
    return () => {
      viewRef.current = null;
      view.destroy();
    };
    // The editor is made once per `key`; its text is its own from then on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch(setDiagnostics(view.state, diagnostics(view.state.doc, problems)));
  }, [problems]);

  return (
    <div
      ref={hostRef}
      aria-label="Workflow file"
      className="h-full min-h-0 overflow-hidden select-text"
    />
  );
}

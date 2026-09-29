import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { yaml } from "@codemirror/lang-yaml";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { lintGutter, setDiagnostics } from "@codemirror/lint";
import { Annotation, EditorState, StateEffect, StateField } from "@codemirror/state";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { Decoration, EditorView, keymap, lineNumbers, type DecorationSet } from "@codemirror/view";
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
  selection?: { id: string | null } | null;
  /** Undefined while the current text is being checked; never navigate with stale positions. */
  stepLines?: Record<string, number>;
}

/** Marks a change the view made to follow its `text`, as opposed to one the person typed. */
const fromOutside = Annotation.define<boolean>();

const markStep = StateEffect.define<number | null>();
const stepHighlight = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    value = value.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (effect.is(markStep)) {
        value =
          effect.value === null
            ? Decoration.none
            : Decoration.set([Decoration.line({ class: "cm-selectedStep" }).range(effect.value)]);
      }
    }
    return value;
  },
  provide: (field) => EditorView.decorations.from(field),
});

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
  // CodeMirror's caret and selection are black on white by default, whatever the page is.
  ".cm-content": { caretColor: "var(--color-ink)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--color-ink)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection":
    { backgroundColor: "var(--color-raised)" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "transparent" },
  ".cm-selectedStep": {
    backgroundColor: "var(--color-raised)",
    boxShadow: "inset 3px 0 var(--color-accent)",
  },
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
export function WorkflowEditor({
  text,
  onChange,
  problems,
  readOnly = false,
  selection,
  stepLines,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const handledSelection = useRef<Props["selection"]>(undefined);
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
          stepHighlight,
          lintGutter(),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          yaml(),
          syntaxHighlighting(dark ? oneDarkHighlightStyle : defaultHighlightStyle),
          EditorView.editable.of(!readOnly),
          EditorState.readOnly.of(readOnly),
          EditorView.updateListener.of((update) => {
            const typed = update.transactions.some((tr) => !tr.annotation(fromOutside));
            if (update.docChanged && typed) {
              onChangeRef.current?.(update.state.doc.toString());
            }
          }),
          theme,
        ],
      }),
    });
    viewRef.current = view;
    handledSelection.current = undefined;
    return () => {
      viewRef.current = null;
      view.destroy();
    };
    // The editor is made once per `key`; its text is its own from then on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly]);

  // The text is the editor's own while the person types: `onChange` hands it up and it comes
  // straight back. When it comes back different — Revert, or a save that changed the file — the
  // document is replaced, without telling `onChange`, which would only hand it up again.
  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === text) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      annotations: fromOutside.of(true),
    });
  }, [text]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch(setDiagnostics(view.state, diagnostics(view.state.doc, problems)));
  }, [problems]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || stepLines === undefined) return;
    const lineNumber = selection?.id ? stepLines[selection.id] : undefined;
    if (!lineNumber || lineNumber > view.state.doc.lines) {
      view.dispatch({ effects: markStep.of(null) });
      handledSelection.current = selection;
      return;
    }
    const line = view.state.doc.line(lineNumber);
    const navigate = handledSelection.current !== selection;
    handledSelection.current = selection;
    view.dispatch({
      ...(navigate ? { selection: { anchor: line.from } } : {}),
      effects: [
        markStep.of(line.from),
        ...(navigate ? [EditorView.scrollIntoView(line.from, { y: "center" })] : []),
      ],
    });
  }, [selection, stepLines, readOnly]);

  return (
    <div
      ref={hostRef}
      aria-label="Workflow file"
      className="h-full min-h-0 overflow-hidden select-text"
    />
  );
}

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  defaultHighlightStyle,
  LanguageDescription,
  syntaxHighlighting,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { Annotation, Compartment, EditorState, type Extension } from "@codemirror/state";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

const fromOutside = Annotation.define<boolean>();

interface Props {
  /** File name, used to pick syntax highlighting. */
  path: string;
  text: string;
  onChange?: (text: string) => void;
  /** When given, show `text` as a diff against this. */
  original?: string;
  /** Two panes side by side instead of one with the removals inline. Ignored without `original`. */
  split?: boolean;
}

/** Colours come from the app's CSS variables, so the viewer follows light and dark by itself. */
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
  ".cm-content": { caretColor: "var(--color-ink)" },
  "&.cm-focused": { outline: "none" },
  ".cm-changedLine": { backgroundColor: "rgba(80, 200, 120, 0.14) !important" },
  ".cm-deletedChunk": { backgroundColor: "rgba(240, 90, 90, 0.14)", paddingLeft: "6px" },
  ".cm-changedText": { background: "rgba(80, 200, 120, 0.3) !important" },
  ".cm-deletedChunk .cm-deletedText": { background: "rgba(240, 90, 90, 0.3) !important" },
  ".cm-collapsedLines": {
    color: "var(--color-ink-faint)",
    background: "var(--color-surface)",
    padding: "2px 8px",
  },
});

/**
 * A code editor when onChange is supplied, otherwise a read-only file or diff — inline, or as two panes.
 * Deliberately thin — it owns a CodeMirror instance and nothing else — so everything around it
 * can be tested without it.
 */
export function CodeView({ path, text, original, split, onChange }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);

  const editorRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const textRef = useRef(text);
  useEffect(() => {
    onChangeRef.current = onChange;
    textRef.current = text;
  });
  const editable = onChange !== undefined && original === undefined;

  const readOnlyText = editable ? null : text;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const dark = !window.matchMedia("(prefers-color-scheme: light)").matches;
    const language = new Compartment();

    const base: Extension[] = [
      lineNumbers(),
      EditorView.editable.of(editable),
      EditorState.readOnly.of(!editable),
      ...(editable
        ? [
            history(),
            keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
            EditorView.updateListener.of((update) => {
              if (update.docChanged && !update.transactions.some((t) => t.annotation(fromOutside)))
                onChangeRef.current?.(update.state.sliceDoc());
            }),
            EditorState.lineSeparator.of(textRef.current.includes("\r\n") ? "\r\n" : "\n"),
          ]
        : []),
      EditorView.lineWrapping,
      syntaxHighlighting(dark ? oneDarkHighlightStyle : defaultHighlightStyle),
      language.of([]),
      theme,
    ];

    // Two panes are a `MergeView`, which builds and owns its own pair of editors; one pane is an
    // ordinary `EditorView`, with the removals folded into it when there is something to compare.
    let views: EditorView[];
    let destroy: () => void;
    if (original !== undefined && split) {
      const merge = new MergeView({
        a: { doc: original, extensions: base },
        b: { doc: textRef.current, extensions: base },
        parent: host,
        highlightChanges: true,
        gutter: true,
        collapseUnchanged: { margin: 3, minSize: 8 },
      });
      views = [merge.a, merge.b];
      destroy = () => merge.destroy();
    } else {
      const extensions = [...base];
      if (original !== undefined) {
        extensions.push(
          unifiedMergeView({
            original,
            mergeControls: false,
            highlightChanges: true,
            gutter: true,
            syntaxHighlightDeletions: true,
            collapseUnchanged: { margin: 3, minSize: 8 },
          }),
        );
      }
      const view = new EditorView({
        parent: host,
        state: EditorState.create({ doc: textRef.current, extensions }),
      });
      editorRef.current = view;
      views = [view];
      destroy = () => view.destroy();
    }

    // Grammars are loaded on demand; highlighting arrives a moment after the text. One
    // compartment serves both panes — the effect is dispatched to each.
    let disposed = false;
    const match = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
    void match?.load().then((support) => {
      if (disposed) return;
      for (const view of views) view.dispatch({ effects: language.reconfigure(support) });
    }, console.error);

    return () => {
      disposed = true;
      editorRef.current = null;
      destroy();
    };
  }, [path, original, split, editable, readOnlyText]);

  useEffect(() => {
    const view = editorRef.current;
    if (view && view.state.sliceDoc() !== text) {
      view.dispatch({
        annotations: fromOutside.of(true),
        changes: { from: 0, to: view.state.doc.length, insert: text },
      });
    }
  }, [text]);

  return <div ref={hostRef} className="h-full min-h-0 overflow-hidden select-text" />;
}

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  defaultHighlightStyle,
  LanguageDescription,
  syntaxHighlighting,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { getChunks, getOriginalDoc, MergeView, unifiedMergeView } from "@codemirror/merge";
import {
  Annotation,
  Compartment,
  EditorState,
  Transaction,
  type Extension,
  type Text,
} from "@codemirror/state";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { Decoration, EditorView, keymap, lineNumbers, WidgetType } from "@codemirror/view";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

const fromOutside = Annotation.define<boolean>();

/** Which side of a diff a line belongs to: the text as it was, or as it is. */
export type DiffSideName = "old" | "new";

/** Something shown as a block under a line: a comment thread beside the lines it is about. */
export interface LineNote {
  /** Stable across renders: the block keeps its place and its contents when the rest changes. */
  key: string;
  /** 1-based, in the text of `side`. Clamped to the text when it is past the end. */
  line: number;
  side: DiffSideName;
}

/** The lines selected in one pane, for whatever wants to act on them. */
export interface LineSelection {
  side: DiffSideName;
  /** 1-based, inclusive. */
  from: number;
  to: number;
  /** Those lines, whole. */
  text: string;
}

interface Props {
  /** File name, used to pick syntax highlighting. */
  path: string;
  text: string;
  onChange?: (text: string) => void;
  /** When given, show `text` as a diff against this. */
  original?: string;
  /** Two panes side by side instead of one with the removals inline. Ignored without `original`. */
  split?: boolean;
  /** Blocks to show under lines, drawn by `renderNote`. */
  notes?: LineNote[];
  renderNote?: (key: string) => ReactNode;
  /** Told whenever the selected lines change, and with `null` when nothing is selected. */
  onSelect?: (selection: LineSelection | null) => void;
}

/** A block under a line whose contents React draws, into a node this hands CodeMirror. */
class NoteBlock extends WidgetType {
  constructor(
    readonly key: string,
    readonly node: HTMLElement,
  ) {
    super();
  }
  eq(other: NoteBlock) {
    return other.key === this.key;
  }
  toDOM() {
    return this.node;
  }
  ignoreEvent() {
    return true;
  }
}

/** Where a line of the old text sits in the new one, for a pane that shows only the new. */
function mapOldLine(state: EditorState, oldLine: number): number {
  let original: Text;
  try {
    original = getOriginalDoc(state);
  } catch {
    return oldLine;
  }
  const line = original.line(Math.max(1, Math.min(oldLine, original.lines)));
  let shift = 0;
  for (const chunk of getChunks(state)?.chunks ?? []) {
    if (line.from < chunk.fromA) break;
    if (line.from < chunk.toA) {
      // Inside a changed stretch: the top of it, as the new text has it.
      return state.doc.lineAt(Math.min(chunk.fromB, state.doc.length)).number;
    }
    shift = chunk.toB - chunk.toA;
  }
  return state.doc.lineAt(Math.min(line.from + shift, state.doc.length)).number;
}

/** The decorations that put each note's block under its line, in this pane. */
function noteBlocks(
  state: EditorState,
  notes: LineNote[],
  nodes: Map<string, HTMLElement>,
  pane: DiffSideName,
  unified: boolean,
) {
  const placed = notes
    .filter((note) => note.side === pane || (unified && pane === "new"))
    .map((note) => {
      const line =
        note.side === pane ? note.line : unified ? mapOldLine(state, note.line) : note.line;
      const at = state.doc.line(Math.max(1, Math.min(line, state.doc.lines)));
      let node = nodes.get(note.key);
      if (!node) {
        node = document.createElement("div");
        node.className = "cm-note";
        nodes.set(note.key, node);
      }
      return Decoration.widget({
        widget: new NoteBlock(note.key, node),
        block: true,
        side: 1,
      }).range(at.to);
    })
    .sort((a, b) => a.from - b.from);
  return Decoration.set(placed, true);
}

/** The lines the selection covers, whole, or nothing when it is only a caret. */
function selectedLines(state: EditorState, side: DiffSideName): LineSelection | null {
  const { from, to } = state.selection.main;
  if (from === to) return null;
  const first = state.doc.lineAt(from);
  // A selection ending at the very start of a line does not include that line.
  const last = state.doc.lineAt(to > first.to && state.doc.lineAt(to).from === to ? to - 1 : to);
  return {
    side,
    from: first.number,
    to: last.number,
    text: state.doc.sliceString(first.from, last.to),
  };
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
export function CodeView({
  path,
  text,
  original,
  split,
  onChange,
  notes,
  renderNote,
  onSelect,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);

  const historyRef = useRef<Compartment | null>(null);
  const editorRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const onSelectRef = useRef(onSelect);
  const textRef = useRef(text);
  useEffect(() => {
    onChangeRef.current = onChange;
    onSelectRef.current = onSelect;
    textRef.current = text;
  });
  // The panes and their note compartments, so notes can change without the editors.
  const panesRef = useRef<{ view: EditorView; side: DiffSideName; notes: Compartment }[]>([]);
  // One node per note, kept for as long as the note is: React draws into it, CodeMirror places it.
  const nodesRef = useRef(new Map<string, HTMLElement>());
  // The same nodes, as state, so the render below can hand each to a portal.
  const [slots, setSlots] = useState<{ key: string; node: HTMLElement }[]>([]);
  const [mounted, setMounted] = useState(0);
  const editable = onChange !== undefined && original === undefined;

  const readOnlyText = editable ? null : text;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const dark = !window.matchMedia("(prefers-color-scheme: light)").matches;
    const language = new Compartment();
    const undoHistory = new Compartment();
    historyRef.current = editable ? undoHistory : null;

    const base: Extension[] = [
      lineNumbers(),
      EditorView.editable.of(editable),
      EditorState.readOnly.of(!editable),
      ...(editable
        ? [
            undoHistory.of(history()),
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

    // Each pane reports its selected lines, and has a compartment for the notes under lines.
    const paneExtensions = (side: DiffSideName, noteSlot: Compartment): Extension[] => [
      ...base,
      noteSlot.of([]),
      EditorView.updateListener.of((update) => {
        if (update.selectionSet || update.docChanged)
          onSelectRef.current?.(selectedLines(update.state, side));
      }),
    ];
    const oldNotes = new Compartment();
    const newNotes = new Compartment();

    // Two panes are a `MergeView`, which builds and owns its own pair of editors; one pane is an
    // ordinary `EditorView`, with the removals folded into it when there is something to compare.
    let views: EditorView[];
    let destroy: () => void;
    if (original !== undefined && split) {
      const merge = new MergeView({
        a: { doc: original, extensions: paneExtensions("old", oldNotes) },
        b: { doc: textRef.current, extensions: paneExtensions("new", newNotes) },
        parent: host,
        highlightChanges: true,
        gutter: true,
        collapseUnchanged: { margin: 3, minSize: 8 },
      });
      views = [merge.a, merge.b];
      panesRef.current = [
        { view: merge.a, side: "old", notes: oldNotes },
        { view: merge.b, side: "new", notes: newNotes },
      ];
      destroy = () => merge.destroy();
    } else {
      const extensions = paneExtensions("new", newNotes);
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
      panesRef.current = [{ view, side: "new", notes: newNotes }];
      destroy = () => view.destroy();
    }
    setMounted((n) => n + 1);

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
      historyRef.current = null;
      editorRef.current = null;
      panesRef.current = [];
      onSelectRef.current?.(null);
      destroy();
    };
  }, [path, original, split, editable, readOnlyText]);

  // Put the notes under their lines, in whichever pane shows that side. In one pane, a note
  // on the old text goes under the line the new text has where the old one was.
  const noteKeys = (notes ?? []).map((note) => `${note.side}:${note.line}:${note.key}`).join("\n");
  useEffect(() => {
    const unified = panesRef.current.length === 1 && original !== undefined;
    for (const pane of panesRef.current) {
      const blocks = noteBlocks(pane.view.state, notes ?? [], nodesRef.current, pane.side, unified);
      pane.view.dispatch({ effects: pane.notes.reconfigure(EditorView.decorations.of(blocks)) });
    }
    // Nodes for notes that are gone are let go of, after React has stopped drawing into them.
    const keep = new Set((notes ?? []).map((note) => note.key));
    for (const key of [...nodesRef.current.keys()]) {
      if (!keep.has(key)) nodesRef.current.delete(key);
    }
    setSlots([...nodesRef.current].map(([key, node]) => ({ key, node })));
    // `noteKeys` is what the list of notes amounts to; `notes` itself is a new array each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteKeys, mounted, original]);

  useEffect(() => {
    const view = editorRef.current;
    if (view && view.state.sliceDoc() !== text) {
      // A disk reload/discard starts a new baseline. Mapping old undo entries through a
      // whole-document replacement can otherwise resurrect text from the previous version.
      const undoHistory = historyRef.current;
      view.dispatch({
        effects: undoHistory ? undoHistory.reconfigure([]) : [],
        annotations: [fromOutside.of(true), Transaction.addToHistory.of(false)],
        changes: { from: 0, to: view.state.doc.length, insert: text },
      });
      if (undoHistory) view.dispatch({ effects: undoHistory.reconfigure(history()) });
    }
  }, [text]);

  return (
    <>
      <div ref={hostRef} className="h-full min-h-0 overflow-hidden select-text" />
      {renderNote && slots.map(({ key, node }) => createPortal(renderNote(key), node, key))}
    </>
  );
}

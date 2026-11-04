// Read-only diff editor on CodeMirror 6 (@codemirror/merge): side-by-side or
// unified, character-level highlights, unchanged regions folded, syntax
// highlighting loaded on demand from the file name.
import { useEffect, useRef, useState } from "react";
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { MergeView, getChunks, unifiedMergeView, type Chunk } from "@codemirror/merge";
import { Compartment, EditorState, type Extension, type Text } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

export type DiffLayout = "split" | "unified";

export interface DiffStats {
  added: number;
  deleted: number;
  changes: number;
}

/** Colours come from CSS variables, so light and dark themes just work. */
const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.operatorKeyword, t.controlKeyword], color: "var(--syn-keyword)" },
  { tag: [t.string, t.special(t.string), t.regexp], color: "var(--syn-string)" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--syn-number)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--syn-function)" },
  { tag: [t.typeName, t.className, t.namespace], color: "var(--syn-type)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--syn-property)" },
  { tag: [t.tagName, t.macroName], color: "var(--syn-tag)" },
  { tag: [t.meta, t.annotation], color: "var(--syn-meta)" },
  { tag: t.heading, color: "var(--syn-keyword)", fontWeight: "bold" },
  { tag: t.link, color: "var(--syn-string)", textDecoration: "underline" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "12px", backgroundColor: "var(--bg)", color: "var(--text)" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "20px" },
  ".cm-content": { padding: "0" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--muted)", border: "none" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 8px 0 10px", minWidth: "32px" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--selection) !important" },
});

/** Shared by every editor in the diff. */
const base: Extension = [
  EditorState.readOnly.of(true),
  EditorView.editable.of(false),
  lineNumbers(),
  syntaxHighlighting(highlight),
  theme,
];

const COLLAPSE = { margin: 3, minSize: 6 };
/** Give up on a minimal diff for huge files rather than freezing the UI. */
const DIFF_CONFIG = { scanLimit: 20_000, timeout: 1500 };

function lineSpan(doc: Text, from: number, to: number): number {
  if (to <= from) return 0;
  return doc.lineAt(Math.max(from, to - 1)).number - doc.lineAt(from).number + 1;
}

function statsOf(chunks: readonly Chunk[], a: Text, b: Text): DiffStats {
  let added = 0;
  let deleted = 0;
  for (const c of chunks) {
    deleted += lineSpan(a, c.fromA, c.toA);
    added += lineSpan(b, c.fromB, c.toB);
  }
  return { added, deleted, changes: chunks.length };
}

interface Props {
  path: string;
  oldText: string;
  newText: string;
  layout: DiffLayout;
  onStats?: (s: DiffStats) => void;
}

/** Imperative handle for change navigation. */
export interface CodeDiffHandle {
  go: (dir: 1 | -1) => void;
}

export function CodeDiff({ path, oldText, newText, layout, onStats, handle }: Props & { handle?: (h: CodeDiffHandle | null) => void }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const stats = useRef(onStats);
  stats.current = onStats;

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const lang = new Compartment();
    const ext = [base, lang.of([])];
    let views: EditorView[] = [];
    let destroy = () => {};
    // Where changes are, in the editor that navigation scrolls.
    let positions: () => number[] = () => [];
    try {
      if (layout === "split") {
        const mv = new MergeView({
          a: { doc: oldText, extensions: ext },
          b: { doc: newText, extensions: ext },
          parent,
          highlightChanges: true,
          gutter: true,
          collapseUnchanged: COLLAPSE,
          diffConfig: DIFF_CONFIG,
        });
        views = [mv.a, mv.b];
        destroy = () => mv.destroy();
        positions = () => mv.chunks.map((c) => c.fromB);
        stats.current?.(statsOf(mv.chunks, mv.a.state.doc, mv.b.state.doc));
      } else {
        const view = new EditorView({
          parent,
          state: EditorState.create({
            doc: newText,
            extensions: [
              ext,
              unifiedMergeView({
                original: oldText,
                mergeControls: false,
                highlightChanges: true,
                gutter: true,
                syntaxHighlightDeletions: true,
                collapseUnchanged: COLLAPSE,
                diffConfig: DIFF_CONFIG,
              }),
            ],
          }),
        });
        views = [view];
        destroy = () => view.destroy();
        const info = () => getChunks(view.state);
        positions = () => info()?.chunks.map((c) => c.fromB) ?? [];
        const c = info();
        if (c) stats.current?.(statsOf(c.chunks, EditorState.create({ doc: oldText }).doc, view.state.doc));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    }

    const target = views[views.length - 1];
    handle?.({
      go(dir) {
        if (!target) return;
        // The line at the middle of what is visible. In the side-by-side view
        // the merge view scrolls, not the editors themselves.
        const box = parent.firstElementChild ?? parent;
        const rect = box.getBoundingClientRect();
        const x = target.contentDOM.getBoundingClientRect().left + 4;
        const at = target.posAtCoords({ x, y: rect.top + rect.height / 2 }, false);
        const line = target.state.doc.lineAt(at);
        const pos = positions();
        const next = dir > 0 ? pos.find((p) => p > line.to) : [...pos].reverse().find((p) => p < line.from);
        if (next != null) target.dispatch({ effects: EditorView.scrollIntoView(next, { y: "center" }) });
      },
    });

    let alive = true;
    const desc = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
    desc
      ?.load()
      .then((support) => {
        if (alive) for (const v of views) v.dispatch({ effects: lang.reconfigure(support) });
      })
      .catch(() => {});

    return () => {
      alive = false;
      handle?.(null);
      destroy();
      parent.textContent = "";
    };
    // `handle` is a stable setter from the parent.
  }, [path, oldText, newText, layout]);

  return (
    <div className={`code-diff code-diff-${layout}`} ref={host}>
      {error && <div className="diff-empty">无法显示差异：{error}</div>}
    </div>
  );
}

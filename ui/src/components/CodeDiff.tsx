// Read-only diff editor on CodeMirror 6 (@codemirror/merge): side-by-side or
// unified, character-level highlights, unchanged regions folded, syntax
// highlighting loaded on demand from the file name.
import { useEffect, useRef, useState } from "react";
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { MergeView, getChunks, unifiedMergeView, type Chunk } from "@codemirror/merge";
import { Compartment, EditorState, RangeSet, StateEffect, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import { EditorView, GutterMarker, gutter, lineNumbers } from "@codemirror/view";
import {
  buildIndex,
  chunkCheck,
  revertChunk,
  splitLines,
  stageState,
  withChunk,
  withLine,
  type Check,
  type ChunkStage,
  toLineChunk,
  type LineChunk,
} from "../lib/staging";
import { ContextMenu } from "./ContextMenu";
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

/** Block/line staging for a HEAD ↔ worktree diff. */
export interface Staging {
  /** Current staged content (null: not in the index). */
  index: string | null;
  /** Store this as the file's staged content. */
  onStage: (content: string) => void;
  /** Discard a block: replace the working-tree file with `content` and, if
   * the block was (partly) staged, set the staged content to `index`. */
  onDiscard: (content: string, index: string | null) => void;
}

// ------------------------------------------------------------ stage gutters

class BoxMarker extends GutterMarker {
  constructor(readonly state: Check) {
    super();
  }
  eq(o: BoxMarker) {
    return o.state === this.state;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = `stage-box ${this.state === true ? "on" : this.state === "partial" ? "partial" : ""}`;
    el.title = this.state === true ? "已暂存此块（点击取消）" : "暂存此块（右键更多）";
    return el;
  }
}

class DotMarker extends GutterMarker {
  constructor(readonly on: boolean) {
    super();
  }
  eq(o: DotMarker) {
    return o.on === this.on;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = `stage-dot ${this.on ? "on" : ""}`;
    el.title = this.on ? "已暂存此行" : "暂存此行";
    return el;
  }
}

const setMarks = StateEffect.define<{ boxes: RangeSet<GutterMarker>; dots: RangeSet<GutterMarker> }>();
const marks = StateField.define<{ boxes: RangeSet<GutterMarker>; dots: RangeSet<GutterMarker> }>({
  create: () => ({ boxes: RangeSet.empty, dots: RangeSet.empty }),
  update(v, tr) {
    for (const e of tr.effects) if (e.is(setMarks)) return e.value;
    return v;
  },
});

type GutterHit = (view: EditorView, line: number, kind: "box" | "dot", e: MouseEvent) => boolean;

/** The two stage gutters: block checkboxes and per-line toggles. */
function stageGutters(hit: GutterHit): Extension {
  const handlers = (kind: "box" | "dot") => ({
    mousedown: (view: EditorView, block: { from: number }, e: Event) =>
      (e as MouseEvent).button === 0 && hit(view, view.state.doc.lineAt(block.from).number, kind, e as MouseEvent),
    contextmenu: (view: EditorView, block: { from: number }, e: Event) =>
      hit(view, view.state.doc.lineAt(block.from).number, kind, e as MouseEvent),
  });
  return [
    marks,
    gutter({ class: "cm-stage-boxes", markers: (v) => v.state.field(marks).boxes, domEventHandlers: handlers("box") }),
    gutter({ class: "cm-stage-dots", markers: (v) => v.state.field(marks).dots, domEventHandlers: handlers("dot") }),
  ];
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

export function CodeDiff({
  path,
  oldText,
  newText,
  layout,
  onStats,
  handle,
  staging,
}: Props & { handle?: (h: CodeDiffHandle | null) => void; staging?: Staging }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; k: number } | null>(null);
  const stats = useRef(onStats);
  stats.current = onStats;
  const stagingRef = useRef(staging);
  stagingRef.current = staging;
  const stageable = !!staging;
  // Set by the editor effect: re-reads the index, and acts on block k.
  const ctl = useRef<{ setIndex: (i: string | null) => void; chunk: (k: number, act: "stage" | "unstage" | "discard") => void; check: (k: number) => Check } | null>(null);
  // Scroll position kept across re-creation (e.g. after discarding a block).
  const scroll = useRef<{ key: string; top: number } | null>(null);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const lang = new Compartment();
    const ext = [base, lang.of([])];

    // ---- staging state, shared by the gutters below
    const H = splitLines(oldText);
    const W = splitLines(newText);
    let chunks: LineChunk[] = [];
    let stage: ChunkStage[] = [];
    // Per editor: line number → what clicking it toggles.
    const targets = new Map<EditorView, Map<number, { k: number; side?: "del" | "add"; i?: number }>>();
    let sideA: EditorView | null = null;
    let sideB: EditorView | null = null;
    const render = () => {
      for (const [view, side] of [
        [sideA, "a"],
        [sideB, "b"],
      ] as const) {
        if (!view) continue;
        const doc = view.state.doc;
        const boxes: Range<GutterMarker>[] = [];
        const dots: Range<GutterMarker>[] = [];
        const map = new Map<number, { k: number; side?: "del" | "add"; i?: number }>();
        const at = (n: number) => doc.line(Math.max(1, Math.min(doc.lines, n))).from;
        chunks.forEach((c, k) => {
          const st = stage[k];
          if (!st) return;
          if (side === "b") {
            // Block checkbox on the block's first new line; for a pure
            // deletion, in the old side (split) or next to where it was.
            if (c.b1 > c.b0 || !sideA) {
              const n = Math.min(c.b0 + 1, doc.lines);
              boxes.push(new BoxMarker(chunkCheck(st)).range(at(n)));
              if (!map.has(n)) map.set(n, { k });
            }
            for (let j = c.b0; j < c.b1; j++) {
              dots.push(new DotMarker(st.add[j - c.b0]).range(at(j + 1)));
              map.set(-(j + 1), { k, side: "add", i: j - c.b0 });
            }
          } else {
            if (c.b1 === c.b0 && c.a1 > c.a0) {
              boxes.push(new BoxMarker(chunkCheck(st)).range(at(c.a0 + 1)));
              map.set(c.a0 + 1, { k });
            }
            for (let i = c.a0; i < c.a1; i++) {
              dots.push(new DotMarker(st.del[i - c.a0]).range(at(i + 1)));
              map.set(-(i + 1), { k, side: "del", i: i - c.a0 });
            }
          }
        });
        targets.set(view, map);
        view.dispatch({ effects: setMarks.of({ boxes: RangeSet.of(boxes, true), dots: RangeSet.of(dots, true) }) });
      }
    };
    const apply = (next: ChunkStage[]) => {
      stage = next;
      render();
      stagingRef.current?.onStage(buildIndex(H, W, chunks, next));
    };
    const hit: GutterHit = (view, line, kind, e) => {
      // Boxes are keyed by line, dots by -line.
      const t = targets.get(view)?.get(kind === "box" ? line : -line);
      if (!t || !stage[t.k]) return false;
      e.preventDefault();
      if (e.type === "contextmenu") {
        setMenu({ x: e.clientX, y: e.clientY, k: t.k });
      } else if (kind === "box" || t.side == null) {
        apply(withChunk(stage, t.k, chunkCheck(stage[t.k]) !== true));
      } else {
        apply(withLine(stage, t.k, t.side, t.i!));
      }
      return true;
    };
    const stageExt = stageable ? stageGutters(hit) : [];
    let views: EditorView[] = [];
    let destroy = () => {};
    // Where changes are, in the editor that navigation scrolls.
    let positions: () => number[] = () => [];
    try {
      if (layout === "split") {
        const mv = new MergeView({
          a: { doc: oldText, extensions: [ext, stageExt] },
          b: { doc: newText, extensions: [ext, stageExt] },
          parent,
          highlightChanges: true,
          gutter: true,
          collapseUnchanged: COLLAPSE,
          diffConfig: DIFF_CONFIG,
        });
        views = [mv.a, mv.b];
        destroy = () => mv.destroy();
        positions = () => mv.chunks.map((c) => c.fromB);
        sideA = mv.a;
        sideB = mv.b;
        chunks = mv.chunks.map((c) => toLineChunk(c, mv.a.state.doc, mv.b.state.doc));
        stats.current?.(statsOf(mv.chunks, mv.a.state.doc, mv.b.state.doc));
      } else {
        const view = new EditorView({
          parent,
          state: EditorState.create({
            doc: newText,
            extensions: [
              ext,
              stageExt,
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
        const oldDoc = EditorState.create({ doc: oldText }).doc;
        if (c) stats.current?.(statsOf(c.chunks, oldDoc, view.state.doc));
        sideB = view;
        chunks = (c?.chunks ?? []).map((ch) => toLineChunk(ch, oldDoc, view.state.doc));
      }
      if (stageable) {
        const index = stagingRef.current?.index ?? null;
        stage = stageState(H, W, index == null ? null : splitLines(index), chunks);
        render();
        ctl.current = {
          setIndex(index) {
            stage = stageState(H, W, index == null ? null : splitLines(index), chunks);
            render();
          },
          check: (k) => chunkCheck(stage[k]),
          chunk(k, act) {
            if (act === "discard") {
              const staged = chunkCheck(stage[k]) !== false;
              stagingRef.current?.onDiscard(
                revertChunk(H, W, chunks, k),
                staged ? buildIndex(H, W, chunks, withChunk(stage, k, false)) : null,
              );
            } else apply(withChunk(stage, k, act === "stage"));
          },
        };
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    }

    const target = views[views.length - 1];
    // Restore the scroll position when the same diff is rebuilt.
    const scrollBox = () => (parent.querySelector(".cm-mergeView") ?? parent.querySelector(".cm-scroller")) as HTMLElement | null;
    const key = `${path}:${layout}`;
    if (scroll.current?.key === key) {
      const top = scroll.current.top;
      requestAnimationFrame(() => {
        const box = scrollBox();
        if (box) box.scrollTop = top;
      });
    }
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
      ctl.current = null;
      scroll.current = { key, top: scrollBox()?.scrollTop ?? 0 };
      destroy();
      parent.textContent = "";
    };
    // `handle` is a stable setter from the parent.
  }, [path, oldText, newText, layout, stageable]);

  // A new index (after staging, or from outside) only redraws the checkboxes.
  const index = staging?.index ?? null;
  useEffect(() => ctl.current?.setIndex(index), [index]);

  return (
    <div className={`code-diff code-diff-${layout}`} ref={host}>
      {error && <div className="diff-empty">无法显示差异：{error}</div>}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "暂存此块", disabled: ctl.current?.check(menu.k) === true, onClick: () => ctl.current?.chunk(menu.k, "stage") },
            { label: "取消暂存此块", disabled: ctl.current?.check(menu.k) === false, onClick: () => ctl.current?.chunk(menu.k, "unstage") },
            "separator",
            { label: "丢弃此块（还原为 HEAD）…", danger: true, onClick: () => ctl.current?.chunk(menu.k, "discard") },
          ]}
        />
      )}
    </div>
  );
}

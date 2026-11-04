// Block- and line-level staging from the HEAD ↔ worktree diff.
import type { Chunk } from "@codemirror/merge";
import type { Text } from "@codemirror/state";
//
// The diff editor finds the changed blocks ("chunks") between HEAD (H) and
// the working tree (W). Staging never builds patches: the UI computes the
// whole content the index should hold — H with the selected blocks or lines
// of W applied — and the backend stores it as the file's staged version.
// To draw the checkboxes, the current index content (I) is matched back onto
// the same blocks.

/** A changed block as 0-based line ranges: H[a0, a1) became W[b0, b1). */
export interface LineChunk {
  a0: number;
  a1: number;
  b0: number;
  b1: number;
}

/** What of a block is staged: per H line "its deletion is staged", per W
 * line "its addition is staged". */
export interface ChunkStage {
  del: boolean[];
  add: boolean[];
}

export type Check = true | false | "partial";

/** Lines with their terminators ("a\nb" → ["a\n", "b"]); "" → []. */
export function splitLines(text: string): string[] {
  const out: string[] = [];
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) === 10) {
      out.push(text.slice(start, i + 1));
      start = i + 1;
    }
  }
  if (start < text.length) out.push(text.slice(start));
  return out;
}

/** Concatenate lines, adding a line break where a line without one is
 * followed by more (e.g. text appended after a last line that had none). */
function join(lines: string[], eol: string): string {
  let s = "";
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    s += l;
    if (i < lines.length - 1 && !l.endsWith("\n")) s += eol;
  }
  return s;
}

const eolOf = (lines: string[]) => (lines.some((l) => l.endsWith("\r\n")) ? "\r\n" : "\n");

export function chunkCheck(s: ChunkStage): Check {
  const all = [...s.del, ...s.add];
  if (all.length === 0) return false;
  if (all.every(Boolean)) return true;
  return all.some(Boolean) ? "partial" : false;
}

const same = (x: string[], from: number, y: string[]) => {
  if (from + y.length > x.length) return false;
  for (let i = 0; i < y.length; i++) if (x[from + i] !== y[i]) return false;
  return true;
};

/** Longest common subsequence of `x` and `y`: which `y` items are matched. */
function lcsMatched(x: string[], y: string[]): { inX: boolean[]; inY: boolean[] } {
  const n = x.length;
  const m = y.length;
  const inX = new Array<boolean>(n).fill(false);
  const inY = new Array<boolean>(m).fill(false);
  if (n * m > 4_000_000) return { inX, inY }; // too big to bother; report nothing staged
  const t: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) t[i][j] = x[i] === y[j] ? t[i + 1][j + 1] + 1 : Math.max(t[i + 1][j], t[i][j + 1]);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      inX[i++] = true;
      inY[j++] = true;
    } else if (t[i + 1][j] >= t[i][j + 1]) i++;
    else j++;
  }
  return { inX, inY };
}

const nothing = (chunks: LineChunk[]): ChunkStage[] =>
  chunks.map((c) => ({ del: new Array(c.a1 - c.a0).fill(false), add: new Array(c.b1 - c.b0).fill(false) }));

/**
 * What is staged, by matching the index `I` onto the blocks. `I` normally
 * is H with some blocks (or lines) of W applied; anything else (staged
 * from outside, unrelated edits) is reported as best it can be.
 */
export function stageState(H: string[], W: string[], I: string[] | null, chunks: LineChunk[]): ChunkStage[] {
  if (!I) return nothing(chunks);
  const out: ChunkStage[] = [];
  let p = 0;
  let prev = 0;
  for (let k = 0; k < chunks.length; k++) {
    const c = chunks[k];
    const before = H.slice(prev, c.a0);
    if (!same(I, p, before)) return nothing(chunks);
    p += before.length;
    const h = H.slice(c.a0, c.a1);
    const w = W.slice(c.b0, c.b1);
    // The unchanged lines that follow delimit this block in I.
    const next = k + 1 < chunks.length ? H.slice(c.a1, chunks[k + 1].a0) : H.slice(c.a1);
    const last = k + 1 === chunks.length;
    const fits = (len: number) => (last ? p + len + next.length === I.length && same(I, p + len, next) : same(I, p + len, next));
    let end: number;
    if (same(I, p, w) && fits(w.length)) end = p + w.length;
    else if (same(I, p, h) && fits(h.length)) end = p + h.length;
    else {
      end = -1;
      for (let q = p; q <= I.length; q++)
        if (fits(q - p)) {
          end = q;
          break;
        }
      if (end < 0) return nothing(chunks);
    }
    const x = I.slice(p, end);
    // Kept H lines are unstaged deletions; the other lines of x are staged additions.
    const kept = lcsMatched(x, h);
    const rest = x.filter((_, i) => !kept.inX[i]);
    const added = lcsMatched(rest, w);
    out.push({ del: kept.inY.map((k) => !k), add: added.inY });
    p = end;
    prev = c.a1;
  }
  return out;
}

/** Index content for a selection: H with the staged parts of each block. */
export function buildIndex(H: string[], W: string[], chunks: LineChunk[], stage: ChunkStage[]): string {
  const out: string[] = [];
  let prev = 0;
  chunks.forEach((c, k) => {
    out.push(...H.slice(prev, c.a0));
    for (let i = c.a0; i < c.a1; i++) if (!stage[k].del[i - c.a0]) out.push(H[i]);
    for (let j = c.b0; j < c.b1; j++) if (stage[k].add[j - c.b0]) out.push(W[j]);
    prev = c.a1;
  });
  out.push(...H.slice(prev));
  return join(out, eolOf(W.length ? W : H));
}

/** Worktree content with block `k` put back to its HEAD version. */
export function revertChunk(H: string[], W: string[], chunks: LineChunk[], k: number): string {
  const c = chunks[k];
  return join([...W.slice(0, c.b0), ...H.slice(c.a0, c.a1), ...W.slice(c.b1)], eolOf(W.length ? W : H));
}

/** A stage with block `k` set entirely to `on`. */
export function withChunk(stage: ChunkStage[], k: number, on: boolean): ChunkStage[] {
  return stage.map((s, i) => (i === k ? { del: s.del.map(() => on), add: s.add.map(() => on) } : s));
}

/** A stage with one line of block `k` toggled. */
export function withLine(stage: ChunkStage[], k: number, side: "del" | "add", index: number): ChunkStage[] {
  return stage.map((s, i) => {
    if (i !== k) return s;
    const v = [...s[side]];
    v[index] = !v[index];
    return { ...s, [side]: v };
  });
}

/** A diff-editor chunk as line ranges (chunks cover whole lines). */
export function toLineChunk(c: Chunk, a: Text, b: Text): LineChunk {
  const range = (doc: Text, from: number, to: number) => {
    const first = doc.lineAt(from).number - 1;
    if (to <= from) return [first, first];
    // A chunk touching a last line without a line break ends one past the
    // end of the document.
    if (to > doc.length) return [first, doc.lines];
    const end = doc.lineAt(to);
    return [first, end.number - 1 + (to > end.from ? 1 : 0)];
  };
  const [a0, a1] = range(a, c.fromA, c.toA);
  const [b0, b1] = range(b, c.fromB, c.toB);
  return { a0, a1, b0, b1 };
}

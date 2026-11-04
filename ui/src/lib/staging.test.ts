import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import {
  buildIndex,
  chunkCheck,
  revertChunk,
  splitLines,
  stageState,
  toLineChunk,
  withChunk,
  withLine,
  type LineChunk,
} from "./staging";

// HEAD: a b c d e      worktree: a B c d e f
// block 0: b → B (H[1,2) → W[1,2)); block 1: + f at the end (H[5,5) → W[5,6)).
const H = splitLines("a\nb\nc\nd\ne\n");
const W = splitLines("a\nB\nc\nd\ne\nf\n");
const chunks: LineChunk[] = [
  { a0: 1, a1: 2, b0: 1, b1: 2 },
  { a0: 5, a1: 5, b0: 5, b1: 6 },
];

describe("splitLines", () => {
  it("keeps terminators", () => {
    expect(splitLines("a\r\nb")).toEqual(["a\r\n", "b"]);
    expect(splitLines("")).toEqual([]);
    expect(splitLines("x\n")).toEqual(["x\n"]);
  });
});

describe("stageState", () => {
  it("nothing staged when the index is HEAD", () => {
    const s = stageState(H, W, H, chunks);
    expect(s.map(chunkCheck)).toEqual([false, false]);
  });
  it("everything staged when the index is the worktree", () => {
    const s = stageState(H, W, W, chunks);
    expect(s.map(chunkCheck)).toEqual([true, true]);
  });
  it("one block staged", () => {
    const s = stageState(H, W, splitLines("a\nb\nc\nd\ne\nf\n"), chunks);
    expect(s.map(chunkCheck)).toEqual([false, true]);
  });
  it("not in the index at all", () => {
    expect(stageState([], W, null, [{ a0: 0, a1: 0, b0: 0, b1: 6 }]).map(chunkCheck)).toEqual([false]);
  });
  it("partially staged lines", () => {
    // HEAD x y, worktree X1 X2 y: block H[0,1) → W[0,2). Index stages X1 only, keeps x.
    const h = splitLines("x\ny\n");
    const w = splitLines("X1\nX2\ny\n");
    const c = [{ a0: 0, a1: 1, b0: 0, b1: 2 }];
    const s = stageState(h, w, splitLines("x\nX1\ny\n"), c);
    expect(s[0]).toEqual({ del: [false], add: [true, false] });
    expect(chunkCheck(s[0])).toBe("partial");
  });
});

describe("buildIndex", () => {
  it("round-trips every combination of blocks", () => {
    for (const a of [false, true])
      for (const b of [false, true]) {
        let s = stageState(H, W, H, chunks);
        s = withChunk(withChunk(s, 0, a), 1, b);
        const index = buildIndex(H, W, chunks, s);
        expect(stageState(H, W, splitLines(index), chunks).map(chunkCheck)).toEqual([a, b]);
      }
  });
  it("stages single lines", () => {
    const h = splitLines("x\ny\n");
    const w = splitLines("X1\nX2\ny\n");
    const c = [{ a0: 0, a1: 1, b0: 0, b1: 2 }];
    let s = stageState(h, w, h, c);
    s = withLine(s, 0, "add", 1);
    expect(buildIndex(h, w, c, s)).toBe("x\nX2\ny\n");
    s = withLine(s, 0, "del", 0);
    expect(buildIndex(h, w, c, s)).toBe("X2\ny\n");
  });
  it("adds a line break after a last line that had none", () => {
    const h = splitLines("a");
    const w = splitLines("a\nb\n");
    // The editor reports the last line as changed: "a" → "a\n" + "b\n".
    const c = [{ a0: 0, a1: 1, b0: 0, b1: 2 }];
    const s = withLine(withLine(stageState(h, w, h, c), 0, "add", 1), 0, "del", 0);
    expect(buildIndex(h, w, c, [{ del: [false], add: [false, true] }])).toBe("a\nb\n");
    expect(buildIndex(h, w, c, s)).toBe("b\n");
  });
  it("keeps CRLF", () => {
    const h = splitLines("a\r\nb\r\n");
    const w = splitLines("a\r\nB\r\n");
    const c = [{ a0: 1, a1: 2, b0: 1, b1: 2 }];
    expect(buildIndex(h, w, c, withChunk(stageState(h, w, h, c), 0, true))).toBe("a\r\nB\r\n");
  });
});

describe("revertChunk", () => {
  it("puts one block back", () => {
    expect(revertChunk(H, W, chunks, 0)).toBe("a\nb\nc\nd\ne\nf\n");
    expect(revertChunk(H, W, chunks, 1)).toBe("a\nB\nc\nd\ne\n");
  });
});

/** Chunks exactly as the diff editor computes them. */
function editorChunks(h: string, w: string): LineChunk[] {
  const a = Text.of(h.split("\n"));
  const b = Text.of(w.split("\n"));
  return Chunk.build(a, b).map((c) => toLineChunk(c, a, b));
}

describe("with the editor's own chunks", () => {
  const cases: [string, string][] = [
    ["a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\nf\n"], // change + append
    ["a\nb\nc\n", "b\nc\n"], // delete first line
    ["a\nb\nc\n", "a\nb\nc\nd\ne\n"], // append at end
    ["a\nb\nc", "a\nX\nc"], // no final newline
    ["a\nb\nc", "a\nb\nc\nd"], // append after a last line without newline
    ["", "new\nfile\n"], // new file
    ["x\ny\nz\n", "x\nz\n"], // pure deletion in the middle
    ["1\n2\n3\n4\n5\n6\n7\n8\n", "1\nTWO\n3\n4\n5\n6\nSEVEN\n8\n9\n"],
  ];
  it.each(cases)("staging all / nothing round-trips: %j → %j", (h, w) => {
    const H = splitLines(h);
    const W = splitLines(w);
    const chunks = editorChunks(h, w);
    expect(chunks.length).toBeGreaterThan(0);
    const none = stageState(H, W, H, chunks);
    expect(buildIndex(H, W, chunks, none)).toBe(h);
    const all = chunks.reduce((s, _, k) => withChunk(s, k, true), none);
    expect(buildIndex(H, W, chunks, all)).toBe(w);
    expect(stageState(H, W, W, chunks).map(chunkCheck)).toEqual(chunks.map(() => true));
    // Each block alone is recognised again from the index it produces.
    chunks.forEach((_, k) => {
      const one = buildIndex(H, W, chunks, withChunk(none, k, true));
      expect(stageState(H, W, splitLines(one), chunks).map(chunkCheck)).toEqual(chunks.map((_, i) => i === k));
      // And reverting it from the worktree gives back HEAD's version of it.
      if (chunks.length === 1) expect(revertChunk(H, W, chunks, k)).toBe(h);
    });
  });
});

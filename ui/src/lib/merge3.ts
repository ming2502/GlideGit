// Three-way merge into regions, for the merge tool.
//
// Diffs base→ours and base→theirs (line chunks from @codemirror/merge, the
// same algorithm as the diff editor) and groups changes whose base ranges
// overlap or touch. A group with changes from one side only is
// non-conflicting; with changes from both it is a conflict unless both made
// the same change. Like git, changes on adjacent lines count as overlapping.
import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { splitLines, toLineChunk, type LineChunk } from "./staging";

export type RegionKind = "ours" | "theirs" | "same" | "conflict";

/** A changed region: half-open line ranges in each version. */
export interface Region {
  kind: RegionKind;
  base: [number, number];
  ours: [number, number];
  theirs: [number, number];
}

export interface Merge3 {
  base: string[];
  ours: string[];
  theirs: string[];
  regions: Region[];
}

function lineChunks(a: string, b: string): LineChunk[] {
  const ta = Text.of(a.split("\n"));
  const tb = Text.of(b.split("\n"));
  return Chunk.build(ta, tb).map((c) => toLineChunk(c, ta, tb));
}

/** Where base lines [lo, hi) are in a side, given that side's chunks in the group. */
function sideRange(all: LineChunk[], members: LineChunk[], lo: number, hi: number): [number, number] {
  if (members.length) {
    const first = members[0];
    const last = members[members.length - 1];
    return [first.b0 - (first.a0 - lo), last.b1 + (hi - last.a1)];
  }
  // No change on this side here: shift by the size changes before it.
  let delta = 0;
  for (const c of all) if (c.a1 <= lo && !(c.a0 === c.a1 && c.a0 === lo)) delta += c.b1 - c.b0 - (c.a1 - c.a0);
  return [lo + delta, hi + delta];
}

const sameLines = (x: string[], y: string[]) => x.length === y.length && x.every((l, i) => l === y[i]);

export function merge3(base: string, ours: string, theirs: string): Merge3 {
  const B = splitLines(base);
  const O = splitLines(ours);
  const T = splitLines(theirs);
  const oc = lineChunks(base, ours);
  const tc = lineChunks(base, theirs);
  const items = [...oc.map((c) => ({ side: "o" as const, c })), ...tc.map((c) => ({ side: "t" as const, c }))].sort(
    (x, y) => x.c.a0 - y.c.a0 || x.c.a1 - y.c.a1,
  );

  const regions: Region[] = [];
  let i = 0;
  while (i < items.length) {
    let lo = items[i].c.a0;
    let hi = items[i].c.a1;
    const members = { o: [] as LineChunk[], t: [] as LineChunk[] };
    // Take every change that overlaps or touches the group so far.
    while (i < items.length && (items[i].c.a0 < hi || items[i].c.a0 === hi || members.o.length + members.t.length === 0)) {
      const { side, c } = items[i];
      members[side].push(c);
      lo = Math.min(lo, c.a0);
      hi = Math.max(hi, c.a1);
      i++;
    }
    const o = sideRange(oc, members.o, lo, hi);
    const t = sideRange(tc, members.t, lo, hi);
    let kind: RegionKind;
    if (members.o.length && members.t.length) kind = sameLines(O.slice(...o), T.slice(...t)) ? "same" : "conflict";
    else kind = members.o.length ? "ours" : "theirs";
    regions.push({ kind, base: [lo, hi], ours: o, theirs: t });
  }
  return { base: B, ours: O, theirs: T, regions };
}

/** Join lines, adding a line break where a line without one is followed by more. */
export function joinLines(lines: string[]): string {
  const eol = lines.some((l) => l.endsWith("\r\n")) ? "\r\n" : "\n";
  let s = "";
  lines.forEach((l, i) => {
    s += l;
    if (i < lines.length - 1 && !l.endsWith("\n")) s += eol;
  });
  return s;
}

/** The text a region takes when resolved with `choice`. */
export function regionText(m: Merge3, r: Region, choice: "base" | "ours" | "theirs" | "both"): string {
  const pick = (side: "base" | "ours" | "theirs") => m[side].slice(...r[side]);
  if (choice === "both") return joinLines([...pick("ours"), ...pick("theirs")]);
  return joinLines(pick(choice));
}

/** Everything non-conflicting applied, conflicts left at base (the "magic wand"). */
export function autoMerge(m: Merge3): string {
  const out: string[] = [];
  let prev = 0;
  for (const r of m.regions) {
    out.push(...m.base.slice(prev, r.base[0]));
    if (r.kind === "ours" || r.kind === "same") out.push(...m.ours.slice(...r.ours));
    else if (r.kind === "theirs") out.push(...m.theirs.slice(...r.theirs));
    else out.push(...m.base.slice(...r.base));
    prev = r.base[1];
  }
  out.push(...m.base.slice(prev));
  return joinLines(out);
}

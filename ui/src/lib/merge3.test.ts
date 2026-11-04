import { describe, expect, it } from "vitest";
import { autoMerge, merge3, regionText } from "./merge3";

const kinds = (base: string, ours: string, theirs: string) => merge3(base, ours, theirs).regions.map((r) => r.kind);

describe("merge3", () => {
  it("separate changes do not conflict", () => {
    const base = "1\n2\n3\n4\n5\n6\n7\n";
    const ours = "1\nTWO\n3\n4\n5\n6\n7\n";
    const theirs = "1\n2\n3\n4\n5\nSIX\n7\n";
    const m = merge3(base, ours, theirs);
    expect(m.regions.map((r) => r.kind)).toEqual(["ours", "theirs"]);
    expect(autoMerge(m)).toBe("1\nTWO\n3\n4\n5\nSIX\n7\n");
  });

  it("the same line changed differently conflicts", () => {
    const m = merge3("a\nb\nc\n", "a\nX\nc\n", "a\nY\nc\n");
    expect(m.regions.map((r) => r.kind)).toEqual(["conflict"]);
    const r = m.regions[0];
    expect(regionText(m, r, "ours")).toBe("X\n");
    expect(regionText(m, r, "theirs")).toBe("Y\n");
    expect(regionText(m, r, "both")).toBe("X\nY\n");
    expect(regionText(m, r, "base")).toBe("b\n");
    // The wand leaves conflicts alone.
    expect(autoMerge(m)).toBe("a\nb\nc\n");
  });

  it("identical changes on both sides merge", () => {
    expect(kinds("a\nb\nc\n", "a\nZ\nc\n", "a\nZ\nc\n")).toEqual(["same"]);
  });

  it("adjacent lines conflict, like git", () => {
    expect(kinds("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n")).toEqual(["conflict"]);
  });

  it("insertions at the same place conflict", () => {
    const m = merge3("a\nb\n", "a\nours\nb\n", "a\ntheirs\nb\n");
    expect(m.regions.map((r) => r.kind)).toEqual(["conflict"]);
    expect(regionText(m, m.regions[0], "both")).toBe("ours\ntheirs\n");
  });

  it("maps ranges past earlier size changes", () => {
    // Ours adds two lines at the top; theirs changes line 5.
    const base = "1\n2\n3\n4\n5\n6\n";
    const ours = "0a\n0b\n1\n2\n3\n4\n5\n6\n";
    const theirs = "1\n2\n3\n4\nFIVE\n6\n";
    const m = merge3(base, ours, theirs);
    expect(m.regions.map((r) => r.kind)).toEqual(["ours", "theirs"]);
    const r = m.regions[1];
    expect(m.ours.slice(...r.ours)).toEqual(["5\n"]);
    expect(autoMerge(m)).toBe("0a\n0b\n1\n2\n3\n4\nFIVE\n6\n");
  });

  it("deletion against an edit elsewhere", () => {
    const m = merge3("a\nb\nc\nd\ne\n", "a\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
    expect(m.regions.map((r) => r.kind)).toEqual(["ours", "theirs"]);
    expect(autoMerge(m)).toBe("a\nc\nd\nE\n");
  });

  it("files without a final newline", () => {
    const m = merge3("a\nb", "a\nb\nours", "x\na\nb");
    expect(m.regions.map((r) => r.kind)).toEqual(["theirs", "ours"]);
    expect(autoMerge(m)).toBe("x\na\nb\nours");
  });

  it("no changes at all", () => {
    expect(kinds("a\n", "a\n", "a\n")).toEqual([]);
  });
});

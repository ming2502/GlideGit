import { describe, expect, it } from "vitest";
import { bestMatch, fuzzyMatch } from "./fuzzy";

const rank = (q: string, items: string[]) =>
  items
    .map((t) => ({ t, m: fuzzyMatch(q, t) }))
    .filter((x) => x.m)
    .sort((a, b) => b.m!.score - a.m!.score)
    .map((x) => x.t);

describe("fuzzyMatch", () => {
  it("matches characters in order, case-insensitively", () => {
    expect(fuzzyMatch("cmt", "Commit")?.positions).toEqual([0, 2, 5]);
    expect(fuzzyMatch("tmc", "Commit")).toBeNull();
    expect(fuzzyMatch("", "Commit")?.score).toBe(0);
  });

  it("prefers substrings, prefixes and word starts", () => {
    expect(rank("push", ["Commit and Push", "Push…", "Show Stash"])).toEqual(["Push…", "Commit and Push"]);
    expect(rank("cap", ["Commit and Push", "Cherry Pick", "Escape"])[0]).toBe("Commit and Push");
    expect(rank("nc", ["Next Change", "Previous Change", "Fetch"])[0]).toBe("Next Change");
  });

  it("uses word starts over earlier inner letters", () => {
    // "s" should land on "Stash", not the "s" inside "Changes".
    expect(fuzzyMatch("cs", "Changes Stash")?.positions).toEqual([0, 8]);
  });

  it("starts a scattered match at a word start", () => {
    expect(fuzzyMatch("main", "compare feature/log-index")).toBeNull();
    expect(fuzzyMatch("main", "main ⇄ feature/x")).not.toBeNull();
    expect(fuzzyMatch("mtl", "Move to Another Changelist")).not.toBeNull();
  });

  it("handles Chinese text and spaces in the query", () => {
    expect(fuzzyMatch("撤销", "打开撤销中心")?.positions).toEqual([2, 3]);
    expect(fuzzyMatch("move list", "Move to Another Changelist…")).not.toBeNull();
  });

  it("ranks alias-only matches below title matches", () => {
    const title = bestMatch("stash", "Stash Changes…", ["暂存"]);
    const alias = bestMatch("暂存", "Stash Changes…", ["暂存修改"]);
    expect(title!.title).not.toBeNull();
    expect(alias!.title).toBeNull();
    expect(alias!.score).toBeLessThan(title!.score);
    expect(bestMatch("zzz", "Fetch", ["拉取"])).toBeNull();
  });
});

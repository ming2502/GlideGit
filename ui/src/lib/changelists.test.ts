import { describe, expect, it } from "vitest";
import type { FileChange } from "../bindings/FileChange";
import { addList, emptyChangelists, groupChanges, listOf, moveFiles, prune, removeList, renameList, setActive } from "./changelists";

const change = (path: string): FileChange => ({ path, origPath: null, staged: null, unstaged: "modified", conflicted: false });

describe("changelists", () => {
  it("groups by list, unassigned files in the active one", () => {
    let [s, id] = addList(emptyChangelists(), "Parser work");
    expect(id).toBe("parser-work");
    s = moveFiles(s, ["b.rs"], id);
    const g = groupChanges(s, [change("a.rs"), change("b.rs")]);
    expect(g.map((x) => [x.list.name, x.changes.map((c) => c.path)])).toEqual([
      ["Changes", ["a.rs"]],
      ["Parser work", ["b.rs"]],
    ]);
    // Moving into the active list drops the entry.
    expect(moveFiles(s, ["b.rs"], "default").files).toEqual({});
  });

  it("reuses names and makes ids unique", () => {
    let [s, a] = addList(emptyChangelists(), "UI");
    const [s2, again] = addList(s, " ui ");
    expect(again).toBe(a);
    expect(s2).toBe(s);
    [s] = addList(renameList(s, a, "Other"), "UI");
    expect(s.lists.map((l) => l.id)).toEqual(["default", "ui", "ui-2"]);
  });

  it("switching the active list keeps files where they were", () => {
    let [s, id] = addList(emptyChangelists(), "Feature");
    s = moveFiles(s, ["f.rs"], id);
    s = setActive(s, id, ["a.rs", "f.rs"]);
    expect(s.active).toBe(id);
    expect(listOf(s, "a.rs")).toBe("default");
    expect(listOf(s, "f.rs")).toBe(id);
    expect(listOf(s, "new.rs")).toBe(id);
    expect(s.files).toEqual({ "a.rs": "default" });
  });

  it("removing a list moves its files to the default list", () => {
    let [s, id] = addList(emptyChangelists(), "Feature");
    s = setActive(moveFiles(s, ["f.rs"], id), id, ["a.rs", "f.rs"]);
    s = removeList(s, id);
    expect(s.active).toBe("default");
    expect(s.lists).toHaveLength(1);
    expect(listOf(s, "f.rs")).toBe("default");
    expect(s.files).toEqual({});
    expect(removeList(s, "default")).toBe(s);
  });

  it("prunes files without changes", () => {
    let [s, id] = addList(emptyChangelists(), "X");
    s = moveFiles(s, ["a", "b"], id);
    expect(prune(s, ["a"]).files).toEqual({ a: id });
    expect(prune(s, ["a", "b"])).toBe(s);
  });
});

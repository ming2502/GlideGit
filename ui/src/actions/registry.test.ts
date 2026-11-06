import { describe, expect, it } from "vitest";
import { chordFromEvent, chordOwner, declareActions, formatKeys, keysOf, resetAllShortcuts, setShortcut } from "./registry";
import { isMac } from "../lib/format";

const press = (code: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) => ({
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});
const mod = isMac ? { metaKey: true } : { ctrlKey: true };

describe("shortcuts", () => {
  it("records chords from key presses", () => {
    expect(chordFromEvent(press("KeyK", { ...mod, shiftKey: true }))).toBe("mod+shift+k");
    expect(chordFromEvent(press("Comma", mod))).toBe("mod+comma");
    expect(chordFromEvent(press("F6"))).toBe("f6");
    // A modifier on its own is not a chord yet.
    expect(chordFromEvent(press("ShiftLeft", { shiftKey: true }))).toBeNull();
    expect(chordFromEvent(press("MetaLeft", { metaKey: true }))).toBeNull();
  });

  it("overrides, conflicts and reset", () => {
    declareActions([
      { id: "t.a", title: "A", keys: ["mod+k"] },
      { id: "t.b", title: "B" },
    ]);
    expect(chordOwner("mod+k", "t.b")).toBe("t.a");
    setShortcut("t.b", ["mod+shift+y"]);
    expect(keysOf({ id: "t.b" })).toEqual(["mod+shift+y"]);
    setShortcut("t.a", []);
    expect(keysOf({ id: "t.a", keys: ["mod+k"] })).toEqual([]);
    expect(chordOwner("mod+k", "t.b")).toBeNull();
    setShortcut("t.a", null);
    expect(keysOf({ id: "t.a", keys: ["mod+k"] })).toEqual(["mod+k"]);
    resetAllShortcuts();
    expect(keysOf({ id: "t.b" })).toEqual([]);
    expect(formatKeys(["mod+comma"])).toBe(isMac ? "⌘," : "Ctrl+,");
  });
});

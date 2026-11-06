// Single source of truth for every user-invokable action. Menus, buttons,
// shortcuts and the command palette all resolve through here.
import { useEffect, useRef, useSyncExternalStore } from "react";
import { isMac } from "../lib/format";
import { load, save } from "../lib/storage";

export interface Action {
  id: string;
  title: string;
  /** e.g. "mod+shift+k", "mod+enter", "mod+shift+backquote", "escape" */
  keys?: string[];
  run: () => void;
  enabled?: () => boolean;
  /** Other words the command palette should find it by (e.g. Chinese names). */
  keywords?: string;
  /** Leave out of the command palette. */
  hidden?: boolean;
}

const registry = new Map<string, Action>();

/** Every action registered so far (also after its view unmounts), for the shortcut settings. */
const catalog = new Map<string, { title: string; keys: string[]; hidden: boolean }>();

// ---------------------------------------------------------------- keymap
// The user's shortcut changes: action id → keys (an empty list unbinds).

let overrides: Record<string, string[]> = load("keymap", {});
let keymapVersion = 0;
const keymapListeners = new Set<() => void>();

/** The keys an action answers to now: the user's choice, else its default. */
export function keysOf(a: { id: string; keys?: string[] }): string[] {
  return overrides[a.id] ?? a.keys ?? [];
}

/** Set an action's shortcut (`null`: back to the default). */
export function setShortcut(id: string, keys: string[] | null): void {
  const next = { ...overrides };
  if (keys == null) delete next[id];
  else next[id] = keys;
  overrides = next;
  save("keymap", overrides);
  keymapVersion++;
  for (const l of keymapListeners) l();
}

export function resetAllShortcuts(): void {
  overrides = {};
  save("keymap", overrides);
  keymapVersion++;
  for (const l of keymapListeners) l();
}

export function isCustomized(id: string): boolean {
  return id in overrides;
}

/** Re-render when shortcuts change (labels show them). */
export function useKeymapVersion(): number {
  return useSyncExternalStore(
    (cb) => {
      keymapListeners.add(cb);
      return () => keymapListeners.delete(cb);
    },
    () => keymapVersion,
  );
}

export interface CatalogEntry {
  id: string;
  title: string;
  defaultKeys: string[];
  keys: string[];
}

/** Every known action that can have a shortcut, by title. */
export function shortcutCatalog(): CatalogEntry[] {
  return [...catalog.entries()]
    .filter(([id, c]) => !c.hidden || id === "palette.open")
    .map(([id, c]) => ({ id, title: c.title, defaultKeys: c.keys, keys: keysOf({ id, keys: c.keys }) }))
    .sort((a, b) => a.title.localeCompare(b.title));
}

/** The action (other than `except`) already using this chord. */
export function chordOwner(chord: string, except: string): string | null {
  for (const [id, c] of catalog) {
    if (id !== except && keysOf({ id, keys: c.keys }).includes(chord)) return id;
  }
  return null;
}

/** A chord string ("mod+shift+k") for a key press; null for a lone modifier. */
export function chordFromEvent(e: KeyboardEvent | { code: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }): string | null {
  const key = eventKey(e as KeyboardEvent);
  if (/^(meta|control|alt|shift)(left|right)?$/.test(key) || key === "") return null;
  const parts: string[] = [];
  if (isMac ? e.metaKey : e.ctrlKey) parts.push("mod");
  if (isMac && e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  parts.push(key);
  return parts.join("+");
}

/** What an action is, without how it runs: declared up front by views. */
export type ActionInfo = Pick<Action, "id" | "title" | "keys" | "keywords" | "hidden">;

/**
 * Make actions known (for the shortcut settings) before their view mounts.
 * Views call this at module load with the same descriptions they register.
 */
export function declareActions(actions: ActionInfo[]): void {
  for (const a of actions) {
    if (!catalog.has(a.id)) catalog.set(a.id, { title: a.title, keys: a.keys ?? [], hidden: !!a.hidden });
  }
}

/** Index declared actions by id, declaring them at the same time. */
export function defineActions<const T extends readonly ActionInfo[]>(actions: T): { [K in T[number]["id"]]: ActionInfo } {
  declareActions([...actions]);
  return Object.fromEntries(actions.map((a) => [a.id, a])) as { [K in T[number]["id"]]: ActionInfo };
}

export function registerActions(actions: Action[]): () => void {
  for (const a of actions) {
    registry.set(a.id, a);
    catalog.set(a.id, { title: a.title, keys: a.keys ?? [], hidden: !!a.hidden });
  }
  return () => {
    for (const a of actions) if (registry.get(a.id) === a) registry.delete(a.id);
  };
}

export function runAction(id: string): boolean {
  const a = registry.get(id);
  if (!a || (a.enabled && !a.enabled())) return false;
  a.run();
  return true;
}

export function getAction(id: string): Action | undefined {
  return registry.get(id);
}

/** Every registered action, for the command palette. */
export function listActions(): Action[] {
  return [...registry.values()];
}

/** Register actions for the lifetime of a component; always calls the latest closures. */
export function useActions(actions: Action[]): void {
  const latest = useRef(actions);
  latest.current = actions;
  const ids = actions.map((a) => a.id).join("|");
  useEffect(() => {
    const proxies: Action[] = latest.current.map((a, i) => ({
      id: a.id,
      title: a.title,
      keys: a.keys,
      keywords: a.keywords,
      hidden: a.hidden,
      run: () => latest.current[i]?.run(),
      enabled: () => latest.current[i]?.enabled?.() ?? true,
    }));
    return registerActions(proxies);
  }, [ids]);
}

interface Chord {
  mod: boolean;
  /** Literal Ctrl (distinct from `mod` on macOS). */
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

function parseChord(s: string): Chord {
  const parts = s.toLowerCase().split("+");
  return {
    mod: parts.includes("mod"),
    ctrl: parts.includes("ctrl"),
    shift: parts.includes("shift"),
    alt: parts.includes("alt"),
    key: parts[parts.length - 1],
  };
}

function eventKey(e: KeyboardEvent): string {
  const c = e.code;
  if (c.startsWith("Key")) return c.slice(3).toLowerCase();
  if (c.startsWith("Digit")) return c.slice(5);
  return c.toLowerCase(); // enter, escape, backquote, arrowup, f5 ...
}

function matches(chord: Chord, e: KeyboardEvent): boolean {
  // On Windows/Linux `mod` is Ctrl, so a chord may name either.
  const wantCtrl = chord.ctrl || (!isMac && chord.mod);
  const wantMeta = isMac && chord.mod;
  return (
    wantCtrl === e.ctrlKey &&
    wantMeta === e.metaKey &&
    chord.shift === e.shiftKey &&
    chord.alt === e.altKey &&
    chord.key === eventKey(e)
  );
}

function isTextInput(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return t.isContentEditable || t.tagName === "INPUT" || t.tagName === "TEXTAREA";
}

export function installKeyHandler(): () => void {
  const handler = (e: KeyboardEvent) => {
    // A modal dialog/popup owns the keyboard; global shortcuts pause meanwhile.
    if (document.querySelector('[aria-modal="true"]')) return;
    const inText = isTextInput(e.target);
    for (const a of registry.values()) {
      for (const k of keysOf(a)) {
        const chord = parseChord(k);
        // Plain keys belong to the focused text field.
        if (inText && !chord.mod && !chord.ctrl && chord.key !== "escape") continue;
        if (matches(chord, e) && (!a.enabled || a.enabled())) {
          e.preventDefault();
          e.stopPropagation();
          a.run();
          return;
        }
      }
    }
  };
  window.addEventListener("keydown", handler, true);
  return () => window.removeEventListener("keydown", handler, true);
}

const PUNCTUATION: Record<string, string> = {
  comma: ",",
  period: ".",
  slash: "/",
  backslash: "\\",
  semicolon: ";",
  quote: "'",
  minus: "-",
  equal: "=",
};

const MAC_SYMBOLS: Record<string, string> = {
  mod: "⌘",
  ctrl: "⌃",
  bracketleft: "[",
  bracketright: "]",
  tab: "⇥",
  shift: "⇧",
  alt: "⌥",
  enter: "↩",
  escape: "⎋",
  backquote: "`",
  ...PUNCTUATION,
  space: "Space",
  backspace: "⌫",
  delete: "⌦",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};
const WIN_NAMES: Record<string, string> = {
  mod: "Ctrl",
  ctrl: "Ctrl",
  bracketleft: "[",
  bracketright: "]",
  tab: "Tab",
  shift: "Shift",
  alt: "Alt",
  enter: "Enter",
  escape: "Esc",
  backquote: "`",
  ...PUNCTUATION,
  space: "Space",
  backspace: "Backspace",
  delete: "Delete",
  arrowup: "Up",
  arrowdown: "Down",
  arrowleft: "Left",
  arrowright: "Right",
};

export function formatKeys(keys?: string[]): string {
  if (!keys?.length) return "";
  const parts = keys[0].toLowerCase().split("+");
  const map = isMac ? MAC_SYMBOLS : WIN_NAMES;
  const out = parts.map((p) => map[p] ?? p.toUpperCase());
  return isMac ? out.join("") : out.join("+");
}

export function shortcutOf(id: string): string {
  const a = registry.get(id);
  return a ? formatKeys(keysOf(a)) : "";
}

// Single source of truth for every user-invokable action. Menus, buttons,
// shortcuts and (later) the command palette all resolve through here.
import { useEffect, useRef } from "react";
import { isMac } from "../lib/format";

export interface Action {
  id: string;
  title: string;
  /** e.g. "mod+shift+k", "mod+enter", "mod+shift+backquote", "escape" */
  keys?: string[];
  run: () => void;
  enabled?: () => boolean;
}

const registry = new Map<string, Action>();

export function registerActions(actions: Action[]): () => void {
  for (const a of actions) registry.set(a.id, a);
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
      run: () => latest.current[i]?.run(),
      enabled: () => latest.current[i]?.enabled?.() ?? true,
    }));
    return registerActions(proxies);
  }, [ids]);
}

interface Chord {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

function parseChord(s: string): Chord {
  const parts = s.toLowerCase().split("+");
  return {
    mod: parts.includes("mod"),
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
  const mod = isMac ? e.metaKey : e.ctrlKey;
  return (
    chord.mod === mod &&
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
      for (const k of a.keys ?? []) {
        const chord = parseChord(k);
        // Plain keys belong to the focused text field.
        if (inText && !chord.mod && chord.key !== "escape") continue;
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

const MAC_SYMBOLS: Record<string, string> = {
  mod: "⌘",
  shift: "⇧",
  alt: "⌥",
  enter: "↩",
  escape: "⎋",
  backquote: "`",
};
const WIN_NAMES: Record<string, string> = {
  mod: "Ctrl",
  shift: "Shift",
  alt: "Alt",
  enter: "Enter",
  escape: "Esc",
  backquote: "`",
};

export function formatKeys(keys?: string[]): string {
  if (!keys?.length) return "";
  const parts = keys[0].toLowerCase().split("+");
  const map = isMac ? MAC_SYMBOLS : WIN_NAMES;
  const out = parts.map((p) => map[p] ?? p.toUpperCase());
  return isMac ? out.join("") : out.join("+");
}

export function shortcutOf(id: string): string {
  return formatKeys(registry.get(id)?.keys);
}

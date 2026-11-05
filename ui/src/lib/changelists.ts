// Changelists: named groups of changed files. Files without an entry in
// `files` belong to the active list. All functions return new state.
import type { Changelist } from "../bindings/Changelist";
import type { Changelists } from "../bindings/Changelists";
import type { FileChange } from "../bindings/FileChange";

export const DEFAULT_ID = "default";

export const emptyChangelists = (): Changelists => ({
  lists: [{ id: DEFAULT_ID, name: "Changes" }],
  active: DEFAULT_ID,
  files: {},
});

export function listOf(state: Changelists, path: string): string {
  const id = state.files[path];
  return id && state.lists.some((l) => l.id === id) ? id : state.active;
}

/** Changes grouped by list, in list order; empty lists are kept. */
export function groupChanges(state: Changelists, changes: FileChange[]): { list: Changelist; changes: FileChange[] }[] {
  const by = new Map<string, FileChange[]>(state.lists.map((l) => [l.id, []]));
  for (const c of changes) by.get(listOf(state, c.path))!.push(c);
  return state.lists.map((list) => ({ list, changes: by.get(list.id)! }));
}

export function moveFiles(state: Changelists, paths: string[], listId: string): Changelists {
  const files = { ...state.files };
  for (const p of paths) {
    if (listId === state.active) delete files[p];
    else files[p] = listId;
  }
  return { ...state, files };
}

/** A new list's id: from its name, unique among the lists. */
function newId(state: Changelists, name: string): string {
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "list";
  let id = base;
  for (let n = 2; state.lists.some((l) => l.id === id); n++) id = `${base}-${n}`;
  return id;
}

export function findByName(state: Changelists, name: string): Changelist | undefined {
  const n = name.trim().toLowerCase();
  return state.lists.find((l) => l.name.toLowerCase() === n);
}

/** Add a list (or reuse one with the same name). Returns the state and its id. */
export function addList(state: Changelists, name: string): [Changelists, string] {
  const existing = findByName(state, name);
  if (existing) return [state, existing.id];
  const id = newId(state, name);
  return [{ ...state, lists: [...state.lists, { id, name: name.trim() }] }, id];
}

export function renameList(state: Changelists, id: string, name: string): Changelists {
  return { ...state, lists: state.lists.map((l) => (l.id === id ? { ...l, name: name.trim() } : l)) };
}

/** Remove a list; its files go to the default list (which becomes active if this one was). */
export function removeList(state: Changelists, id: string): Changelists {
  if (id === DEFAULT_ID) return state;
  const active = state.active === id ? DEFAULT_ID : state.active;
  const files: Record<string, string> = {};
  for (const [p, l] of Object.entries(state.files)) {
    if (l === id) {
      if (active !== DEFAULT_ID) files[p] = DEFAULT_ID;
    } else if (l !== active) files[p] = l;
  }
  // Files that were implicitly in the removed (active) list need nothing: they now fall to the default.
  return { lists: state.lists.filter((l) => l.id !== id), active, files };
}

/**
 * Make `id` the active list. `paths` are the current changes: those in the
 * old active list without an entry get one, so they stay where they are.
 */
export function setActive(state: Changelists, id: string, paths: string[]): Changelists {
  if (id === state.active || !state.lists.some((l) => l.id === id)) return state;
  const files = { ...state.files };
  for (const p of paths) if (!(p in files) || !state.lists.some((l) => l.id === files[p])) files[p] = state.active;
  for (const [p, l] of Object.entries(files)) if (l === id) delete files[p];
  return { ...state, active: id, files };
}

/** Forget files that no longer have changes. */
export function prune(state: Changelists, paths: string[]): Changelists {
  const keep = new Set(paths);
  const files: Record<string, string> = {};
  let changed = false;
  for (const [p, l] of Object.entries(state.files)) {
    if (keep.has(p)) files[p] = l;
    else changed = true;
  }
  return changed ? { ...state, files } : state;
}

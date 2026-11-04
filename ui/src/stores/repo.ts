import { create } from "zustand";
import { api, errorMessage, type PullMode } from "../api";
import type { DiffMode } from "../bindings/DiffMode";
import type { FileChange } from "../bindings/FileChange";
import type { FileDiff } from "../bindings/FileDiff";
import type { RepoSnapshot } from "../bindings/RepoSnapshot";
import { load, save } from "../lib/storage";

export type View = "changes" | "log";

export interface Toast {
  id: number;
  kind: "error" | "info" | "success";
  title: string;
  detail?: string;
}

export interface Task {
  id: number;
  title: string;
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

interface RepoState {
  root: string | null;
  snapshot: RepoSnapshot | null;
  view: View;
  recent: string[];

  selectedPath: string | null;
  diff: FileDiff | null;
  diffLoading: boolean;
  /** Optimistic include/exclude state, cleared by the next snapshot. */
  pendingStage: Record<string, boolean>;

  tasks: Task[];
  toasts: Toast[];
  branchPopup: boolean;
  confirm: ConfirmRequest | null;
  pullMode: PullMode;
  logVersion: number;

  openRepo: (path: string) => Promise<void>;
  closeRepo: () => Promise<void>;
  applySnapshot: (root: string, snapshot: RepoSnapshot) => void;
  setView: (v: View) => void;
  select: (path: string | null) => void;
  reloadDiff: () => Promise<void>;
  setIncluded: (changes: FileChange[], include: boolean) => Promise<void>;
  runTask: <T>(title: string, fn: (root: string) => Promise<T>, success?: string) => Promise<T | undefined>;
  toast: (t: Omit<Toast, "id">) => void;
  dismissToast: (id: number) => void;
  setBranchPopup: (open: boolean) => void;
  ask: (req: Omit<ConfirmRequest, "resolve">) => Promise<boolean>;
  setPullMode: (m: PullMode) => void;
}

let seq = 0;
let diffSeq = 0;

/** A file counts as "included" in the commit when nothing remains unstaged. */
export function isIncluded(c: FileChange): boolean | "partial" {
  if (c.conflicted) return false;
  if (c.staged && c.unstaged) return "partial";
  return c.staged != null;
}

export const useRepo = create<RepoState>((set, get) => ({
  root: null,
  snapshot: null,
  view: "changes",
  recent: load<string[]>("recentRepos", []),
  selectedPath: null,
  diff: null,
  diffLoading: false,
  pendingStage: {},
  tasks: [],
  toasts: [],
  branchPopup: false,
  confirm: null,
  pullMode: load<PullMode>("pullMode", "merge"),
  logVersion: 0,

  async openRepo(path) {
    try {
      const snapshot = await api.openRepo(path);
      const prev = get().root;
      if (prev && prev !== snapshot.root) void api.closeRepo(prev);
      const recent = [snapshot.root, ...get().recent.filter((r) => r !== snapshot.root)].slice(0, 15);
      save("recentRepos", recent);
      set({
        root: snapshot.root,
        snapshot,
        recent,
        selectedPath: null,
        diff: null,
        pendingStage: {},
        view: "changes",
      });
    } catch (e) {
      get().toast({ kind: "error", title: "无法打开仓库", detail: errorMessage(e) });
    }
  },

  async closeRepo() {
    const root = get().root;
    if (root) await api.closeRepo(root).catch(() => {});
    set({ root: null, snapshot: null, selectedPath: null, diff: null });
  },

  applySnapshot(root, snapshot) {
    if (root !== get().root) return;
    const { selectedPath } = get();
    const stillThere = selectedPath && snapshot.changes.some((c) => c.path === selectedPath);
    set((s) => ({
      snapshot,
      pendingStage: {},
      logVersion: s.logVersion + 1,
      ...(stillThere ? {} : { selectedPath: null, diff: null }),
    }));
    if (stillThere) void get().reloadDiff();
  },

  setView: (view) => set({ view }),

  select(path) {
    set({ selectedPath: path });
    void get().reloadDiff();
  },

  async reloadDiff() {
    const { root, selectedPath, snapshot } = get();
    const mine = ++diffSeq;
    if (!root || !selectedPath || !snapshot) {
      set({ diff: null, diffLoading: false });
      return;
    }
    const change = snapshot.changes.find((c) => c.path === selectedPath);
    if (!change) return;
    const mode: DiffMode = change.conflicted ? "workingTree" : "head";
    set({ diffLoading: true });
    try {
      const diff = await api.diffFile(root, selectedPath, mode, change.unstaged === "untracked");
      if (mine === diffSeq) set({ diff, diffLoading: false }); // drop stale responses
    } catch (e) {
      if (mine === diffSeq) {
        set({ diff: null, diffLoading: false });
        get().toast({ kind: "error", title: "无法加载 diff", detail: errorMessage(e) });
      }
    }
  },

  async setIncluded(changes, include) {
    const root = get().root;
    if (!root || changes.length === 0) return;
    const paths = changes.flatMap((c) => (c.origPath && !include ? [c.path, c.origPath] : [c.path]));
    set((s) => {
      const pending = { ...s.pendingStage };
      for (const c of changes) pending[c.path] = include;
      return { pendingStage: pending };
    });
    try {
      await (include ? api.stage(root, paths) : api.unstage(root, paths));
    } catch (e) {
      set({ pendingStage: {} });
      get().toast({ kind: "error", title: include ? "Stage 失败" : "Unstage 失败", detail: errorMessage(e) });
    }
  },

  async runTask(title, fn, success) {
    const root = get().root;
    if (!root) return undefined;
    const id = ++seq;
    set((s) => ({ tasks: [...s.tasks, { id, title }] }));
    try {
      const r = await fn(root);
      if (success) get().toast({ kind: "success", title: success });
      return r;
    } catch (e) {
      get().toast({ kind: "error", title: `${title}失败`, detail: errorMessage(e) });
      return undefined;
    } finally {
      set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) }));
    }
  },

  toast(t) {
    const id = ++seq;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { ...t, id }] }));
    const ttl = t.kind === "error" ? 9000 : 3500;
    setTimeout(() => get().dismissToast(id), ttl);
  },

  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setBranchPopup: (branchPopup) => set({ branchPopup }),

  ask(req) {
    return new Promise<boolean>((resolve) => {
      set({
        confirm: {
          ...req,
          resolve: (ok) => {
            set({ confirm: null });
            resolve(ok);
          },
        },
      });
    });
  },

  setPullMode(pullMode) {
    save("pullMode", pullMode);
    set({ pullMode });
  },
}));

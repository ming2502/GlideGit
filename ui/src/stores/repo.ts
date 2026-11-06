import { create } from "zustand";
import { api, errorMessage, isCancelled, type PullMode } from "../api";
import type { Progress } from "../bindings/Progress";
import type { DiffMode } from "../bindings/DiffMode";
import type { FileChange } from "../bindings/FileChange";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { FileTexts } from "../bindings/FileTexts";
import type { RepoSnapshot } from "../bindings/RepoSnapshot";
import type { Changelists } from "../bindings/Changelists";
import { load, save } from "../lib/storage";

export type View = "changes" | "log" | "stash";

export interface Toast {
  id: number;
  kind: "error" | "info" | "success";
  title: string;
  detail?: string;
}

export interface Task {
  id: number;
  title: string;
  /** Repository the task runs in (tasks keep running when you switch tabs). */
  repo: string;
  startedAt: number;
  /** Remote tasks report git progress and can be cancelled. */
  cancellable: boolean;
  cancelling?: boolean;
  progress?: Progress;
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

/** Per-repository state kept for every open tab. */
export interface RepoTab {
  root: string;
  snapshot: RepoSnapshot;
  selectedPath: string | null;
}

interface RepoState {
  /** Open repositories, in tab order. Snapshots stay live in background tabs. */
  tabs: RepoTab[];
  /** Active tab; `snapshot`/`selectedPath` below mirror it. */
  root: string | null;
  snapshot: RepoSnapshot | null;
  view: View;
  recent: string[];

  selectedPath: string | null;
  /** Both sides of the selected file, for the diff editor. */
  diff: FileTexts | null;
  diffLoading: boolean;
  /** Optimistic include/exclude state, cleared by the next snapshot. */
  pendingStage: Record<string, boolean>;

  tasks: Task[];
  toasts: Toast[];
  branchPopup: boolean;
  /** The undo center (operation log) popup is open. */
  undoCenter: boolean;
  /** The command palette is open. */
  palette: boolean;
  /** Changelists of the active repository, once loaded. */
  changelists: { root: string; state: Changelists } | null;
  confirm: ConfirmRequest | null;
  pushDialog: boolean;
  stashDialog: boolean;
  /** Conflicted file open in the merge tool. */
  mergePath: string | null;
  /** Commit the interactive rebase dialog starts from. */
  rebaseFrom: CommitSummary | null;
  /** File open in the annotate view: at a commit, or the working copy (rev null). */
  annotate: { path: string; rev: string | null } | null;
  /** A commit the Log should select next time it shows (set by jumps from elsewhere). */
  logJump: string | null;
  /** History view: a file's history, or of lines `range` (1-based, inclusive) at `rev`. */
  history: { path: string; range?: [number, number]; rev?: string | null } | null;
  /** Branch comparison: two refs (names or commit ids) with display names. */
  compare: { left: string; right: string; leftName: string; rightName: string } | null;
  pullMode: PullMode;

  openRepo: (path: string) => Promise<void>;
  /** Close the active tab. */
  closeRepo: () => Promise<void>;
  closeTab: (root: string) => Promise<void>;
  switchTab: (root: string) => void;
  cycleTab: (step: 1 | -1) => void;
  /** Reopen the tabs from the previous session. */
  restoreTabs: () => Promise<void>;
  applySnapshot: (root: string, snapshot: RepoSnapshot) => void;
  setView: (v: View) => void;
  select: (path: string | null) => void;
  reloadDiff: () => Promise<void>;
  setIncluded: (changes: FileChange[], include: boolean) => Promise<void>;
  runTask: <T>(title: string, fn: (root: string) => Promise<T>, success?: string) => Promise<T | undefined>;
  /** Like runTask, but passes a task id for progress events and cancellation. */
  /** A task not tied to a repository (e.g. downloading an update). */
  runAppTask: <T>(title: string, fn: (taskId: number) => Promise<T>) => Promise<T | undefined>;
  runRemote: (
    title: string,
    fn: (root: string, taskId: number) => Promise<void>,
    success?: string,
  ) => Promise<boolean>;
  setTaskProgress: (taskId: number, progress: Progress) => void;
  cancelTask: (taskId: number) => void;
  setPushDialog: (open: boolean) => void;
  setStashDialog: (open: boolean) => void;
  setMergePath: (path: string | null) => void;
  setRebaseFrom: (commit: CommitSummary | null) => void;
  setAnnotate: (a: { path: string; rev: string | null } | null) => void;
  setHistory: (h: { path: string; range?: [number, number]; rev?: string | null } | null) => void;
  setCompare: (c: { left: string; right: string; leftName: string; rightName: string } | null) => void;
  /** Switch to the Log and select this commit. */
  showInLog: (oid: string) => void;
  clearLogJump: () => void;
  toast: (t: Omit<Toast, "id">) => void;
  dismissToast: (id: number) => void;
  setBranchPopup: (open: boolean) => void;
  setUndoCenter: (open: boolean) => void;
  setPalette: (open: boolean) => void;
  loadChangelists: () => Promise<void>;
  /** Change the changelists (applied at once, then saved). */
  updateChangelists: (fn: (s: Changelists) => Changelists) => void;
  ask: (req: Omit<ConfirmRequest, "resolve">) => Promise<boolean>;
  setPullMode: (m: PullMode) => void;
}

/** "推送失败", but "Cherry-pick 1a2b3c4d 失败" after Latin text. */
const failedTitle = (title: string) => (/[A-Za-z0-9]$/.test(title) ? `${title} 失败` : `${title}失败`);

let seq = 0;
let diffSeq = 0;

/** A file counts as "included" in the commit when nothing remains unstaged. */
export function isIncluded(c: FileChange): boolean | "partial" {
  if (c.conflicted) return false;
  if (c.staged && c.unstaged) return "partial";
  return c.staged != null;
}

function persistTabs(tabs: RepoTab[], active: string | null) {
  save("openTabs", tabs.map((t) => t.root));
  save("activeTab", active);
}

export const useRepo = create<RepoState>((set, get) => ({
  tabs: [],
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
  undoCenter: false,
  palette: false,
  changelists: null,
  confirm: null,
  pushDialog: false,
  stashDialog: false,
  mergePath: null,
  rebaseFrom: null,
  annotate: null,
  logJump: null,
  history: null,
  compare: null,
  pullMode: load<PullMode>("pullMode", "merge"),

  async openRepo(path) {
    try {
      const snapshot = await api.openRepo(path);
      const root = snapshot.root;
      const recent = [root, ...get().recent.filter((r) => r !== root)].slice(0, 15);
      save("recentRepos", recent);
      set((s) =>
        s.tabs.some((t) => t.root === root)
          ? { recent, tabs: s.tabs.map((t) => (t.root === root ? { ...t, snapshot } : t)) }
          : { recent, tabs: [...s.tabs, { root, snapshot, selectedPath: null }] },
      );
      get().switchTab(root);
      set({ view: "changes" });
    } catch (e) {
      get().toast({ kind: "error", title: "无法打开仓库", detail: errorMessage(e) });
    }
  },

  async closeRepo() {
    const root = get().root;
    if (root) await get().closeTab(root);
  },

  async closeTab(root) {
    const { tabs, root: active } = get();
    const i = tabs.findIndex((t) => t.root === root);
    if (i < 0) return;
    void api.closeRepo(root).catch(() => {});
    const rest = tabs.filter((t) => t.root !== root);
    set({ tabs: rest });
    if (active === root) {
      const next = rest[Math.min(i, rest.length - 1)];
      if (next) get().switchTab(next.root);
      else set({ root: null, snapshot: null, selectedPath: null, diff: null, pendingStage: {} });
    }
    persistTabs(get().tabs, get().root);
  },

  switchTab(root) {
    const { tabs, root: active, selectedPath } = get();
    const target = tabs.find((t) => t.root === root);
    if (!target) return;
    // Remember the selection of the tab we are leaving.
    const saved = tabs.map((t) => (t.root === active ? { ...t, selectedPath } : t));
    diffSeq++; // drop any in-flight diff for the old tab
    set({
      tabs: saved,
      root,
      snapshot: target.snapshot,
      selectedPath: target.selectedPath,
      diff: null,
      diffLoading: false,
      pendingStage: {},
      branchPopup: false,
      undoCenter: false,
      changelists: null,
    });
    persistTabs(saved, root);
    if (target.selectedPath) void get().reloadDiff();
  },

  cycleTab(step) {
    const { tabs, root } = get();
    if (tabs.length < 2) return;
    const i = tabs.findIndex((t) => t.root === root);
    get().switchTab(tabs[(i + step + tabs.length) % tabs.length].root);
  },

  async restoreTabs() {
    const roots = load<string[]>("openTabs", []);
    const active = load<string | null>("activeTab", null);
    // Open all in parallel; repos that moved or were deleted are skipped quietly.
    const results = await Promise.allSettled(roots.map((r) => api.openRepo(r)));
    const tabs: RepoTab[] = [];
    for (const r of results) {
      if (r.status === "fulfilled" && !tabs.some((t) => t.root === r.value.root))
        tabs.push({ root: r.value.root, snapshot: r.value, selectedPath: null });
    }
    if (tabs.length === 0 || get().tabs.length > 0) return;
    set({ tabs });
    get().switchTab(tabs.find((t) => t.root === active)?.root ?? tabs[0].root);
  },

  applySnapshot(root, snapshot) {
    set((s) => ({ tabs: s.tabs.map((t) => (t.root === root ? { ...t, snapshot } : t)) }));
    if (root !== get().root) return;
    const { selectedPath } = get();
    const stillThere = selectedPath && snapshot.changes.some((c) => c.path === selectedPath);
    set({
      snapshot,
      pendingStage: {},
      ...(stillThere ? {} : { selectedPath: null, diff: null }),
    });
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
      const diff = await api.fileTexts(root, selectedPath, change.origPath, mode, change.unstaged === "untracked");
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
    set((s) => ({ tasks: [...s.tasks, { id, title, repo: root, startedAt: Date.now(), cancellable: false }] }));
    try {
      const r = await fn(root);
      if (success) get().toast({ kind: "success", title: success });
      return r;
    } catch (e) {
      get().toast({ kind: "error", title: failedTitle(title), detail: errorMessage(e) });
      return undefined;
    } finally {
      set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) }));
    }
  },

  async runAppTask(title, fn) {
    const id = ++seq;
    set((s) => ({ tasks: [...s.tasks, { id, title, repo: "", startedAt: Date.now(), cancellable: false }] }));
    try {
      return await fn(id);
    } catch (e) {
      get().toast({ kind: "error", title: failedTitle(title), detail: errorMessage(e) });
      return undefined;
    } finally {
      set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) }));
    }
  },

  async runRemote(title, fn, success) {
    const root = get().root;
    if (!root) return false;
    const id = ++seq;
    set((s) => ({ tasks: [...s.tasks, { id, title, repo: root, startedAt: Date.now(), cancellable: true }] }));
    try {
      await fn(root, id);
      if (success) get().toast({ kind: "success", title: success });
      return true;
    } catch (e) {
      if (isCancelled(e)) get().toast({ kind: "info", title: `${title}已取消` });
      else get().toast({ kind: "error", title: failedTitle(title), detail: errorMessage(e) });
      return false;
    } finally {
      set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) }));
    }
  },

  setTaskProgress(taskId, progress) {
    set((s) => ({ tasks: s.tasks.map((t) => (t.id === taskId ? { ...t, progress } : t)) }));
  },

  cancelTask(taskId) {
    set((s) => ({ tasks: s.tasks.map((t) => (t.id === taskId ? { ...t, cancelling: true } : t)) }));
    void api.cancelTask(taskId);
  },

  setPushDialog: (pushDialog) => set({ pushDialog }),
  setStashDialog: (stashDialog) => set({ stashDialog }),
  setMergePath: (mergePath) => set({ mergePath }),
  setRebaseFrom: (rebaseFrom) => set({ rebaseFrom }),
  setAnnotate: (annotate) => set({ annotate }),
  setHistory: (history) => set({ history }),
  setCompare: (compare) => set({ compare }),
  showInLog: (oid) => set({ view: "log", logJump: oid, annotate: null, history: null, compare: null }),
  clearLogJump: () => set({ logJump: null }),

  toast(t) {
    const id = ++seq;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { ...t, id }] }));
    const ttl = t.kind === "error" ? 9000 : 3500;
    setTimeout(() => get().dismissToast(id), ttl);
  },

  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setBranchPopup: (branchPopup) => set({ branchPopup }),
  setUndoCenter: (undoCenter) => set({ undoCenter }),
  setPalette: (palette) => set({ palette }),
  async loadChangelists() {
    const root = get().root;
    if (!root) return;
    try {
      const state = await api.changelists(root);
      if (get().root === root) set({ changelists: { root, state } });
    } catch (e) {
      get().toast({ kind: "error", title: "无法读取 Changelist", detail: errorMessage(e) });
    }
  },
  updateChangelists(fn) {
    const cur = get().changelists;
    if (!cur || cur.root !== get().root) return;
    const next = fn(cur.state);
    if (next === cur.state) return;
    set({ changelists: { root: cur.root, state: next } });
    api.saveChangelists(cur.root, next).catch((e) =>
      get().toast({ kind: "error", title: "Changelist 保存失败", detail: errorMessage(e) }),
    );
  },

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

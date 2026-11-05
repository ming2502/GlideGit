// Browser preview mode: when the UI runs outside Tauri (`pnpm dev` in a normal
// browser), IPC is served from this in-memory fake repository. Dev-only.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { BlameCommit } from "../bindings/BlameCommit";
import type { Branch } from "../bindings/Branch";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { FileChange } from "../bindings/FileChange";
import type { FileDiff } from "../bindings/FileDiff";
import type { GraphRow } from "../bindings/GraphRow";
import type { LogFilter } from "../bindings/LogFilter";
import type { LogStatus } from "../bindings/LogStatus";
import type { RepoSnapshot } from "../bindings/RepoSnapshot";

const now = Math.floor(Date.now() / 1000);

function branch(name: string, extra: Partial<Branch> = {}): Branch {
  const isRemote = name.includes("/") && name.startsWith("origin/");
  return {
    name,
    fullRef: isRemote ? `refs/remotes/${name}` : `refs/heads/${name}`,
    oid: "a1b2c3d4e5f6",
    upstream: null,
    upstreamGone: false,
    ahead: 0,
    behind: 0,
    isHead: false,
    isRemote,
    ...extra,
  };
}

let changes: FileChange[] = [
  { path: "crates/core/src/session.rs", origPath: null, staged: "modified", unstaged: null, conflicted: false },
  { path: "crates/git-backend/src/repo.rs", origPath: null, staged: "modified", unstaged: "modified", conflicted: false },
  { path: "ui/src/views/CommitPanel.tsx", origPath: null, staged: null, unstaged: "modified", conflicted: false },
  { path: "ui/src/views/LogView.tsx", origPath: "ui/src/views/History.tsx", staged: "renamed", unstaged: null, conflicted: false },
  { path: "docs/old-notes.md", origPath: null, staged: null, unstaged: "deleted", conflicted: false },
  { path: "ui/src/dev/mock.ts", origPath: null, staged: "added", unstaged: null, conflicted: false },
  { path: "scratch/todo.txt", origPath: null, staged: null, unstaged: "untracked", conflicted: false },
  { path: "assets/banner.png", origPath: null, staged: null, unstaged: "untracked", conflicted: false },
];

const MAIN_REPO = "/Users/demo/GlideGit";
const DEMO_REPOS = [MAIN_REPO, "/Users/demo/website", "/Users/demo/infra"];
let pickCount = 0;

/** Operation in progress in the main demo repo (a conflicting cherry-pick). */
let operation: RepoSnapshot["operation"] = null;
function publish(): void {
  void emit("repo-snapshot", { root: MAIN_REPO, snapshot: snapshot(MAIN_REPO) });
}

function snapshot(root: string = MAIN_REPO): RepoSnapshot {
  if (root !== MAIN_REPO) {
    // Secondary demo repos: a clean-ish tree on main.
    return {
      root,
      head: { branch: "main", oid: "b0b0b0b0", detached: false, upstream: "origin/main", ahead: 0, behind: 0 },
      operation: null,
      stale: false,
      operationInfo: null,
      changes: root.endsWith("website")
        ? [{ path: "src/index.html", origPath: null, staged: null, unstaged: "modified", conflicted: false }]
        : [],
      branches: { local: [branch("main", { isHead: true, upstream: "origin/main" })], remote: [branch("origin/main")], recent: [] },
    };
  }
  return {
    root: MAIN_REPO,
    head: { branch: "feature/commit-panel", oid: "a1b2c3d4", detached: false, upstream: "origin/feature/commit-panel", ahead: 2, behind: 1 },
    operation,
    stale: false,
    operationInfo: operation
      ? { step: null, total: null, commit: "78dde6c Log filters", branch: null, onto: null, canSkip: true }
      : null,
    changes,
    branches: {
      local: [
        branch("feature/commit-panel", { isHead: true, upstream: "origin/feature/commit-panel", ahead: 2, behind: 1 }),
        branch("main", { upstream: "origin/main" }),
        branch("feature/log-index", { upstream: "origin/feature/log-index", ahead: 4 }),
        branch("fix/windows-crlf", { upstream: "origin/fix/windows-crlf", upstreamGone: true }),
      ],
      remote: [branch("origin/main"), branch("origin/feature/commit-panel"), branch("origin/feature/log-index"), branch("origin/release/0.1")],
      recent: ["main", "feature/log-index"],
    },
  };
}

const subjects = [
  "Add RepoSession with serialised writes",
  "Parse git status porcelain v2",
  "Merge branch 'feature/branch-popup'",
  "Branch popup: keyboard navigation",
  "Virtualised diff view",
  "Initial commit",
];
const log: CommitSummary[] = Array.from({ length: 240 }, (_, i) => ({
  oid: (i * 2654435761).toString(16).padStart(8, "0").repeat(5).slice(0, 40),
  parents: i === 2 ? ["p1", "p2"] : ["p1"],
  authorName: ["Min", "Alex Chen", "Sam"][i % 3],
  authorEmail: "dev@example.com",
  authorTime: now - i * 5400,
  subject: subjects[i % subjects.length],
  refs: i === 0 ? ["HEAD -> feature/commit-panel"] : i === 2 ? ["origin/feature/commit-panel"] : i === 5 ? ["origin/main", "main", "tag: v0.0.1"] : [],
}));

/** Simulated commit index: a long history that is "indexed" over a few seconds. */
/**
 * Simulated commit index: a long history that is "indexed" over a few
 * seconds. `?commits=N` in the URL changes its length (e.g. 2000000 to try
 * the scaled scrollbar).
 */
const MOCK_HISTORY = Math.max(
  1,
  Number(new URLSearchParams(location.search).get("commits")) || 120_000,
);
const authors = ["Min", "Alex Chen", "Sam", "Priya Natarajan", "Jonas Weber"];

// History repeats every 40 rows: a merge commit, a 10-commit side branch
// (rows 2–11) forked from main at row 20, and main in between.
const BLOCK = 40;
function mockParents(r: number): number[] {
  const b = r - (r % BLOCK);
  const k = r % BLOCK;
  const ps =
    k === 0 ? [b + 1, b + 2] : k === 1 ? [b + 12] : k === 11 ? [b + 20] : [r + 1];
  return ps.filter((p) => p < MOCK_HISTORY);
}
const mockOid = (r: number) => (Math.imul(r + 1, 2654435761) >>> 0).toString(16).padStart(8, "0") + r.toString(16).padStart(32, "0");

function mockCommit(i: number): CommitSummary {
  const h = Math.imul(i, 2654435761) >>> 0;
  const k = i % BLOCK;
  const refs =
    i === 0
      ? ["HEAD -> feature/commit-panel", "origin/feature/commit-panel"]
      : i === 2
        ? ["feature/log-graph"]
        : i === 12
          ? ["origin/main", "main"]
          : i % 5000 === 1
            ? [`tag: v0.${Math.floor((MOCK_HISTORY - i) / 5000)}.0`]
            : [];
  return {
    oid: mockOid(i),
    parents: mockParents(i).map(mockOid),
    authorName: authors[h % authors.length],
    authorEmail: "dev@example.com",
    authorTime: now - i * 3100,
    subject:
      k === 0
        ? `Merge branch 'feature/block-${Math.floor(i / BLOCK)}'`
        : `${subjects[h % subjects.length]} (#${MOCK_HISTORY - i})`,
    refs,
  };
}

/** TS copy of the backend layout (crates/log-index/src/graph.rs), on row numbers. */
function layoutRows(n: number): GraphRow[] {
  let lanes: number[] = [];
  const out: GraphRow[] = [];
  for (let r = 0; r < n; r++) {
    const parents = mockParents(r);
    const node = lanes.indexOf(r) < 0 ? lanes.length : lanes.indexOf(r);
    const waited = (p: number) => lanes.some((l) => l !== r && l === p);
    const next: number[] = [];
    const moved: (number | null)[] = lanes.map(() => null);
    const opened: number[] = [];
    const joins: number[] = [];
    const open = () =>
      parents.forEach((p, k) => {
        if (k > 0 && waited(p)) joins.push(p);
        else {
          opened.push(next.length);
          next.push(p);
        }
      });
    lanes.forEach((l, i) => {
      if (i === node) open();
      else if (l !== r) {
        moved[i] = next.length;
        next.push(l);
      }
    });
    if (node === lanes.length) open();
    const up: number[] = [];
    const down: number[] = [];
    lanes.forEach((l, i) => {
      up.push(i, l === r ? node : i);
      if (moved[i] != null) down.push(i, moved[i]!);
    });
    opened.forEach((j) => down.push(node, j));
    joins.forEach((p) => {
      const i = lanes.findIndex((l, i) => l === p && moved[i] != null);
      if (i >= 0) down.push(node, moved[i]!);
    });
    out.push({ lane: node, up, down });
    lanes = next;
  }
  return out;
}
// Three blocks are enough: from the second block on, the layout repeats.
const graphBlocks = layoutRows(Math.min(MOCK_HISTORY, BLOCK * 3));
const mockGraph = (r: number): GraphRow =>
  r < BLOCK * 2 || MOCK_HISTORY <= BLOCK * 3 ? graphBlocks[r] : graphBlocks[BLOCK + (r % BLOCK)];

const indexed = new Map<string, number>();
function logStatus(root: string): LogStatus {
  const total = indexed.get(root) ?? 0;
  return { root, total, complete: total >= MOCK_HISTORY, version: 1, error: null };
}
/** Grow the index like a first build does: first rows at once, the rest in batches. */
function startIndexing(root: string) {
  if (indexed.has(root)) return;
  indexed.set(root, 0);
  let total = 0;
  const step = () => {
    total = Math.min(MOCK_HISTORY, total === 0 ? 300 : total + Math.max(6000, Math.ceil(MOCK_HISTORY / 20)));
    indexed.set(root, total);
    void emit("log-status", logStatus(root));
    if (total < MOCK_HISTORY) setTimeout(step, 150);
  };
  setTimeout(step, 250);
}

/** A longer Rust-like sample file. */
function rustSample(): string {
  return Array.from({ length: 140 }, (_, i) =>
    i % 12 === 0
      ? `\n/// Handles request number ${i}.\npub fn handle_${i}(state: &State, input: &str) -> Result<usize> {`
      : i % 12 === 11
        ? "}"
        : `    let value_${i} = state.lookup("${i}").unwrap_or(${i});`,
  ).join("\n");
}

/** A sample file in two versions, for the diff editor in the demo. */
function textsFor(path: string) {
  const ext = path.split(".").pop() ?? "";
  if (path.endsWith(".png")) return { path, old: null, new: null, binary: true, tooLarge: false };
  const rust = (n: number) =>
    Array.from({ length: n }, (_, i) =>
      i % 12 === 0
        ? `\n/// Handles request number ${i}.\npub fn handle_${i}(state: &State, input: &str) -> Result<usize> {`
        : i % 12 === 11
          ? "}"
          : `    let value_${i} = state.lookup("${i}").unwrap_or(${i});`,
    ).join("\n");
  let old: string;
  if (ext === "md") old = `# GlideGit\n\nA fast Git client.\n\n## Features\n\n- Log with graph\n- Commit panel\n- Branch popup\n`;
  else if (ext === "css") old = Array.from({ length: 30 }, (_, i) => `.rule-${i} {\n  color: var(--text);\n  padding: ${i}px;\n}`).join("\n");
  else old = rust(140);
  let next = old
    .replace('value_25 = state.lookup("25").unwrap_or(25)', "value_25 = state.lookup_cached(\"25\").unwrap_or_default()")
    .replace('    let value_70 = state.lookup("70").unwrap_or(70);\n', "")
    .replace("pub fn handle_96(", "pub(crate) fn handle_96(")
    .replace("- Branch popup", "- Branch popup\n- Log filters and commit details")
    .replace("padding: 7px;", "padding: 8px 12px;");
  next = next.replace("/// Handles request number 120.", "/// Handles request number 120.\n/// Retries once when the cache is cold.");
  next = worktree.get(path) ?? next;
  return { path, old, new: next, binary: false, tooLarge: false, index: staged.get(path) ?? old, lossy: false };
}
/** Demo stash list. */
let stashSeq = 0;
const stashes = [
  { index: 0, oid: mockOid(700_001), message: "try the faster filter path", branch: "feature/log-index", time: now - 3 * 3600 },
  { index: 1, oid: mockOid(700_002), message: "a1b2c3d WIP on graph colours", branch: "main", time: now - 2 * 86400 },
  { index: 2, oid: mockOid(700_003), message: "debug logging", branch: null as string | null, time: now - 9 * 86400 },
];

/** Demo index and working-tree contents changed by block staging / discard. */
const staged = new Map<string, string>();
const worktree = new Map<string, string>();

function diffFor(path: string): FileDiff {
  const lines: FileDiff["hunks"][number]["lines"] = [];
  let o = 40;
  let n = 40;
  const src = [
    ["c", "    pub async fn write<T, F>(&self, f: F) -> Result<T>"],
    ["c", "    where"],
    ["d", "        F: FnOnce(&Repo) -> Result<T> + Send,"],
    ["a", "        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,"],
    ["a", "        T: Send + 'static,"],
    ["c", "    {"],
    ["c", "        let _guard = self.write_lock.lock().await;"],
    ["a", "        self.state.writing.fetch_add(1, Ordering::SeqCst);"],
    ["c", "        let repo = self.repo.clone();"],
  ] as const;
  for (const [k, t] of src) {
    if (k === "c") lines.push({ kind: "context", oldLine: o++, newLine: n++, text: t });
    else if (k === "d") lines.push({ kind: "deleted", oldLine: o++, newLine: null, text: t });
    else lines.push({ kind: "added", oldLine: null, newLine: n++, text: t });
  }
  return {
    path,
    binary: path.endsWith(".png"),
    truncated: false,
    added: 3,
    deleted: 1,
    hunks: [{ header: "@@ -40,7 +40,9 @@ impl RepoSession {", oldStart: 40, oldLines: 7, newStart: 40, newLines: 9, lines }],
  };
}

const cancelled = new Set<number>();
const askWaiters = new Map<number, (answer: string | null) => void>();
let askSeq = 0;

/** Simulate git asking for credentials through askpass. */
function fakeAsk(kind: string, prompt: string, target: string | null): Promise<string | null> {
  const id = ++askSeq;
  void emit("askpass-request", { id, kind, prompt, target });
  return new Promise((resolve) => askWaiters.set(id, resolve));
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Simulate a network op: emit git-like progress, honour cancellation. */
async function fakeRemote(taskId: number, phases: string[]): Promise<null> {
  for (const phase of phases) {
    for (let pct = 0; pct <= 100; pct += 4) {
      if (cancelled.has(taskId)) throw { kind: "cancelled", message: "operation cancelled" };
      await emit("task-progress", {
        taskId,
        progress: { phase, percent: pct, detail: `(${pct * 12}/1200), ${(pct * 0.05).toFixed(2)} MiB | 1.20 MiB/s` },
      });
      await sleep(70);
    }
  }
  return null;
}

const MOCK_PATHS = [
  "crates/log-index/src/lib.rs",
  "crates/log-index/src/graph.rs",
  "crates/core/src/session.rs",
  "crates/app/src/commands.rs",
  "ui/src/views/LogView.tsx",
  "ui/src/views/LogGraph.tsx",
  "ui/src/styles.css",
  "docs/DESIGN.md",
  "README.md",
  "assets/logo.png",
];
/** A few changed files per commit, chosen from the row number. */
function mockFiles(i: number) {
  const n = 1 + (i % 6);
  return Array.from({ length: n }, (_, k) => {
    const path = MOCK_PATHS[(i * 7 + k * 3) % MOCK_PATHS.length];
    const kind = k === 0 && i % 5 === 0 ? "added" : k === 1 && i % 7 === 0 ? "renamed" : k === 2 && i % 4 === 0 ? "deleted" : "modified";
    const binary = path.endsWith(".png");
    return {
      path,
      origPath: kind === "renamed" ? path.replace(/\/([^/]+)$/, "/old_$1") : null,
      kind,
      added: binary ? null : kind === "deleted" ? 0 : 3 + ((i + k) % 40),
      deleted: binary ? null : kind === "added" ? 0 : (i * k) % 17,
    };
  }).filter((f, k, all) => all.findIndex((x) => x.path === f.path) === k);
}

/** Rows matching a filter (cached per filter). Branch filters keep everything. */
const matchCache = new Map<string, number[]>();
function mockMatches(f: LogFilter): number[] {
  const key = JSON.stringify(f);
  const hit = matchCache.get(key);
  if (hit) return hit;
  const text = f.text.trim().toLowerCase();
  const out: number[] = [];
  for (let i = 0; i < MOCK_HISTORY; i++) {
    const c = mockCommit(i);
    if (text && !c.subject.toLowerCase().includes(text) && !(text.length >= 4 && c.oid.startsWith(text))) continue;
    if (f.authors.length && !f.authors.some((x) => `${c.authorName} <${c.authorEmail}>`.toLowerCase().includes(x.toLowerCase()) || c.authorName.toLowerCase().startsWith(x.split("@")[0])))
      continue;
    if (f.since != null && c.authorTime < f.since) continue;
    if (f.until != null && c.authorTime > f.until) continue;
    // Paths: pretend every third commit touches them.
    if (f.paths.length && i % 3 !== 0) continue;
    out.push(i);
  }
  matchCache.set(key, out);
  return out;
}
async function mockFiltered(root: string, f: LogFilter, start: number, count: number) {
  await sleep(120); // git would take a moment
  const rows = mockMatches(f);
  const s = Math.min(start, rows.length);
  const e = Math.min(s + count, rows.length);
  const graphOk = !f.text.trim() && !f.authors.length && f.since == null && f.until == null && !f.paths.length;
  return {
    start: s,
    total: rows.length,
    version: logStatus(root).version,
    rows: rows.slice(s, e).map(mockCommit),
    graph: graphOk ? rows.slice(s, e).map(mockGraph) : [],
  };
}

// Operation log: a few past operations, plus whatever the demo does.
type MockOp = { id: number; time: number; kind: string; title: string; undo: string | null; blocked: string | null; undone: boolean };
const t0 = Math.floor(Date.now() / 1000);
const ops: MockOp[] = [
  { id: 1, time: t0 - 7200, kind: "push", title: "推送 feature/commit-panel", undo: null, blocked: "已推送到远程，无法在本地撤销", undone: false },
  { id: 2, time: t0 - 3600, kind: "branch", title: "新建并切换到分支 feature/commit-panel", undo: "切换回 main，删除 feature/commit-panel", blocked: null, undone: false },
  { id: 3, time: t0 - 1800, kind: "cherryPick", title: "Cherry-pick 78dde6c Log filters（未完成）", undo: null, blocked: "操作在中途停下了，可在顶部横幅中中止", undone: false },
  { id: 4, time: t0 - 600, kind: "commit", title: "提交：Add RepoSession with serialised writes", undo: "撤销提交，修改保留在暂存区", blocked: null, undone: false },
  { id: 5, time: t0 - 120, kind: "rollback", title: "回滚 ui/src/App.tsx", undo: "恢复被丢弃的修改（作为未暂存的修改）", blocked: null, undone: false },
];
const OP_OF: Record<string, (a: Record<string, unknown>) => [string, string, string | null]> = {
  commit: (a) => [a.amend ? "amend" : "commit", `${a.amend ? "修改提交" : "提交"}：${String(a.message).split("\n")[0]}`, "撤销提交，修改保留在暂存区"],
  checkout_branch: (a) => ["checkout", `切换到 ${a.name}`, "切换回 feature/commit-panel"],
  create_branch: (a) => ["branch", `新建分支 ${a.name}`, `删除 ${a.name}`],
  delete_branch: (a) => ["branch", `删除分支 ${a.name}`, `恢复 ${a.name}`],
  reset: (a) => ["reset", `重置到 ${String(a.oid).slice(0, 7)}（${a.mode}）`, `回到操作前的提交 f00dbab${a.mode === "hard" ? "，恢复重置前的本地修改" : ""}`],
  rollback: (a) => ["rollback", `回滚 ${(a.tracked as string[]).length + (a.untracked as string[]).length} 个文件`, "恢复被丢弃的修改（作为未暂存的修改）"],
  write_worktree_file: (a) => ["discard", `丢弃 ${a.path} 中的修改块`, "恢复被丢弃的修改（作为未暂存的修改）"],
  stash_push: () => ["stash", "Stash 本地修改", "把修改从 stash 取回"],
  create_tag: (a) => ["tag", `新建标签 ${a.name}`, `删除 ${a.name}`],
};
function recordOp(cmd: string, a: Record<string, unknown>) {
  const f = OP_OF[cmd];
  if (!f) return;
  const [kind, title, undo] = f(a);
  ops.push({ id: ops.length + 1, time: Math.floor(Date.now() / 1000), kind, title, undo, blocked: null, undone: false });
}

let changelists = {
  lists: [
    { id: "default", name: "Changes" },
    { id: "log-filters", name: "Log filters" },
  ],
  active: "default",
  files: { "ui/src/views/LogView.tsx": "log-filters" } as Record<string, string>,
};
let shelfSeq = 1;
let shelf: { id: string; oid: string; name: string; changelist: string | null; time: number }[] = [
  { id: "1700000000000-a1b2c3d4", oid: mockOid(699_999), name: "试验：Canvas 图的抗锯齿", changelist: "Graph", time: t0 - 86400 * 2 },
];

export function installMock(): void {
  mockWindows("main");
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    recordOp(cmd, a);
    switch (cmd) {
      case "changelists":
        return structuredClone(changelists);
      case "save_changelists":
        changelists = structuredClone(a.changelists as typeof changelists);
        return structuredClone(changelists);
      case "shelf_list":
        return [...shelf];
      case "shelve": {
        const paths = a.paths as string[];
        const e = { id: `${Date.now()}-${shelfSeq}`, oid: mockOid(700_000 + shelfSeq++), name: String(a.name), changelist: (a.changelist as string | null) ?? null, time: Math.floor(Date.now() / 1000) };
        shelf.unshift(e);
        changes = changes.filter((c) => !paths.includes(c.path));
        return sleep(200).then(() => (publish(), e));
      }
      case "unshelve": {
        const i = shelf.findIndex((e) => e.id === a.id);
        if (i < 0) throw { kind: "invalid", message: "the shelf changed; refresh and try again" };
        if (!a.keep) shelf.splice(i, 1);
        changes = [{ path: "crates/log-index/src/filter.rs", origPath: null, staged: null, unstaged: "modified", conflicted: false }, ...changes.filter((c) => c.path !== "crates/log-index/src/filter.rs")];
        return sleep(200).then(() => (publish(), false));
      }
      case "drop_shelf":
        shelf = shelf.filter((e) => e.id !== a.id);
        return sleep(100).then(() => null);
      case "operation_log":
        return [...ops].reverse();
      case "undo_operation": {
        const op = ops.find((o) => o.id === a.id);
        if (!op || op.undone || !op.undo) throw { kind: "invalid", message: "这个操作已经撤销过了" };
        if (op.kind === "branch" && op.id === 2)
          return sleep(200).then(() => {
            throw { kind: "invalid", message: "仓库在这次操作之后又有变化，无法安全撤销" };
          });
        return sleep(250).then(() => {
          op.undone = true;
          ops.push({ id: ops.length + 1, time: Math.floor(Date.now() / 1000), kind: "undo", title: `撤销：${op.title}`, undo: null, blocked: "撤销操作本身不能再撤销", undone: false });
          publish();
          return op.undo;
        });
      }
      case "app_info":
        return { version: "0.1.0", gitVersion: "2.55.0 (mock)", gitPath: "/usr/local/bin/git", gitBundled: false, platform: "macos" };
      case "fetch":
        return (async () => {
          // Demo: the first fetch walks through the HTTPS login prompts.
          if (askSeq === 0) {
            const user = await fakeAsk("username", "Username for 'https://github.com': ", "https://github.com");
            if (user == null) throw { kind: "gitFailed", message: "fatal: could not read Username for 'https://github.com': terminal prompts disabled" };
            const pass = await fakeAsk("password", `Password for 'https://${user}@github.com': `, `https://${user}@github.com`);
            if (pass == null) throw { kind: "gitFailed", message: "fatal: could not read Password" };
          }
          return fakeRemote(a.taskId as number, ["Counting objects", "Receiving objects", "Resolving deltas"]);
        })();
      case "askpass_respond":
        askWaiters.get(a.id as number)?.(a.answer as string | null);
        askWaiters.delete(a.id as number);
        return true;
      case "pull":
        return fakeRemote(a.taskId as number, ["Receiving objects", "Updating files"]);
      case "push":
        return fakeRemote(a.taskId as number, ["Compressing objects", "Writing objects"]);
      case "cancel_task":
        cancelled.add(a.taskId as number);
        return true;
      case "push_preview":
        return {
          branch: "feature/commit-panel",
          remote: "origin",
          target: "origin/feature/commit-panel",
          newBranch: false,
          commits: log.slice(0, 3),
          behind: 1,
        };
      case "plugin:dialog|open":
        // Each "Open…" in the demo picks the next sample repository.
        return DEMO_REPOS[pickCount++ % DEMO_REPOS.length];
      case "open_repo": {
        // Like the app: the cached snapshot first, the fresh one a moment later.
        const root = a.path as string;
        setTimeout(() => void emit("repo-snapshot", { root, snapshot: snapshot(root) }), 600);
        return { ...snapshot(root), stale: true };
      }
      case "refresh":
        return snapshot(a.root as string);
      case "stage":
      case "unstage": {
        const paths = a.paths as string[];
        changes = changes.map((c) =>
          !paths.includes(c.path)
            ? c
            : cmd === "stage"
              ? { ...c, staged: c.unstaged === "untracked" ? "added" : (c.unstaged ?? c.staged), unstaged: null, conflicted: false }
              : { ...c, unstaged: c.staged === "added" ? "untracked" : (c.staged ?? c.unstaged), staged: null },
        );
        return null;
      }
      case "diff_file":
        return diffFor(a.path as string);
      case "log_status":
        startIndexing(a.root as string);
        return logStatus(a.root as string);
      case "commit_details": {
        const i = parseInt(String(a.oid).slice(8), 16) || 0;
        const c = mockCommit(i);
        const files = mockFiles(i);
        return {
          oid: c.oid,
          parents: c.parents,
          authorName: c.authorName,
          authorEmail: c.authorEmail,
          authorTime: c.authorTime,
          committerName: i % 4 === 0 ? "GitHub" : c.authorName,
          committerEmail: i % 4 === 0 ? "noreply@github.com" : c.authorEmail,
          committerTime: c.authorTime + (i % 4 === 0 ? 7200 : 0),
          message:
            i % 3 === 0
              ? `${c.subject}\n\nKeep the log index in sync with refs after every refresh.\nIncremental updates prepend new commits and drop unreachable ones.\n\nCloses #${1200 + (i % 300)}`
              : c.subject,
          files,
        };
      }
      case "checkout_detached":
      case "create_tag":
      case "revert":
      case "reset":
        return sleep(200).then(() => null);
      case "cherry_pick":
        // Demo: cherry-picks stop on a conflict.
        return sleep(250).then(() => {
          operation = "cherryPick";
          changes = [
            { path: "crates/log-index/src/lib.rs", origPath: null, staged: null, unstaged: "modified", conflicted: true },
            ...changes,
          ];
          publish();
          throw {
            kind: "gitFailed",
            message: `error: could not apply ${String(a.oid).slice(0, 7)}... Log filters\nCONFLICT (content): Merge conflict in crates/log-index/src/lib.rs`,
          };
        });
      case "abort_operation":
      case "skip_operation":
      case "continue_operation":
        return sleep(200).then(() => {
          operation = null;
          changes = changes.filter((c) => !c.conflicted);
          publish();
          return null;
        });
      case "compare_branches":
        return sleep(150).then(() => ({
          leftOid: mockOid(0),
          rightOid: mockOid(12),
          mergeBase: mockOid(20),
          onlyLeft: Array.from({ length: 12 }, (_, k) => mockCommit(k)),
          onlyRight: [12, 13, 14].map((k) => mockCommit(k + 300)),
          onlyLeftCount: 12,
          onlyRightCount: 3,
        }));
      case "diff_files":
        return sleep(100).then(() => mockFiles(7).concat(mockFiles(11)).filter((f, k, all) => all.findIndex((x) => x.path === f.path) === k));
      case "file_history": {
        const path = a.path as string;
        const all = Array.from({ length: 23 }, (_, k) => {
          const c = mockCommit(30 + k * 17);
          const renamed = k === 15;
          return {
            commit: c,
            path: k > 15 ? path.replace(/([^/]+)$/, "old_$1") : path,
            origPath: renamed ? path.replace(/([^/]+)$/, "old_$1") : null,
            kind: k === 22 ? "added" : renamed ? "renamed" : "modified",
          };
        });
        return sleep(120).then(() => all.slice(a.skip as number, (a.skip as number) + (a.limit as number)));
      }
      case "line_history":
        return sleep(120).then(() => [0, 4, 9, 22].map((k) => mockCommit(30 + k * 17)));
      case "blame": {
        const text = rustSample();
        const n = text.split("\n").length;
        const people = [
          ["Min", 2 * 3600, "Add RepoSession with serialised writes"],
          ["Alex Chen", 5 * 86400, "Parse git status porcelain v2"],
          ["Sam", 40 * 86400, "Initial commit"],
          ["Priya Natarajan", 200 * 86400, "Branch popup: keyboard navigation"],
        ] as const;
        const commits: BlameCommit[] = people.map(([name, age, summary], k) => ({
          oid: mockOid(500 + k),
          authorName: name,
          authorEmail: `${name.split(" ")[0].toLowerCase()}@example.com`,
          authorTime: now - age,
          summary,
          previousOid: k < 3 ? mockOid(501 + k) : null,
          previousPath: k < 3 ? (a.path as string) : null,
        }));
        commits.push({ oid: "0".repeat(40), authorName: "Not Committed Yet", authorEmail: "", authorTime: now, summary: "", previousOid: null, previousPath: null });
        // Runs of a few lines each, newest edits scattered through older code.
        const lines = Array.from({ length: n }, (_, i) => (i > 40 && i < 44 ? 4 : Math.floor(i / 7) % 9 === 0 ? 0 : (Math.floor(i / 5) % 3) + 1));
        return sleep(150).then(() => ({ path: a.path, text, lines, commits }));
      }
      case "rebase_plan": {
        const i = parseInt(String(a.oid).slice(8), 16) || 0;
        if (i > 12) throw { kind: "invalid", message: "the range contains merge commits; interactive rebase would flatten them" };
        // Oldest first, from the chosen commit up to HEAD.
        return Array.from({ length: i + 1 }, (_, k) => ({ ...mockCommit(i - k), parents: [mockOid(i - k + 1)] }));
      }
      case "interactive_rebase":
        return sleep(400).then(() => (publish(), !(a.steps as { action: string }[]).some((s) => s.action === "edit")));
      case "conflict_texts": {
        const base = rustSample();
        const ours = base
          .replace("fn handle_24(", "fn handle_24_cached(")
          .replace("    let value_40 = state.lookup(\"40\").unwrap_or(40);", "    let value_40 = state.lookup_fast(\"40\").unwrap_or(40);")
          .replace("/// Handles request number 96.", "/// Handles request number 96 (ours).");
        const theirs = base
          .replace("    let value_40 = state.lookup(\"40\").unwrap_or(40);", "    let value_40 = state.lookup(\"40\").expect(\"value 40\");")
          .replace("    let value_70 = state.lookup(\"70\").unwrap_or(70);\n", "")
          .replace("/// Handles request number 120.", "/// Handles request number 120.\n/// Retries once.");
        return {
          path: a.path,
          base,
          ours,
          theirs,
          oursLabel: "feature/commit-panel",
          theirsLabel: "78dde6c Log filters",
          binary: false,
          tooLarge: false,
          lossy: false,
        };
      }
      case "resolve_conflict":
      case "resolve_with_side":
        // Dev aid: lets the demo (and manual checks) see what was saved.
        (window as unknown as { __lastResolve: unknown }).__lastResolve = a.content ?? a.side;
        changes = changes.map((c) => (c.path === a.path ? { ...c, conflicted: false, unstaged: null, staged: "modified" } : c));
        return sleep(150).then(() => (publish(), null));
      case "stash_list":
        return stashes.map((e, index) => ({ ...e, index }));
      case "stash_push": {
        const o = a.options as { message: string | null; includeUntracked: boolean };
        stashes.unshift({
          index: 0,
          oid: mockOid(900_000 + stashSeq++),
          message: o.message ?? "f00dbab WIP: commit panel tweaks",
          branch: "feature/commit-panel",
          time: Math.floor(Date.now() / 1000),
        });
        changes = changes.filter((c) => o.includeUntracked ? false : c.unstaged === "untracked");
        return sleep(200).then(() => (publish(), null));
      }
      case "stash_apply":
      case "stash_drop": {
        const i = stashes.findIndex((e) => e.oid === a.oid);
        if (i !== a.index) throw { kind: "invalid", message: "the stash list changed; refresh and try again" };
        if (cmd === "stash_drop" || a.pop) stashes.splice(i, 1);
        if (cmd === "stash_apply")
          changes = [{ path: "crates/log-index/src/filter.rs", origPath: null, staged: null, unstaged: "modified", conflicted: false }, ...changes];
        return sleep(200).then(() => (publish(), null));
      }
      case "stash_details": {
        const s = stashes.find((e) => e.oid === a.oid);
        return {
          oid: a.oid,
          parents: [mockOid(3), mockOid(800_000), mockOid(800_001)],
          authorName: "Min",
          authorEmail: "dev@example.com",
          authorTime: s?.time ?? now,
          committerName: "Min",
          committerEmail: "dev@example.com",
          committerTime: s?.time ?? now,
          message: `On ${s?.branch ?? "main"}: ${s?.message ?? ""}`,
          files: [
            { path: "crates/log-index/src/filter.rs", origPath: null, kind: "modified", added: 12, deleted: 3 },
            { path: "ui/src/views/LogFilterBar.tsx", origPath: null, kind: "modified", added: 5, deleted: 1 },
            { path: "notes/todo.md", origPath: null, kind: "untracked", added: null, deleted: null },
          ],
        };
      }
      case "stage_content":
        staged.set(a.path as string, a.content as string);
        return sleep(60).then(() => {
          changes = changes.map((c) => (c.path === a.path ? { ...c, staged: "modified" } : c));
          publish();
          return null;
        });
      case "write_worktree_file":
        worktree.set(a.path as string, a.content as string);
        return sleep(60).then(() => {
          publish();
          return null;
        });
      case "file_texts":
      case "commit_file_texts":
        return sleep(40).then(() => textsFor(a.path as string));
      case "commit_file_diff":
        return diffFor(a.path as string);
      case "log_authors": {
        const q = String(a.query ?? "").toLowerCase();
        return authors
          .map((name, k) => ({ name, email: `${name.split(" ")[0].toLowerCase()}@example.com`, commits: 24_000 - k * 3100 }))
          .filter((x) => `${x.name} <${x.email}>`.toLowerCase().includes(q));
      }
      case "log_page": {
        const filter = a.filter as LogFilter | null;
        if (filter) return mockFiltered(a.root as string, filter, a.start as number, a.count as number);
        const st = logStatus(a.root as string);
        const start = Math.min(a.start as number, st.total);
        const end = Math.min(start + (a.count as number), st.total);
        const rows = Array.from({ length: end - start }, (_, k) => mockCommit(start + k));
        const graph = rows.map((_, k) => mockGraph(start + k));
        return { start, total: st.total, version: st.version, rows, graph };
      }
      case "log_row_of": {
        if (a.filter) {
          const rows = mockMatches(a.filter as LogFilter);
          const i = rows.findIndex((r) => mockOid(r) === a.oid);
          return i < 0 ? null : i;
        }
        const i = parseInt(String(a.oid).slice(8), 16);
        return Number.isFinite(i) && mockOid(i) === a.oid ? i : null;
      }
      case "last_commit_message":
        return "Add RepoSession with serialised writes";
      case "commit":
        changes = changes.filter((c) => !c.staged);
        return "f00dbabe".repeat(5);
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}

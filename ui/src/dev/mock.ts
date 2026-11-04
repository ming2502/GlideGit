// Browser preview mode: when the UI runs outside Tauri (`pnpm dev` in a normal
// browser), IPC is served from this in-memory fake repository. Dev-only.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Branch } from "../bindings/Branch";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { FileChange } from "../bindings/FileChange";
import type { FileDiff } from "../bindings/FileDiff";
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
    commitTime: now - 3600,
    subject: "subject",
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

function snapshot(root: string = MAIN_REPO): RepoSnapshot {
  if (root !== MAIN_REPO) {
    // Secondary demo repos: a clean-ish tree on main.
    return {
      root,
      head: { branch: "main", oid: "b0b0b0b0", detached: false, upstream: "origin/main", ahead: 0, behind: 0 },
      operation: null,
      changes: root.endsWith("website")
        ? [{ path: "src/index.html", origPath: null, staged: null, unstaged: "modified", conflicted: false }]
        : [],
      branches: { local: [branch("main", { isHead: true, upstream: "origin/main" })], remote: [branch("origin/main")], recent: [] },
    };
  }
  return {
    root: MAIN_REPO,
    head: { branch: "feature/commit-panel", oid: "a1b2c3d4", detached: false, upstream: "origin/feature/commit-panel", ahead: 2, behind: 1 },
    operation: null,
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

export function installMock(): void {
  mockWindows("main");
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "app_info":
        return { version: "0.1.0", gitVersion: "2.55.0 (mock)", platform: "macos" };
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
      case "open_repo":
        return snapshot(a.path as string);
      case "refresh":
        return snapshot(a.root as string);
      case "stage":
      case "unstage": {
        const paths = a.paths as string[];
        changes = changes.map((c) =>
          !paths.includes(c.path)
            ? c
            : cmd === "stage"
              ? { ...c, staged: c.unstaged === "untracked" ? "added" : (c.unstaged ?? c.staged), unstaged: null }
              : { ...c, unstaged: c.staged === "added" ? "untracked" : (c.staged ?? c.unstaged), staged: null },
        );
        return null;
      }
      case "diff_file":
        return diffFor(a.path as string);
      case "log":
        return log;
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

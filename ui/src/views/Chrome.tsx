// Toolbar (top) and status bar (bottom).
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { PullMode } from "../api";
import { shortcutOf, runAction } from "../actions/registry";
import { ContextMenu } from "../components/ContextMenu";
import { BranchIcon, ChevronDown, FetchIcon, FolderIcon, PullIcon, PushIcon, RefreshIcon } from "../components/Icons";
import { repoName } from "../lib/format";
import { useRepo } from "../stores/repo";

export async function pickRepository(): Promise<void> {
  const dir = await open({ directory: true, multiple: false, title: "打开 Git 仓库" });
  if (typeof dir === "string") await useRepo.getState().openRepo(dir);
}

const PULL_LABEL: Record<PullMode, string> = {
  merge: "Pull (Merge)",
  rebase: "Pull (Rebase)",
  ffOnly: "Pull (Fast-forward only)",
};

export function Toolbar() {
  const root = useRepo((s) => s.root)!;
  const snapshot = useRepo((s) => s.snapshot)!;
  const view = useRepo((s) => s.view);
  const setView = useRepo((s) => s.setView);
  const recent = useRepo((s) => s.recent);
  const pullMode = useRepo((s) => s.pullMode);
  const setPullMode = useRepo((s) => s.setPullMode);
  const [menu, setMenu] = useState<null | { kind: "repo" | "pull"; x: number; y: number }>(null);
  const head = snapshot.head;

  return (
    <div className="toolbar" data-tauri-drag-region>
      <button
        className="tb-btn tb-repo"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ kind: "repo", x: r.left, y: r.bottom + 4 });
        }}
      >
        <FolderIcon />
        <b>{repoName(root)}</b>
        <ChevronDown />
      </button>
      <button className="tb-btn" onClick={() => runAction("branches.popup")} title={`分支 ${shortcutOf("branches.popup")}`}>
        <BranchIcon />
        <span>{head.branch ?? (head.oid ? head.oid.slice(0, 8) : "—")}</span>
        <ChevronDown />
      </button>
      <div className="tabs">
        <button className={`tab ${view === "changes" ? "on" : ""}`} onClick={() => setView("changes")}>
          Commit <span className="kbd">{shortcutOf("view.changes")}</span>
        </button>
        <button className={`tab ${view === "log" ? "on" : ""}`} onClick={() => setView("log")}>
          Log <span className="kbd">{shortcutOf("view.log")}</span>
        </button>
      </div>
      <span className="spacer" data-tauri-drag-region />
      <button className="tb-icon" onClick={() => runAction("git.fetch")} title="Fetch">
        <FetchIcon />
      </button>
      <span className="split-btn">
        <button className="tb-icon" onClick={() => runAction("git.pull")} title={`${PULL_LABEL[pullMode]} ${shortcutOf("git.pull")}`}>
          <PullIcon />
        </button>
        <button
          className="tb-caret"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ kind: "pull", x: r.left - 160, y: r.bottom + 4 });
          }}
          aria-label="pull options"
        >
          <ChevronDown />
        </button>
      </span>
      <button className="tb-icon" onClick={() => runAction("git.push")} title={`Push ${shortcutOf("git.push")}`}>
        <PushIcon />
        {head.ahead > 0 && <span className="badge">{head.ahead}</span>}
      </button>
      <button className="tb-icon" onClick={() => runAction("repo.refresh")} title={`刷新 ${shortcutOf("repo.refresh")}`}>
        <RefreshIcon />
      </button>
      {menu?.kind === "repo" && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "打开仓库…", shortcut: shortcutOf("repo.open"), onClick: () => void pickRepository() },
            { label: "关闭仓库", onClick: () => void useRepo.getState().closeRepo() },
            ...(recent.filter((r) => r !== root).length ? (["separator"] as const) : []),
            ...recent
              .filter((r) => r !== root)
              .slice(0, 8)
              .map((r) => ({ label: repoName(r), shortcut: r, onClick: () => void useRepo.getState().openRepo(r) })),
          ]}
        />
      )}
      {menu?.kind === "pull" && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={(Object.keys(PULL_LABEL) as PullMode[]).map((m) => ({
            label: `${m === pullMode ? "✓ " : "   "}${PULL_LABEL[m]}`,
            onClick: () => {
              setPullMode(m);
              runAction("git.pull");
            },
          }))}
        />
      )}
    </div>
  );
}

export function StatusBar({ gitVersion }: { gitVersion: string }) {
  const snapshot = useRepo((s) => s.snapshot);
  const tasks = useRepo((s) => s.tasks);
  const head = snapshot?.head;
  return (
    <div className="statusbar">
      {tasks.length > 0 ? (
        <span className="sb-task">
          <span className="spinner" /> {tasks[tasks.length - 1].title}…
          {tasks.length > 1 && <span className="muted"> (+{tasks.length - 1})</span>}
        </span>
      ) : (
        <span className="muted">{snapshot ? `${snapshot.changes.length} 个变更` : ""}</span>
      )}
      <span className="spacer" />
      <span className="muted">git {gitVersion}</span>
      {head && (
        <button className="sb-branch" onClick={() => runAction("branches.popup")}>
          <BranchIcon />
          {head.branch ?? `detached ${head.oid?.slice(0, 8) ?? ""}`}
          {head.ahead > 0 && <span className="sb-ab">↑{head.ahead}</span>}
          {head.behind > 0 && <span className="sb-ab">↓{head.behind}</span>}
        </button>
      )}
    </div>
  );
}

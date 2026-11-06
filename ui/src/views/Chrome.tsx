// Toolbar (top) and status bar (bottom).
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppInfo, PullMode } from "../api";
import { shortcutOf, runAction } from "../actions/registry";
import { ContextMenu } from "../components/ContextMenu";
import { BranchIcon, ChevronDown, FetchIcon, FolderIcon, PullIcon, PushIcon, RefreshIcon, SearchIcon, UndoIcon } from "../components/Icons";
import { repoName } from "../lib/format";
import { useRepo, type Task } from "../stores/repo";
import { useOperationLog } from "./UndoCenter";

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
  const lastUndoable = useOperationLog().find((o) => o.undo && !o.undone);

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
        <button className={`tab ${view === "stash" ? "on" : ""}`} onClick={() => setView("stash")}>
          Stash / Shelf <span className="kbd">{shortcutOf("view.stash")}</span>
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
      <button className="tb-icon" onClick={() => runAction("git.push")} title={`Push… ${shortcutOf("git.push")}`}>
        <PushIcon />
        {head.ahead > 0 && <span className="badge">{head.ahead}</span>}
      </button>
      <button className="tb-icon" onClick={() => runAction("palette.open")} title={`查找操作 ${shortcutOf("palette.open")}（或双击 Shift）`}>
        <SearchIcon />
      </button>
      <button
        className="tb-icon"
        onClick={() => runAction("git.undoCenter")}
        title={`撤销中心 ${shortcutOf("git.undoCenter")}${lastUndoable ? `\n最近可撤销：${lastUndoable.title}` : ""}`}
      >
        <UndoIcon />
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
            { label: "关闭当前标签", shortcut: shortcutOf("tabs.close"), onClick: () => void useRepo.getState().closeRepo() },
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

function TaskProgress({ task }: { task: Task }) {
  const cancel = useRepo((s) => s.cancelTask);
  const multi = useRepo((s) => s.tabs.length > 1);
  const p = task.progress;
  const pct = p?.percent ?? null;
  return (
    <span className="sb-task">
      <span className="spinner" />
      <span className="sb-task-title">
        {multi && task.repo && <span className="muted">{repoName(task.repo)} · </span>}
        {task.title}
      </span>
      {p && (
        <span className="muted sb-task-phase">
          {p.phase}
          {pct != null ? ` ${pct}%` : p.detail ? ` ${p.detail}` : ""}
        </span>
      )}
      {pct != null && (
        <span className="progress" aria-hidden>
          <span className="progress-fill" style={{ width: `${pct}%` }} />
        </span>
      )}
      {task.cancellable && (
        <button
          className="sb-cancel"
          disabled={task.cancelling}
          title={task.cancelling ? "正在取消…" : "取消"}
          onClick={() => cancel(task.id)}
        >
          {task.cancelling ? "取消中…" : "✕"}
        </button>
      )}
    </span>
  );
}

export function StatusBar({ info }: { info: AppInfo | null }) {
  const snapshot = useRepo((s) => s.snapshot);
  const tasks = useRepo((s) => s.tasks);
  const [showAll, setShowAll] = useState(false);
  const head = snapshot?.head;
  const latest = tasks[tasks.length - 1];
  return (
    <div className="statusbar">
      {latest ? (
        <>
          <TaskProgress task={latest} />
          {tasks.length > 1 && (
            <button className="sb-more" onClick={() => setShowAll((v) => !v)}>
              +{tasks.length - 1} 个任务
            </button>
          )}
          {showAll && tasks.length > 1 && (
            <div className="task-popover" onMouseLeave={() => setShowAll(false)}>
              {tasks.map((t) => (
                <div key={t.id} className="task-popover-row">
                  <TaskProgress task={t} />
                </div>
              ))}
            </div>
          )}
        </>
      ) : (
        <span className="muted">
          {snapshot ? `${snapshot.changes.length} 个变更` : ""}
          {/* Opened from the cache; the real status is on its way. */}
          {snapshot?.stale && (
            <span className="sb-stale" title="显示的是上次保存的状态，正在后台刷新">
              <span className="spinner inline" />
              正在刷新…
            </span>
          )}
        </span>
      )}
      <span className="spacer" />
      <span
        className="muted"
        title={info ? `${info.gitPath}${info.gitBundled ? "\n没有找到已安装的 git，正在使用 GlideGit 自带的 MinGit" : ""}` : undefined}
      >
        git {info?.gitVersion ?? ""}
        {info?.gitBundled ? "（内置）" : ""}
      </span>
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

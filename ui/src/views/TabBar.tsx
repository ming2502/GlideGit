// Repository tabs. Every tab keeps a live session; switching is instant.
import { useMemo, useState } from "react";
import { shortcutOf } from "../actions/registry";
import { ContextMenu } from "../components/ContextMenu";
import { repoName } from "../lib/format";
import { useRepo, type RepoTab } from "../stores/repo";
import { pickRepository } from "./Chrome";

export function TabBar() {
  const tabs = useRepo((s) => s.tabs);
  const active = useRepo((s) => s.root);
  const tasks = useRepo((s) => s.tasks);
  const busyRepos = useMemo(() => new Set(tasks.map((t) => t.repo)), [tasks]);
  const switchTab = useRepo((s) => s.switchTab);
  const closeTab = useRepo((s) => s.closeTab);
  const [menu, setMenu] = useState<{ x: number; y: number; tab: RepoTab } | null>(null);

  // Two repos with the same folder name get their parent folder as a hint.
  const label = (root: string) => {
    const name = repoName(root);
    const clash = tabs.filter((t) => repoName(t.root) === name).length > 1;
    if (!clash) return name;
    const parts = root.replace(/\\/g, "/").split("/").filter(Boolean);
    return `${parts[parts.length - 2] ?? ""}/${name}`;
  };

  return (
    <div className="tabbar" role="tablist" data-tauri-drag-region>
      {tabs.map((t) => {
        const head = t.snapshot.head;
        const changes = t.snapshot.changes.length;
        return (
          <div
            key={t.root}
            role="tab"
            aria-selected={t.root === active}
            className={`repo-tab ${t.root === active ? "on" : ""}`}
            title={t.root}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                void closeTab(t.root);
              } else if (e.button === 0) switchTab(t.root);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu({ x: e.clientX, y: e.clientY, tab: t });
            }}
          >
            {busyRepos.has(t.root) && <span className="spinner tab-spinner" />}
            <span className="tab-name">{label(t.root)}</span>
            <span className="tab-branch">{head.branch ?? head.oid?.slice(0, 7) ?? ""}</span>
            {changes > 0 && <span className="tab-count">{changes}</span>}
            <button
              className="tab-close"
              aria-label={`关闭 ${repoName(t.root)}`}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => void closeTab(t.root)}
            >
              ✕
            </button>
          </div>
        );
      })}
      <button className="tab-add" title={`打开仓库 ${shortcutOf("repo.open")}`} onClick={() => void pickRepository()}>
        ＋
      </button>
      <span className="spacer" data-tauri-drag-region />
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "关闭", onClick: () => void closeTab(menu.tab.root) },
            {
              label: "关闭其他标签",
              disabled: tabs.length < 2,
              onClick: () => {
                for (const t of tabs) if (t.root !== menu.tab.root) void closeTab(t.root);
                switchTab(menu.tab.root);
              },
            },
            "separator",
            { label: "复制路径", onClick: () => void navigator.clipboard.writeText(menu.tab.root) },
          ]}
        />
      )}
    </div>
  );
}

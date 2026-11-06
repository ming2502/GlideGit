// Repository tabs. Every tab keeps a live session; switching is instant.
import { useMemo, useState } from "react";
import { shortcutOf } from "../actions/registry";
import { ContextMenu } from "../components/ContextMenu";
import { repoName } from "../lib/format";
import { useRepo, type RepoTab } from "../stores/repo";
import { pickRepository } from "./Chrome";
import { t } from "../i18n";

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
      {tabs.map((tab) => {
        const head = tab.snapshot.head;
        const changes = tab.snapshot.changes.length;
        return (
          <div
            key={tab.root}
            role="tab"
            aria-selected={tab.root === active}
            className={`repo-tab ${tab.root === active ? "on" : ""}`}
            title={tab.root}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                void closeTab(tab.root);
              } else if (e.button === 0) switchTab(tab.root);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu({ x: e.clientX, y: e.clientY, tab });
            }}
          >
            {busyRepos.has(tab.root) && <span className="spinner tab-spinner" />}
            <span className="tab-name">{label(tab.root)}</span>
            <span className="tab-branch">{head.branch ?? head.oid?.slice(0, 7) ?? ""}</span>
            {changes > 0 && <span className="tab-count">{changes}</span>}
            <button
              className="tab-close"
              aria-label={t("关闭 {repoName}", { repoName: repoName(tab.root) })}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => void closeTab(tab.root)}
            >
              ✕
            </button>
          </div>
        );
      })}
      <button className="tab-add" title={t("打开仓库 {shortcutOf}", { shortcutOf: shortcutOf("repo.open") })} onClick={() => void pickRepository()}>
        {t("＋")}
      </button>
      <span className="spacer" data-tauri-drag-region />
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: t("关闭"), onClick: () => void closeTab(menu.tab.root) },
            {
              label: t("关闭其他标签"),
              disabled: tabs.length < 2,
              onClick: () => {
                for (const t of tabs) if (t.root !== menu.tab.root) void closeTab(t.root);
                switchTab(menu.tab.root);
              },
            },
            "separator",
            { label: t("复制路径"), onClick: () => void navigator.clipboard.writeText(menu.tab.root) },
          ]}
        />
      )}
    </div>
  );
}

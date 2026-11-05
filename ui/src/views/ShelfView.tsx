// Shelf: changes set aside with "Shelve Changes" (kept by GlideGit under
// refs/glidegit/shelf, independent of git's stash). Unshelving puts the
// files back into the changelist they came from.
import { useCallback, useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import { api } from "../api";
import type { ShelfEntry } from "../bindings/ShelfEntry";
import { ContextMenu, type MenuItem } from "../components/ContextMenu";
import { addList, findByName, moveFiles } from "../lib/changelists";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { CommitDetailsPane } from "./CommitDetails";

export function ShelfPanel({ tabs }: { tabs: ReactNode }) {
  const root = useRepo((s) => s.root)!;
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const [list, setList] = useState<ShelfEntry[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; e: ShelfEntry } | null>(null);

  const reload = useCallback(() => {
    api
      .shelfList(root)
      .then((l) => {
        setList(l);
        setSelected((cur) => (cur && l.some((e) => e.id === cur) ? cur : (l[0]?.id ?? null)));
      })
      .catch(() => setList([]));
  }, [root]);
  useEffect(reload, [reload, snapshot]);

  const entry = list?.find((e) => e.id === selected) ?? null;

  const unshelve = async (e: ShelfEntry, keep: boolean) => {
    // The files, to put back into their changelist afterwards.
    const files = await api
      .commitDetails(root, e.oid)
      .then((d) => d.files.map((f) => f.path))
      .catch(() => [] as string[]);
    const conflicts = await runTask(`取消搁置 ${e.name}`, (r) => api.unshelve(r, e.id, e.oid, e.name, keep));
    if (conflicts === undefined) return;
    const st = useRepo.getState();
    if (e.changelist && files.length) {
      if (!st.changelists) await st.loadChangelists();
      st.updateChangelists((cl) => {
        const existing = findByName(cl, e.changelist!);
        const [next, id] = existing ? [cl, existing.id] : addList(cl, e.changelist!);
        return moveFiles(next, files, id);
      });
    }
    if (conflicts)
      st.toast({
        kind: "info",
        title: "取消搁置时有冲突",
        detail: "Shelf 中的修改与当前代码冲突，已保留这条搁置记录。请在 Changes 视图中解决冲突。",
      });
    else st.toast({ kind: "success", title: `已取消搁置：${e.name}`, detail: e.changelist ? `文件已放回 Changelist「${e.changelist}」` : undefined });
    reload();
  };

  const drop = async (e: ShelfEntry) => {
    const ok = await ask({
      title: `删除搁置「${e.name}」？`,
      message: "其中保存的修改将被删除（可在撤销中心恢复）。",
      confirmLabel: "删除",
      danger: true,
    });
    if (ok) void runTask(`删除搁置 ${e.name}`, (r) => api.dropShelf(r, e.id, e.oid, e.name), `已删除：${e.name}`).then(reload);
  };

  const items = (e: ShelfEntry): (MenuItem | "separator")[] => [
    { label: "取消搁置（Unshelve）", onClick: () => void unshelve(e, false) },
    { label: "应用并保留在 Shelf", onClick: () => void unshelve(e, true) },
    "separator",
    { label: "删除…", danger: true, onClick: () => void drop(e) },
  ];

  const onKey = (ev: KeyboardEvent<HTMLDivElement>) => {
    if (!list?.length) return;
    const i = list.findIndex((e) => e.id === selected);
    if (ev.key === "ArrowDown") setSelected(list[Math.min(list.length - 1, i + 1)].id);
    else if (ev.key === "ArrowUp") setSelected(list[Math.max(0, i - 1)].id);
    else if (ev.key === "Enter" && entry) void unshelve(entry, false);
    else if ((ev.key === "Delete" || ev.key === "Backspace") && entry) void drop(entry);
    else return;
    ev.preventDefault();
  };

  return (
    <div className="stash-view">
      <div className="stash-side">
        <div className="panel-header">
          {tabs}
          <span className="muted">{list ? `${list.length} 个` : ""}</span>
        </div>
        {list === null ? (
          <div className="empty">
            <span className="spinner inline" /> 正在读取…
          </div>
        ) : list.length === 0 ? (
          <div className="empty">
            Shelf 是空的
            <div className="muted">在 Changes 中右键文件或 Changelist，选择「搁置」，把修改暂时放到一边</div>
          </div>
        ) : (
          <div className="stash-list" tabIndex={0} onKeyDown={onKey}>
            {list.map((e) => (
              <div
                key={e.id}
                className={`stash-row ${e.id === selected ? "selected" : ""}`}
                onMouseDown={() => setSelected(e.id)}
                onDoubleClick={() => void unshelve(e, false)}
                onContextMenu={(ev) => {
                  ev.preventDefault();
                  setSelected(e.id);
                  setMenu({ x: ev.clientX, y: ev.clientY, e });
                }}
              >
                <div className="stash-msg">{e.name}</div>
                <div className="stash-meta muted">
                  {e.changelist && <span className="ref ref-local">{e.changelist}</span>}
                  <span>{relativeTime(e.time)}</span>
                </div>
              </div>
            ))}
          </div>
        )}
        {entry && (
          <div className="stash-actions">
            <span className="spacer" />
            <button className="btn btn-sm" onClick={() => void drop(entry)}>
              删除
            </button>
            <button className="btn btn-sm" onClick={() => void unshelve(entry, true)} title="应用修改，同时保留这条搁置记录">
              应用
            </button>
            <button className="btn btn-sm btn-primary" onClick={() => void unshelve(entry, false)}>
              取消搁置
            </button>
          </div>
        )}
      </div>
      <div className="stash-detail">
        {entry ? (
          <CommitDetailsPane oid={entry.oid} refs={[]} onJump={() => {}} hideParents />
        ) : (
          <div className="diff-empty">{list?.length ? "选择一条搁置记录查看修改" : ""}</div>
        )}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={items(menu.e)} onClose={() => setMenu(null)} />}
    </div>
  );
}

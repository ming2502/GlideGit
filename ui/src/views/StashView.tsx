// Stash view: the list of stashes with apply / pop / drop, and the selected
// stash's changes in the same details pane the Log uses. Also the dialog
// that creates a stash.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { api } from "../api";
import type { CommitDetails } from "../bindings/CommitDetails";
import type { CommitFile } from "../bindings/CommitFile";
import type { StashEntry } from "../bindings/StashEntry";
import { ContextMenu, type MenuItem } from "../components/ContextMenu";
import { relativeTime } from "../lib/format";
import { load, save } from "../lib/storage";
import { useRepo } from "../stores/repo";
import { CommitDetailsPane } from "./CommitDetails";
import { ShelfPanel } from "./ShelfView";
import { t } from "../i18n";

/** Stash and Shelf, switched by tabs in the side panel's header. */
export function StashView() {
  const [tab, setTab] = useState<"stash" | "shelf">(() => load("stashTab", "stash"));
  useEffect(() => save("stashTab", tab), [tab]);
  const tabs = (
    <span className="seg">
      <button className={tab === "stash" ? "on" : ""} onClick={() => setTab("stash")}>
        Stash
      </button>
      <button className={tab === "shelf" ? "on" : ""} onClick={() => setTab("shelf")}>
        Shelf
      </button>
    </span>
  );
  return tab === "stash" ? <StashPanel tabs={tabs} /> : <ShelfPanel tabs={tabs} />;
}

function StashPanel({ tabs }: { tabs: ReactNode }) {
  const root = useRepo((s) => s.root)!;
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const setStashDialog = useRepo((s) => s.setStashDialog);
  const [list, setList] = useState<StashEntry[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [restoreIndex, setRestoreIndex] = useState(() => load("stashRestoreIndex", false));
  const [menu, setMenu] = useState<{ x: number; y: number; e: StashEntry } | null>(null);
  useEffect(() => save("stashRestoreIndex", restoreIndex), [restoreIndex]);

  const reload = useCallback(() => {
    api
      .stashList(root)
      .then((l) => {
        setList(l);
        setSelected((cur) => (cur && l.some((e) => e.oid === cur) ? cur : (l[0]?.oid ?? null)));
      })
      .catch(() => setList([]));
  }, [root]);
  // Stashing changes the working tree, so a new snapshot is a good moment to re-read.
  useEffect(reload, [reload, snapshot]);

  const entry = list?.find((e) => e.oid === selected) ?? null;
  const name = (e: StashEntry) => `stash@{${e.index}}`;

  const apply = (e: StashEntry, pop: boolean) =>
    void runTask(
      `${pop ? t("弹出") : t("应用")} ${name(e)}`,
      async (r) => {
        await api.stashApply(r, e.index, e.oid, pop, restoreIndex);
        reload();
      },
      pop ? t("已弹出 {name}", { name: name(e) }) : t("已应用 {name}", { name: name(e) }),
    ).then(reload);
  const drop = async (e: StashEntry) => {
    const ok = await ask({
      title: t("删除 {name}？", { name: name(e) }),
      message: t("“{message}” 中保存的修改将被删除。", { message: e.message }),
      confirmLabel: t("删除"),
      danger: true,
    });
    if (ok) void runTask(t("删除 {name}", { name: name(e) }), (r) => api.stashDrop(r, e.index, e.oid), t("已删除 {name}", { name: name(e) })).then(reload);
  };
  const items = (e: StashEntry): (MenuItem | "separator")[] => [
    { label: t("应用"), onClick: () => apply(e, false) },
    { label: t("弹出（应用后删除）"), onClick: () => apply(e, true) },
    "separator",
    { label: t("删除…"), danger: true, onClick: () => void drop(e) },
  ];

  const onKey = (ev: KeyboardEvent<HTMLDivElement>) => {
    if (!list?.length) return;
    const i = list.findIndex((e) => e.oid === selected);
    if (ev.key === "ArrowDown") setSelected(list[Math.min(list.length - 1, i + 1)].oid);
    else if (ev.key === "ArrowUp") setSelected(list[Math.max(0, i - 1)].oid);
    else if ((ev.key === "Delete" || ev.key === "Backspace") && entry) void drop(entry);
    else return;
    ev.preventDefault();
  };

  // Untracked files live in the stash's third parent, a commit of their own.
  const loadDetails = useCallback((r: string, oid: string) => api.stashDetails(r, oid), []);
  const loadTexts = useCallback(
    (r: string, d: CommitDetails, f: CommitFile) =>
      f.kind === "untracked"
        ? api.commitFileTexts(r, d.parents[2], null, f.path, null)
        : api.commitFileTexts(r, d.oid, d.parents[0] ?? null, f.path, f.origPath),
    [],
  );

  return (
    <div className="stash-view">
      <div className="stash-side">
        <div className="panel-header">
          {tabs}
          <span className="muted">{list ? t("{count} 个", { count: list.length }) : ""}</span>
          <span className="spacer" />
          <button className="btn btn-sm" onClick={() => setStashDialog(true)} title={t("把当前修改存入 stash")}>
            {t("Stash 修改…")}
          </button>
        </div>
        {list === null ? (
          <div className="empty">
            <span className="spinner inline" />{t("正在读取…")}
          </div>
        ) : list.length === 0 ? (
          <div className="empty">
            {t("没有 stash")}
            <div className="muted">{t("用「Stash 修改…」暂存当前改动，切换分支后再取回")}</div>
          </div>
        ) : (
          <div className="stash-list" tabIndex={0} onKeyDown={onKey}>
            {list.map((e) => (
              <div
                key={e.oid}
                className={`stash-row ${e.oid === selected ? "selected" : ""}`}
                onMouseDown={() => setSelected(e.oid)}
                onDoubleClick={() => apply(e, false)}
                onContextMenu={(ev) => {
                  ev.preventDefault();
                  setSelected(e.oid);
                  setMenu({ x: ev.clientX, y: ev.clientY, e });
                }}
              >
                <div className="stash-msg">{e.message || t("（无说明）")}</div>
                <div className="stash-meta muted">
                  <span className="mono">{name(e)}</span>
                  {e.branch && <span className="ref ref-local">{e.branch}</span>}
                  <span>{relativeTime(e.time)}</span>
                </div>
              </div>
            ))}
          </div>
        )}
        {entry && (
          <div className="stash-actions">
            <label className="log-dialog-check" title={t("同时恢复 stash 时已暂存的修改（git stash apply --index）")}>
              <input type="checkbox" checked={restoreIndex} onChange={(ev) => setRestoreIndex(ev.target.checked)} />
              {t("恢复暂存状态")}
            </label>
            <span className="spacer" />
            <button className="btn btn-sm" onClick={() => void drop(entry)}>
              {t("删除")}
            </button>
            <button className="btn btn-sm" onClick={() => apply(entry, false)}>
              {t("应用")}
            </button>
            <button className="btn btn-sm btn-primary" onClick={() => apply(entry, true)}>
              {t("弹出")}
            </button>
          </div>
        )}
      </div>
      <div className="stash-detail">
        {entry ? (
          <CommitDetailsPane oid={entry.oid} refs={[]} onJump={() => {}} load={loadDetails} loadTexts={loadTexts} hideParents />
        ) : (
          <div className="diff-empty">{list?.length ? t("选择一个 stash 查看修改") : ""}</div>
        )}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={items(menu.e)} onClose={() => setMenu(null)} />}
    </div>
  );
}

/** "Stash changes…": message, untracked files, keep the staged changes. */
export function StashDialog() {
  const open = useRepo((s) => s.stashDialog);
  const setOpen = useRepo((s) => s.setStashDialog);
  const runTask = useRepo((s) => s.runTask);
  const hasUntracked = useRepo((s) => !!s.snapshot?.changes.some((c) => c.unstaged === "untracked"));
  const [message, setMessage] = useState("");
  const [untracked, setUntracked] = useState(false);
  const [keepIndex, setKeepIndex] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    setMessage("");
    requestAnimationFrame(() => input.current?.focus());
  }, [open]);
  if (!open) return null;
  const close = () => setOpen(false);
  const submit = () => {
    close();
    void runTask(
      t("Stash 修改"),
      (r) => api.stashPush(r, { message: message.trim() || null, includeUntracked: untracked, keepIndex, paths: [] }),
      t("已存入 stash"),
    );
  };
  return (
    <div className="backdrop" onMouseDown={close}>
      <form
        className="dialog log-dialog"
        role="dialog"
        aria-modal
        aria-label={t("Stash 修改")}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && close()}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <h3>{t("Stash 修改")}</h3>
        <label className="log-dialog-field">
          <span>{t("说明（可选）")}</span>
          <input ref={input} className="lf-input" value={message} onChange={(e) => setMessage(e.target.value)} placeholder={t("例如：试验中的解析器改动")} />
        </label>
        <label className="log-dialog-check">
          <input type="checkbox" checked={untracked} onChange={(e) => setUntracked(e.target.checked)} />
          {t("包含未跟踪的文件{note}", { note: hasUntracked ? "" : t("（当前没有）") })}
        </label>
        <label className="log-dialog-check">
          <input type="checkbox" checked={keepIndex} onChange={(e) => setKeepIndex(e.target.checked)} />
          {t("保留已暂存的修改（--keep-index）")}
        </label>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            {t("取消")}
          </button>
          <button type="submit" className="btn btn-primary">
            Stash
          </button>
        </div>
      </form>
    </div>
  );
}

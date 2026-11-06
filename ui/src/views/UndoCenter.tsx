// Undo center: the operations GlideGit ran on this repository, newest first,
// each with what undoing it does (or why it cannot be undone).
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { api } from "../api";
import type { OpEntry } from "../bindings/OpEntry";
import type { OpKind } from "../bindings/OpKind";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

const KIND_ICON: Record<OpKind, string> = {
  commit: "●",
  amend: "✎",
  checkout: "⇢",
  branch: "⑂",
  tag: "⌗",
  reset: "⟲",
  cherryPick: "⇡",
  revert: "↺",
  rebase: "≋",
  pull: "↓",
  push: "↑",
  rollback: "⌫",
  discard: "⌫",
  stash: "⧉",
  shelve: "⊟",
  continue: "▸",
  undo: "↶",
  other: "•",
};

/** The repository's operation log, reloaded whenever its snapshot changes. */
export function useOperationLog(enabled = true): OpEntry[] {
  const root = useRepo((s) => s.root);
  const snapshot = useRepo((s) => s.snapshot);
  const [ops, setOps] = useState<OpEntry[]>([]);
  useEffect(() => {
    if (!root || !enabled) return;
    let alive = true;
    api
      .operationLog(root)
      .then((o) => alive && setOps(o))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [root, snapshot, enabled]);
  return ops;
}

/** Ask, then undo one operation. */
export async function undoOperation(op: OpEntry): Promise<void> {
  const { ask, runTask } = useRepo.getState();
  const ok = await ask({
    title: t("撤销「{title}」？", { title: op.title }),
    message: t("将会：{undo}。\n撤销前会确认仓库仍处于这次操作之后的状态。", { undo: op.undo }),
    confirmLabel: t("撤销"),
  });
  if (!ok) return;
  await runTask(t("撤销 {title}", { title: op.title }), (r) => api.undoOperation(r, op.id), t("已撤销：{title}", { title: op.title }));
}

export function UndoCenter() {
  const open = useRepo((s) => s.undoCenter);
  const setOpen = useRepo((s) => s.setUndoCenter);
  const ops = useOperationLog(open);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setActive(0);
    requestAnimationFrame(() => listRef.current?.focus());
  }, [open]);
  useEffect(() => {
    listRef.current?.querySelector(".uc-item.active")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (!open) return null;
  const close = () => setOpen(false);
  const run = (op: OpEntry | undefined) => {
    if (!op || !op.undo || op.undone) return;
    close();
    void undoOperation(op);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowDown") setActive((i) => Math.min(ops.length - 1, i + 1));
    else if (e.key === "ArrowUp") setActive((i) => Math.max(0, i - 1));
    else if (e.key === "Enter") run(ops[active]);
    else return;
    e.preventDefault();
  };

  return (
    <div className="backdrop clear" onMouseDown={close}>
      <div className="branch-popup undo-center" role="dialog" aria-modal aria-label={t("撤销中心")} onMouseDown={(e) => e.stopPropagation()}>
        <div className="uc-title">
          <b>{t("撤销中心")}</b>
          <span className="muted">{t("GlideGit 在这个仓库中执行过的操作")}</span>
        </div>
        <div className="bp-list uc-list" ref={listRef} tabIndex={0} onKeyDown={onKey}>
          {ops.length === 0 && <div className="uc-empty muted">{t("还没有记录的操作。提交、切换分支、重置、回滚等操作会出现在这里。")}</div>}
          {ops.map((op, i) => (
            <div
              key={op.id}
              className={`uc-item ${i === active ? "active" : ""} ${op.undone ? "undone" : ""}`}
              onMouseEnter={() => setActive(i)}
              onDoubleClick={() => run(op)}
            >
              <span className={`uc-icon uc-${op.kind}`}>{KIND_ICON[op.kind]}</span>
              <div className="uc-text">
                <div className="uc-name">
                  <span className="uc-op-title">{op.title}</span>
                  <span className="muted uc-time">{relativeTime(op.time)}</span>
                </div>
                <div className="uc-detail muted">
                  {op.undone ? t("已撤销") : op.undo ? `↶ ${op.undo}` : op.blocked}
                </div>
              </div>
              {op.undo && !op.undone && (
                <button className="btn btn-sm uc-undo" onClick={() => run(op)}>
                  {t("撤销")}
                </button>
              )}
            </div>
          ))}
        </div>
        <div className="bp-footer muted">{t("↑↓ 选择 · 回车 撤销 · 撤销前会检查仓库没有被再次修改")}</div>
      </div>
    </div>
  );
}

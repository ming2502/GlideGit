// Interactive rebase, graphically: the commits from the chosen one up to
// HEAD, newest first like the Log. Drag (or Alt+↑/↓) to reorder, pick an
// action per commit, edit messages inline, see the resulting branch on the
// right, then run it.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { api, errorMessage } from "../api";
import type { RebaseAction } from "../bindings/RebaseAction";
import { relativeTime } from "../lib/format";
import { isNoop, move, preview, problem, toSteps, type PlanRow } from "../lib/rebasePlan";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

const ACTIONS: { action: RebaseAction; label: string; key: string; hint: string }[] = [
  { action: "pick", label: t("保留"), key: "p", hint: t("pick：原样保留") },
  { action: "reword", label: t("改写信息"), key: "r", hint: t("reword：保留修改，换一条提交信息") },
  { action: "edit", label: t("暂停修改"), key: "e", hint: t("edit：应用后暂停，可修改后在横幅中继续") },
  { action: "squash", label: t("合并到下方"), key: "s", hint: t("squash：并入下方（更早）的提交，保留两条信息") },
  { action: "fixup", label: t("合并（丢弃信息）"), key: "f", hint: t("fixup：并入下方的提交，丢弃这条信息") },
  { action: "drop", label: t("删除"), key: "d", hint: t("drop：丢弃这个提交及其修改") },
];

export function RebaseDialog() {
  const from = useRepo((s) => s.rebaseFrom);
  const setFrom = useRepo((s) => s.setRebaseFrom);
  const root = useRepo((s) => s.root);
  const dirty = useRepo((s) => !!s.snapshot?.changes.some((c) => c.unstaged !== "untracked"));
  const branch = useRepo((s) => s.snapshot?.head.branch ?? null);
  const runTask = useRepo((s) => s.runTask);
  const toast = useRepo((s) => s.toast);
  const [original, setOriginal] = useState<PlanRow["commit"][] | null>(null);
  const [rows, setRows] = useState<PlanRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState(0);
  const [drag, setDrag] = useState<{ from: number; over: number } | null>(null);
  const [autostash, setAutostash] = useState(true);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!from || !root) return;
    let alive = true;
    setOriginal(null);
    setError(null);
    setSel(0);
    api
      .rebasePlan(root, from.oid)
      .then((plan) => {
        if (!alive) return;
        setOriginal(plan);
        setRows([...plan].reverse().map((commit) => ({ commit, action: "pick", message: null })));
        requestAnimationFrame(() => list.current?.focus());
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [from, root]);

  const result = useMemo(() => preview(rows), [rows]);
  if (!from) return null;
  const close = () => setFrom(null);
  const why = problem(rows);
  const noop = original ? isNoop(rows, original) : true;

  const setAction = (i: number, action: RebaseAction) => {
    setRows((rs) => rs.map((r, k) => (k === i ? { ...r, action } : r)));
    // Rewording starts from the commit's full message.
    const r = rows[i];
    if (action === "reword" && r.message == null && root) {
      void api
        .commitDetails(root, r.commit.oid)
        .then((d) => setRows((rs) => rs.map((x) => (x.commit.oid === r.commit.oid && x.message == null ? { ...x, message: d.message } : x))))
        .catch(() => setRows((rs) => rs.map((x) => (x.commit.oid === r.commit.oid ? { ...x, message: r.commit.subject } : x))));
    }
  };

  // Dragging with mouse events rather than HTML5 drag and drop: the
  // webview's file-drop handling can swallow native drag events (Windows).
  const startDrag = (e: React.MouseEvent, from: number) => {
    e.preventDefault();
    setSel(from);
    setDrag({ from, over: from });
    const rowAt = (y: number) => {
      const els = [...(list.current?.querySelectorAll(".rb-row") ?? [])];
      const i = els.findIndex((el) => y < el.getBoundingClientRect().bottom);
      return i < 0 ? els.length - 1 : i;
    };
    const onMove = (ev: MouseEvent) => setDrag({ from, over: rowAt(ev.clientY) });
    const onUp = (ev: MouseEvent) => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.classList.remove("resizing-v");
      const to = rowAt(ev.clientY);
      setDrag(null);
      if (to !== from) {
        setRows((rs) => move(rs, from, to));
        setSel(to);
      }
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.classList.add("resizing-v");
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest("textarea, select")) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const d = e.key === "ArrowDown" ? 1 : -1;
      const to = Math.max(0, Math.min(rows.length - 1, sel + d));
      if (e.altKey) setRows((rs) => move(rs, sel, to));
      setSel(to);
    } else {
      const a = ACTIONS.find((x) => x.key === e.key.toLowerCase() && !e.metaKey && !e.ctrlKey);
      if (!a) return;
      setAction(sel, a.action);
    }
    e.preventDefault();
  };

  const start = () => {
    if (!root || why) return;
    const steps = toSteps(rows);
    close();
    void runTask(t("交互式变基"), async (r) => {
      const finished = await api.interactiveRebase(r, from.oid, steps, autostash && dirty);
      toast(
        finished
          ? { kind: "success", title: t("变基完成") }
          : { kind: "info", title: t("变基已暂停"), detail: t("按横幅提示修改或解决冲突后点「继续」") },
      );
    });
  };

  return (
    <div className="backdrop" onMouseDown={close}>
      <div
        className="dialog rb-dialog"
        role="dialog"
        aria-modal
        aria-label={t("交互式变基")}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && close()}
      >
        <h3>
          {t("交互式变基{note}", { note: branch ? ` ${branch}` : "" })}
          <span className="muted">{t("（从 {hash} {subject} 起，共 {count} 个提交）", { hash: from.oid.slice(0, 7), subject: from.subject, count: rows.length })}</span>
        </h3>
        {error ? (
          <div className="rb-error">{error}</div>
        ) : !original ? (
          <div className="empty">
            <span className="spinner inline" />{t("正在读取提交…")}
          </div>
        ) : (
          <div className="rb-body">
            <div className="rb-plan">
              <div className="rb-hint muted">{t("拖动或 Alt+↑/↓ 调整顺序 · 快捷键 P 保留 / R 改写 / E 暂停 / S 合并 / F 合并丢信息 / D 删除")}</div>
              <div className="rb-list" ref={list} tabIndex={0} onKeyDown={onKey}>
                {rows.map((r, i) => (
                  <div
                    key={r.commit.oid}
                    className={`rb-row rb-${r.action} ${i === sel ? "selected" : ""} ${drag?.over === i && drag.from !== i ? (drag.from < i ? "drop-below" : "drop-above") : ""}`}
                    onMouseDown={() => setSel(i)}
                  >
                    <span className="rb-handle" title={t("拖动调整顺序")} onMouseDown={(e) => startDrag(e, i)}>
                      ⋮⋮
                    </span>
                    <select
                      className="rb-action"
                      value={r.action}
                      onChange={(e) => setAction(i, e.target.value as RebaseAction)}
                      title={ACTIONS.find((a) => a.action === r.action)?.hint}
                    >
                      {ACTIONS.map((a) => (
                        <option key={a.action} value={a.action}>
                          {a.label}
                        </option>
                      ))}
                    </select>
                    <span className="mono rb-hash">{r.commit.oid.slice(0, 7)}</span>
                    <div className="rb-text">
                      {r.action === "reword" ? (
                        <textarea
                          className="lf-input rb-message"
                          rows={Math.min(6, Math.max(2, (r.message ?? "").split("\n").length))}
                          value={r.message ?? ""}
                          placeholder={t("新的提交信息")}
                          onChange={(e) =>
                            setRows((rs) => rs.map((x, k) => (k === i ? { ...x, message: e.target.value } : x)))
                          }
                        />
                      ) : (
                        <span className="rb-subject">{r.commit.subject}</span>
                      )}
                    </div>
                    <span className="muted rb-meta">
                      {r.commit.authorName} · {relativeTime(r.commit.authorTime)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
            <div className="rb-preview">
              <div className="rb-hint muted">{t("变基后的提交（新 → 旧）")}</div>
              {result.map((c) => (
                <div key={c.row.commit.oid} className="rb-result">
                  <span className="rb-dot" />
                  <div>
                    <div className="rb-subject">{c.subject}</div>
                    {(c.melded.length > 0 || c.stops) && (
                      <div className="muted rb-sub">
                        {c.melded.length > 0 && t("并入 {count} 个提交：{subjects}", { count: c.melded.length, subjects: c.melded.map((m) => m.commit.oid.slice(0, 7)).join(t("、")) })}
                        {c.stops && (c.melded.length ? " · " : "") + t("应用后暂停")}
                      </div>
                    )}
                  </div>
                </div>
              ))}
              <div className="rb-result rb-base">
                <span className="rb-dot" />
                <span className="muted">{t("（之前的历史）")}</span>
              </div>
            </div>
          </div>
        )}
        <div className="rb-footer">
          {why && original && <span className="rb-error">{why}</span>}
          {!why && noop && original && <span className="muted">{t("计划与当前一致，不会有任何变化")}</span>}
          <span className="spacer" />
          {dirty && (
            <label className="log-dialog-check" title={t("git rebase --autostash：先 stash 本地修改，结束后自动恢复")}>
              <input type="checkbox" checked={autostash} onChange={(e) => setAutostash(e.target.checked)} />
              {t("自动 stash 本地修改")}
            </label>
          )}
          <button className="btn" onClick={close}>
            {t("取消")}
          </button>
          <button className="btn btn-primary" disabled={!original || !!why || noop} onClick={start}>
            {t("开始变基")}
          </button>
        </div>
      </div>
    </div>
  );
}

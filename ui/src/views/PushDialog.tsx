import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../api";
import type { PushPreview } from "../bindings/PushPreview";
import { relativeTime } from "../lib/format";
import { formatKeys } from "../actions/registry";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

/** IDEA-style push dialog: shows exactly what will be sent before pushing. */
export function PushDialog() {
  const open = useRepo((s) => s.pushDialog);
  const root = useRepo((s) => s.root);
  const setOpen = useRepo((s) => s.setPushDialog);
  const [preview, setPreview] = useState<PushPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [force, setForce] = useState(false);
  const pushRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open || !root) return;
    let alive = true;
    setPreview(null);
    setError(null);
    setForce(false);
    api
      .pushPreview(root)
      .then((p) => {
        if (!alive) return;
        setPreview(p);
        requestAnimationFrame(() => pushRef.current?.focus());
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [open, root]);

  if (!open) return null;

  const close = () => setOpen(false);
  const canPush =
    !!preview?.branch && !!preview.remote && (preview.commits.length > 0 || preview.newBranch || force);

  async function push() {
    if (!canPush) return;
    const lease = force;
    close();
    await useRepo
      .getState()
      .runRemote(t("推送"), (r, id) => api.push(r, lease, id), t("已推送到 {target}", { target: preview?.target ?? t("远端") }));
  }

  return (
    <div className="backdrop" onMouseDown={close}>
      <div
        className="dialog push-dialog"
        role="dialog"
        aria-modal
        aria-label="Push Commits"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") close();
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void push();
        }}
      >
        <h3>Push Commits</h3>
        {error ? (
          <p className="error-text">{error}</p>
        ) : !preview ? (
          <div className="push-loading">
            <span className="spinner" />{t("正在计算待推送的提交…")}
          </div>
        ) : !preview.branch ? (
          <p>{t("当前处于 detached HEAD，无法推送。请先切换到分支。")}</p>
        ) : !preview.remote ? (
          <p>{t("仓库还没有配置远端（remote），无法推送。")}</p>
        ) : (
          <>
            <div className="push-target">
              <b>{preview.branch}</b>
              <span className="muted">→</span>
              <b>{preview.target}</b>
              {preview.newBranch && <span className="pill">{t("新分支")}</span>}
            </div>
            {preview.behind > 0 && (
              <div className="push-warn">
                {t("远端有 {behind} 个本地没有的提交，普通推送会被拒绝。建议先 Pull； 确实要覆盖时勾选下方的 Force Push。", { behind: preview.behind })}
              </div>
            )}
            <div className="push-commits">
              {preview.commits.length === 0 ? (
                <div className="empty">
                  {preview.newBranch ? t("没有新提交，将在远端创建该分支") : t("没有需要推送的提交")}
                </div>
              ) : (
                preview.commits.map((c) => (
                  <div key={c.oid} className="push-commit">
                    <span className="subject-text">{c.subject}</span>
                    <span className="muted">{c.authorName}</span>
                    <span className="muted">{relativeTime(c.authorTime)}</span>
                    <span className="mono muted">{c.oid.slice(0, 8)}</span>
                  </div>
                ))
              )}
            </div>
            <label className="opt">
              <input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} />
              {t("Force Push（--force-with-lease：只在远端未被他人更新时覆盖）")}
            </label>
          </>
        )}
        <div className="dialog-actions">
          <button className="btn" onClick={close}>
            {t("取消")}
          </button>
          <button
            ref={pushRef}
            className={`btn ${force ? "btn-danger" : "btn-primary"}`}
            disabled={!canPush}
            onClick={() => void push()}
          >
            {force ? "Force Push" : "Push"}
            {preview && preview.commits.length > 0 && t(" {count} 个提交", { count: preview.commits.length })}
            <span className="kbd">{formatKeys(["mod+enter"])}</span>
          </button>
        </div>
      </div>
    </div>
  );
}

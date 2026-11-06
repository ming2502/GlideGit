import { useEffect, useRef } from "react";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

export function ConfirmDialog() {
  const req = useRepo((s) => s.confirm);
  const okRef = useRef<HTMLButtonElement>(null);
  useEffect(() => okRef.current?.focus(), [req]);
  if (!req) return null;
  return (
    <div className="backdrop" onMouseDown={() => req.resolve(false)}>
      <div
        className="dialog"
        role="dialog"
        aria-modal
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && req.resolve(false)}
      >
        <h3>{req.title}</h3>
        <p>{req.message}</p>
        <div className="dialog-actions">
          <button className="btn" onClick={() => req.resolve(false)}>
            {t("取消")}
          </button>
          <button
            ref={okRef}
            className={`btn ${req.danger ? "btn-danger" : "btn-primary"}`}
            onClick={() => req.resolve(true)}
          >
            {req.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

export function Toasts() {
  const toasts = useRepo((s) => s.toasts);
  const dismiss = useRepo((s) => s.dismissToast);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.kind}`} onClick={() => dismiss(t.id)}>
          <div className="toast-title">{t.title}</div>
          {t.detail && <pre className="toast-detail">{t.detail}</pre>}
        </div>
      ))}
    </div>
  );
}

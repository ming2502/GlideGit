// Dialogs opened from the Log's context menu: new branch / tag at a commit,
// and reset of the current branch with a choice of mode.
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { ResetMode } from "../bindings/ResetMode";

function Dialog({
  title,
  children,
  onCancel,
  onSubmit,
  submitLabel,
  danger,
  canSubmit = true,
}: {
  title: string;
  children: ReactNode;
  onCancel: () => void;
  onSubmit: () => void;
  submitLabel: string;
  danger?: boolean;
  canSubmit?: boolean;
}) {
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (canSubmit) onSubmit();
  };
  return (
    <div className="backdrop" onMouseDown={onCancel}>
      <form
        className="dialog log-dialog"
        role="dialog"
        aria-modal
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onCancel()}
        onSubmit={submit}
      >
        <h3>{title}</h3>
        {children}
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={onCancel}>
            取消
          </button>
          <button type="submit" className={`btn ${danger ? "btn-danger" : "btn-primary"}`} disabled={!canSubmit}>
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}

/** Characters git never allows in a ref name (it checks the rest). */
const BAD_REF = /(^[-/.]|[/.]$|\.\.|@\{|[\s~^:?*[\\]|\.lock$)/;

function CommitLine({ commit }: { commit: CommitSummary }) {
  return (
    <div className="log-dialog-commit">
      <span className="mono">{commit.oid.slice(0, 8)}</span> <span>{commit.subject}</span>
    </div>
  );
}

export function NewRefDialog({
  kind,
  commit,
  onCancel,
  onSubmit,
}: {
  kind: "branch" | "tag";
  commit: CommitSummary;
  onCancel: () => void;
  onSubmit: (name: string, extra: { checkout: boolean; message: string }) => void;
}) {
  const [name, setName] = useState("");
  const [checkout, setCheckout] = useState(true);
  const [message, setMessage] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const trimmed = name.trim();
  const invalid = trimmed !== "" && BAD_REF.test(trimmed);
  return (
    <Dialog
      title={kind === "branch" ? "从此提交新建分支" : "在此提交新建标签"}
      onCancel={onCancel}
      onSubmit={() => onSubmit(trimmed, { checkout, message })}
      submitLabel="创建"
      canSubmit={trimmed !== "" && !invalid}
    >
      <CommitLine commit={commit} />
      <label className="log-dialog-field">
        <span>{kind === "branch" ? "分支名" : "标签名"}</span>
        <input
          ref={input}
          className="lf-input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={kind === "branch" ? "feature/…" : "v1.0.0"}
          spellCheck={false}
        />
      </label>
      {invalid && <div className="log-dialog-error">名称包含 git 不允许的字符或格式</div>}
      {kind === "branch" ? (
        <label className="log-dialog-check">
          <input type="checkbox" checked={checkout} onChange={(e) => setCheckout(e.target.checked)} />
          创建后切换到新分支
        </label>
      ) : (
        <label className="log-dialog-field">
          <span>说明（可选，填写后创建附注标签）</span>
          <textarea className="lf-input lf-paths" rows={3} value={message} onChange={(e) => setMessage(e.target.value)} />
        </label>
      )}
    </Dialog>
  );
}

const RESET_MODES: { mode: ResetMode; label: string; detail: string }[] = [
  { mode: "soft", label: "Soft", detail: "只移动分支；之后的改动保留在暂存区" },
  { mode: "mixed", label: "Mixed", detail: "移动分支并重置暂存区；改动保留在工作区" },
  { mode: "keep", label: "Keep", detail: "移动分支并更新工作区；会丢失的本地修改则拒绝执行" },
  { mode: "hard", label: "Hard", detail: "移动分支并丢弃所有本地修改，无法恢复" },
];

export function ResetDialog({
  commit,
  branch,
  onCancel,
  onSubmit,
}: {
  commit: CommitSummary;
  /** Current branch name, or null for a detached HEAD. */
  branch: string | null;
  onCancel: () => void;
  onSubmit: (mode: ResetMode) => void;
}) {
  const [mode, setMode] = useState<ResetMode>("mixed");
  return (
    <Dialog
      title={branch ? `将 ${branch} 重置到此提交` : "将 HEAD 重置到此提交"}
      onCancel={onCancel}
      onSubmit={() => onSubmit(mode)}
      submitLabel={mode === "hard" ? "重置并丢弃修改" : "重置"}
      danger={mode === "hard"}
    >
      <CommitLine commit={commit} />
      <div className="log-dialog-modes" role="radiogroup">
        {RESET_MODES.map((m) => (
          <label key={m.mode} className={`log-dialog-mode ${mode === m.mode ? "on" : ""}`}>
            <input type="radio" name="reset-mode" checked={mode === m.mode} onChange={() => setMode(m.mode)} />
            <span>
              <b>{m.label}</b>
              <span className="muted">{m.detail}</span>
            </span>
          </label>
        ))}
      </div>
    </Dialog>
  );
}

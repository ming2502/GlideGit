// Shown while a merge, rebase, cherry-pick or revert is stopped (usually on
// conflicts): what is going on, plus Abort and Continue.
import { api } from "../api";
import type { RepoOperation } from "../bindings/RepoOperation";
import { useRepo } from "../stores/repo";

const NAMES: Record<RepoOperation, string> = {
  merge: "合并",
  rebase: "变基",
  cherryPick: "Cherry-pick",
  revert: "Revert",
};

export function OperationBanner() {
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const busy = useRepo((s) => s.tasks.length > 0);
  const op = snapshot?.operation;
  if (!snapshot || !op) return null;
  // Space between Chinese and Latin text: "中止 Cherry-pick", "中止合并".
  const name = /^[A-Za-z]/.test(NAMES[op]) ? ` ${NAMES[op]} ` : NAMES[op];
  const conflicts = snapshot.changes.filter((c) => c.conflicted).length;

  const abort = async () => {
    const ok = await ask({
      title: `中止${name}？`.replace(" ？", "？"),
      message: "工作区和分支会回到开始之前的状态，已解决的冲突也会丢失。",
      confirmLabel: "中止",
      danger: true,
    });
    if (ok) void runTask(`中止${name}`.trim(), (r) => api.abortOperation(r), `已中止${name}`.trim());
  };

  return (
    <div className="op-banner" role="status">
      <span>
        正在进行<b>{name.trim()}</b>
        {conflicts > 0 ? `，还有 ${conflicts} 个冲突文件待解决` : "，冲突已解决，可以继续"}
      </span>
      <span className="spacer" />
      {conflicts > 0 && (
        <button
          className="btn"
          onClick={() => useRepo.getState().setMergePath(snapshot.changes.find((c) => c.conflicted)!.path)}
          title="在三方合并工具中打开第一个冲突文件（也可双击冲突文件）"
        >
          解决冲突…
        </button>
      )}
      <button className="btn" disabled={busy} onClick={() => void abort()}>
        中止
      </button>
      <button
        className="btn btn-primary"
        disabled={busy || conflicts > 0}
        title={conflicts > 0 ? "先解决并暂存所有冲突文件" : undefined}
        onClick={() => void runTask(`继续${name}`.trim(), (r) => api.continueOperation(r), `${name}已完成`.trim())}
      >
        继续
      </button>
    </div>
  );
}

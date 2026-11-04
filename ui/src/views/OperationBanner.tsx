// Shown while a merge, rebase, cherry-pick or revert is stopped (usually on
// conflicts): what is going on and how far it got, plus the ways forward —
// resolve conflicts, skip this commit, abort, or continue.
import { api } from "../api";
import type { RepoOperation } from "../bindings/RepoOperation";
import { useRepo } from "../stores/repo";

const NAMES: Record<RepoOperation, string> = {
  merge: "合并",
  rebase: "变基",
  cherryPick: "Cherry-pick",
  revert: "Revert",
};

/** Space between Chinese and Latin text: "中止 Cherry-pick", "中止合并". */
const spaced = (s: string) => (/^[A-Za-z]/.test(s) ? ` ${s} ` : s);
const tidy = (s: string) => s.replace(/\s+/g, " ").replace(/ ([，。？])/g, "$1").trim();

export function OperationBanner() {
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const busy = useRepo((s) => s.tasks.length > 0);
  const op = snapshot?.operation;
  if (!snapshot || !op) return null;
  const name = spaced(NAMES[op]);
  const info = snapshot.operationInfo;
  const conflicts = snapshot.changes.filter((c) => c.conflicted).length;

  // What is happening, as specific as git's state files allow.
  let what: string;
  if (op === "rebase") {
    const target = info?.onto ? `到 ${info.onto}` : "";
    what = info?.branch ? `正在将 ${info.branch} 变基${target}` : `正在变基${target}`;
  } else if (op === "merge") what = info?.commit ? `正在合并 ${info.commit}` : "正在合并";
  else what = `正在${name}${info?.commit ?? ""}`;
  const step =
    op === "rebase" && info?.step != null && info.total != null
      ? `第 ${info.step}/${info.total} 个提交${info.commit ? `：${info.commit}` : ""}`
      : null;
  const state =
    conflicts > 0
      ? `还有 ${conflicts} 个冲突文件待解决`
      : op === "rebase" && !info?.commit
        ? "已暂停，可修改后继续"
        : "冲突已解决，可以继续";

  const abort = async () => {
    const ok = await ask({
      title: tidy(`中止${name}？`),
      message: "工作区和分支会回到开始之前的状态，已解决的冲突也会丢失。",
      confirmLabel: "中止",
      danger: true,
    });
    if (ok) void runTask(tidy(`中止${name}`), (r) => api.abortOperation(r), tidy(`已中止${name}`));
  };

  const skip = async () => {
    const ok = await ask({
      title: "跳过这个提交？",
      message: `${info?.commit ?? "当前提交"} 的修改不会被应用${op === "rebase" ? "，变基会继续处理后面的提交" : ""}。已经做的冲突处理也会丢弃。`,
      confirmLabel: "跳过",
      danger: true,
    });
    if (ok) void runTask("跳过提交", (r) => api.skipOperation(r), "已跳过");
  };

  return (
    <div className="op-banner" role="status">
      <span className="op-text">
        <b title={what}>{what}</b>
        {step && <span className="op-step" title={step}>{step}</span>}
        <span className={conflicts > 0 ? "op-state bad" : "op-state ok"}>{state}</span>
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
      {info?.canSkip && (
        <button className="btn" disabled={busy} onClick={() => void skip()} title="不应用当前这个提交，继续后面的">
          跳过此提交
        </button>
      )}
      <button className="btn" disabled={busy} onClick={() => void abort()}>
        中止
      </button>
      <button
        className="btn btn-primary"
        disabled={busy || conflicts > 0}
        title={conflicts > 0 ? "先解决并暂存所有冲突文件" : undefined}
        onClick={() => void runTask(tidy(`继续${name}`), (r) => api.continueOperation(r), tidy(`${name}已完成`))}
      >
        继续
      </button>
    </div>
  );
}

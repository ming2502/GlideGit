// Shown while a merge, rebase, cherry-pick or revert is stopped (usually on
// conflicts): what is going on and how far it got, plus the ways forward —
// resolve conflicts, skip this commit, abort, or continue.
import { api } from "../api";
import type { RepoOperation } from "../bindings/RepoOperation";
import { useRepo } from "../stores/repo";
import { language, t } from "../i18n";

const NAMES: Record<RepoOperation, string> = {
  merge: t("合并"),
  rebase: t("变基"),
  cherryPick: t("Cherry-pick"),
  revert: t("Revert"),
};

/** Space between Chinese and Latin text: "中止 Cherry-pick", "中止合并" (English has its spaces already). */
const spaced = (s: string) => (language === "zh" && /^[A-Za-z]/.test(s) ? ` ${s} ` : s);
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
    const target = info?.onto ? t("到 {onto}", { onto: info.onto }) : "";
    what = info?.branch ? t("正在将 {branch} 变基{target}", { branch: info.branch, target }) : t("正在变基{target}", { target });
  } else if (op === "merge") what = info?.commit ? t("正在合并 {commit}", { commit: info.commit }) : t("正在合并");
  else what = t("正在{name}{target}", { name, target: info?.commit ?? "" });
  const step =
    op === "rebase" && info?.step != null && info.total != null
      ? t("第 {step}/{total} 个提交{commit}", { step: info.step, total: info.total, commit: info.commit ? t("：{commit}", { commit: info.commit }) : "" })
      : null;
  const state =
    conflicts > 0
      ? t("还有 {conflicts} 个冲突文件待解决", { conflicts })
      : op === "rebase" && !info?.commit
        ? t("已暂停，可修改后继续")
        : t("冲突已解决，可以继续");

  const abort = async () => {
    const ok = await ask({
      title: tidy(t("中止{name}？", { name })),
      message: t("工作区和分支会回到开始之前的状态，已解决的冲突也会丢失。"),
      confirmLabel: t("中止"),
      danger: true,
    });
    if (ok) void runTask(tidy(t("中止{name}", { name })), (r) => api.abortOperation(r), tidy(t("已中止{name}", { name })));
  };

  const skip = async () => {
    const ok = await ask({
      title: t("跳过这个提交？"),
      message: t("{commit} 的修改不会被应用{rest}。已经做的冲突处理也会丢弃。", { commit: info?.commit ?? t("当前提交"), rest: op === "rebase" ? t("，变基会继续处理后面的提交") : "" }),
      confirmLabel: t("跳过"),
      danger: true,
    });
    if (ok) void runTask(t("跳过提交"), (r) => api.skipOperation(r), t("已跳过"));
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
          title={t("在三方合并工具中打开第一个冲突文件（也可双击冲突文件）")}
        >
          {t("解决冲突…")}
        </button>
      )}
      {info?.canSkip && (
        <button className="btn" disabled={busy} onClick={() => void skip()} title={t("不应用当前这个提交，继续后面的")}>
          {t("跳过此提交")}
        </button>
      )}
      <button className="btn" disabled={busy} onClick={() => void abort()}>
        {t("中止")}
      </button>
      <button
        className="btn btn-primary"
        disabled={busy || conflicts > 0}
        title={conflicts > 0 ? t("先解决并暂存所有冲突文件") : undefined}
        onClick={() => void runTask(tidy(t("继续{name}", { name })), (r) => api.continueOperation(r), tidy(t("{name}已完成", { name })))}
      >
        {t("继续")}
      </button>
    </div>
  );
}

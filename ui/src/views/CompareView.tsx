// Branch comparison: commits only on one side or the other, and the files
// that differ (tip to tip, or what one side changed since they forked).
import { useEffect, useState, type KeyboardEvent } from "react";
import { api, errorMessage } from "../api";
import type { BranchCompare } from "../bindings/BranchCompare";
import type { ChangeKind } from "../bindings/ChangeKind";
import type { CommitFile } from "../bindings/CommitFile";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { FileTexts } from "../bindings/FileTexts";
import { relativeTime, splitPath } from "../lib/format";
import { useRepo } from "../stores/repo";
import { CommitDetailsPane } from "./CommitDetails";
import { TextsDiff } from "./DiffView";
import { t } from "../i18n";

type Tab = "left" | "right" | "files";
type Mode = "tips" | "left" | "right";

const KIND_LABEL: Record<ChangeKind, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  copied: "C",
  typeChanged: "T",
  untracked: "U",
  conflicted: "!",
};

function CommitList({ commits, total }: { commits: CommitSummary[]; total: number }) {
  const [sel, setSel] = useState(0);
  const showInLog = useRepo((s) => s.showInLog);
  useEffect(() => setSel(0), [commits]);
  const cur = commits[sel];
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") setSel((i) => Math.min(commits.length - 1, i + 1));
    else if (e.key === "ArrowUp") setSel((i) => Math.max(0, i - 1));
    else if (e.key === "Enter" && cur) showInLog(cur.oid);
    else return;
    e.preventDefault();
  };
  if (!commits.length) return <div className="diff-empty">{t("没有对方缺少的提交")}</div>;
  return (
    <div className="hv-body">
      <div className="hv-list" tabIndex={0} onKeyDown={onKey}>
        {commits.map((c, i) => (
          <div
            key={c.oid}
            className={`hv-row ${i === sel ? "selected" : ""}`}
            onMouseDown={() => setSel(i)}
            onDoubleClick={() => showInLog(c.oid)}
            title={t("双击或回车在 Log 中显示")}
          >
            <div className="hv-subject">{c.subject}</div>
            <div className="hv-meta muted">
              <span className="mono">{c.oid.slice(0, 7)}</span>
              <span>{c.authorName}</span>
              <span>{relativeTime(c.authorTime)}</span>
            </div>
          </div>
        ))}
        {total > commits.length && <div className="muted hv-more">{t("另有 {count} 个更早的提交未列出", { count: total - commits.length })}</div>}
      </div>
      <div className="hv-diff">{cur && <CommitDetailsPane oid={cur.oid} refs={cur.refs} onJump={showInLog} />}</div>
    </div>
  );
}

function FileDiffs({ from, to, label }: { from: string; to: string; label: string }) {
  const root = useRepo((s) => s.root)!;
  const [files, setFiles] = useState<CommitFile[] | null>(null);
  const [sel, setSel] = useState(0);
  const [texts, setTexts] = useState<FileTexts | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setFiles(null);
    setSel(0);
    api
      .diffFiles(root, from, to)
      .then((f) => alive && setFiles(f))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [root, from, to]);

  const file = files?.[sel] ?? null;
  useEffect(() => {
    if (!file) return;
    let alive = true;
    setLoading(true);
    api
      .commitFileTexts(root, to, from, file.path, file.origPath)
      .then((t) => alive && setTexts(t))
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [root, from, to, file]);

  if (error) return <div className="diff-empty">{error}</div>;
  if (!files)
    return (
      <div className="diff-empty">
        <span className="spinner" />
      </div>
    );
  if (!files.length) return <div className="diff-empty">{t("两边的文件完全相同")}</div>;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") setSel((i) => Math.min(files.length - 1, i + 1));
    else if (e.key === "ArrowUp") setSel((i) => Math.max(0, i - 1));
    else return;
    e.preventDefault();
  };
  return (
    <div className="hv-body">
      <div className="hv-list" tabIndex={0} onKeyDown={onKey}>
        {files.map((f, i) => {
          const { name, dir } = splitPath(f.path);
          return (
            <div key={f.path} className={`row file-row cmp-file ${i === sel ? "selected" : ""}`} onMouseDown={() => setSel(i)} title={f.origPath ? `${f.origPath} → ${f.path}` : f.path}>
              <span className={`kind kind-${f.kind}`}>{KIND_LABEL[f.kind]}</span>
              <span className={`fname kind-${f.kind}`}>{name}</span>
              <span className="fdir">{dir}</span>
              <span className="spacer" />
              {f.added != null && (
                <span className="diff-stats">
                  <span className="add">+{f.added}</span> <span className="del">−{f.deleted}</span>
                </span>
              )}
            </div>
          );
        })}
      </div>
      <div className="hv-diff">
        {file && <TextsDiff path={file.path} texts={texts && texts.path === file.path ? texts : null} loading={loading} label={label} />}
      </div>
    </div>
  );
}

export function CompareView() {
  const target = useRepo((s) => s.compare);
  const setCompare = useRepo((s) => s.setCompare);
  const root = useRepo((s) => s.root);
  const [data, setData] = useState<BranchCompare | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("left");
  const [mode, setMode] = useState<Mode>("tips");

  useEffect(() => {
    if (!target || !root) return;
    let alive = true;
    setData(null);
    setError(null);
    api
      .compareBranches(root, target.left, target.right)
      .then((d) => {
        if (!alive) return;
        setData(d);
        setTab(d.onlyLeftCount === 0 && d.onlyRightCount > 0 ? "right" : "left");
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [target, root]);

  if (!target) return null;
  const { leftName: L, rightName: R } = target;
  const close = () => setCompare(null);
  const swap = () => setCompare({ left: target.right, right: target.left, leftName: R, rightName: L });
  const base = data?.mergeBase ?? null;
  // Which two commits the file tab compares, and how to describe it.
  const range: { from: string; to: string; label: string } | null = data
    ? mode === "tips"
      ? { from: data.rightOid, to: data.leftOid, label: `${R} → ${L}` }
      : base
        ? mode === "left"
          ? { from: base, to: data.leftOid, label: t("分叉点 {base} → {L}", { base: base.slice(0, 7), L }) }
          : { from: base, to: data.rightOid, label: t("分叉点 {base} → {R}", { base: base.slice(0, 7), R }) }
        : null
    : null;

  return (
    <div className="mt-overlay" role="dialog" aria-modal="true" aria-label={t("比较 {L} 与 {R}", { L, R })} onKeyDown={(e) => e.key === "Escape" && close()}>
      <div className="mt-window">
        <div className="mt-header">
          <b>
            {t("比较 {L} ⇄ {R}", { L, R })}
          </b>
          {data && (
            <span className="muted">
              {base ? t("分叉点 {base}", { base: base.slice(0, 7) }) : t("没有共同历史")}
              {data.onlyLeftCount === 0 && data.onlyRightCount === 0 && t("，两边指向同一提交")}
            </span>
          )}
          <span className="spacer" />
          <button className="btn btn-sm" onClick={swap} title={t("交换左右两边")}>
            {t("⇄ 交换")}
          </button>
          <button className="btn btn-sm" onClick={close} autoFocus>
            {t("关闭")}
          </button>
        </div>
        {error ? (
          <div className="diff-empty">{error}</div>
        ) : !data ? (
          <div className="diff-empty">
            <span className="spinner" /> {t("正在比较…")}
          </div>
        ) : (
          <>
            <div className="cmp-tabs">
              <button className={tab === "left" ? "on" : ""} onClick={() => setTab("left")}>
                {t("仅在 {L} 中的提交", { L })}{" "}<span className="cmp-count">{data.onlyLeftCount}</span>
              </button>
              <button className={tab === "right" ? "on" : ""} onClick={() => setTab("right")}>
                {t("仅在 {R} 中的提交", { R })}{" "}<span className="cmp-count">{data.onlyRightCount}</span>
              </button>
              <button className={tab === "files" ? "on" : ""} onClick={() => setTab("files")}>
                {t("文件差异")}
              </button>
              {tab === "files" && (
                <select className="rb-action cmp-mode" value={mode} onChange={(e) => setMode(e.target.value as Mode)}>
                  <option value="tips">{t("两端对比（{R} → {L}）", { R, L })}</option>
                  <option value="left" disabled={!base}>
                    {t("{L} 自分叉以来的改动", { L })}
                  </option>
                  <option value="right" disabled={!base}>
                    {t("{R} 自分叉以来的改动", { R })}
                  </option>
                </select>
              )}
            </div>
            {tab === "left" ? (
              <CommitList commits={data.onlyLeft} total={data.onlyLeftCount} />
            ) : tab === "right" ? (
              <CommitList commits={data.onlyRight} total={data.onlyRightCount} />
            ) : range ? (
              <FileDiffs key={`${range.from}:${range.to}`} from={range.from} to={range.to} label={range.label} />
            ) : (
              <div className="diff-empty">{t("两边没有共同历史，只能做两端对比")}</div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

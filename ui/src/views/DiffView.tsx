import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useActions } from "../actions/registry";
import type { FileTexts } from "../bindings/FileTexts";
import type { CodeDiffHandle, DiffLayout, DiffStats, Staging } from "../components/CodeDiff";
import { api } from "../api";

// CodeMirror is large; load it on first use so the app shell starts fast.
const CodeDiff = lazy(() => import("../components/CodeDiff").then((m) => ({ default: m.CodeDiff })));
import { splitPath } from "../lib/format";
import { load, save } from "../lib/storage";
import { useRepo } from "../stores/repo";

export function DiffView() {
  const diff = useRepo((s) => s.diff);
  const loading = useRepo((s) => s.diffLoading);
  const selectedPath = useRepo((s) => s.selectedPath);
  const change = useRepo((s) => s.snapshot?.changes.find((c) => c.path === s.selectedPath));
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);

  // Block/line staging: not for conflicts, deletions, or texts that would
  // not survive a round trip.
  const path = selectedPath;
  const staging: Staging | undefined =
    path && diff && diff.path === path && change && !change.conflicted && diff.new != null && !diff.lossy
      ? {
          index: diff.index,
          onStage: (content) =>
            void runTask("暂存选中的修改", (r) =>
              // Unstaging every block of a new file takes it out of the index.
              diff.old == null && content === "" ? api.unstage(r, [path]) : api.stageContent(r, path, content),
            ),
          onDiscard: async (content, index) => {
            const ok = await ask({
              title: "丢弃这块修改？",
              message: `${path} 中的这部分改动将还原为 HEAD 的内容，无法撤销。`,
              confirmLabel: "丢弃",
              danger: true,
            });
            if (!ok) return;
            const current = diff.new!;
            void runTask("丢弃修改", async (r) => {
              if (index != null) await api.stageContent(r, path, index);
              await api.writeWorktreeFile(r, path, content, current);
            });
          },
        }
      : undefined;

  if (!selectedPath) {
    return (
      <div className="diff-empty">
        <div>选择一个文件查看差异</div>
        <div className="muted">↑↓ 切换文件 · 空格 加入/排除提交</div>
      </div>
    );
  }
  return <TextsDiff path={selectedPath} texts={diff} loading={loading} label="HEAD ↔ 工作区" staging={staging} />;
}

/** Both versions of a file in the diff editor, with a header (name, line
 * counts, layout switch, change navigation). Binary, oversized and missing
 * files get a message instead. */
export function TextsDiff({
  path,
  texts,
  loading,
  label,
  staging,
}: {
  path: string;
  texts: FileTexts | null;
  loading: boolean;
  /** What is compared, shown on the right of the header. */
  label: string;
  /** Block/line staging (working-copy diffs only). */
  staging?: Staging;
}) {
  const [layout, setLayout] = useState<DiffLayout>(() => load<DiffLayout>("diffLayout", "split"));
  useEffect(() => save("diffLayout", layout), [layout]);
  // Counts belong to one file content: a new object whenever it changes. A
  // new index (after staging) keeps the editor, the content and the counts.
  const doc = useMemo(() => ({}), [path, texts?.old, texts?.new]);
  const [counted, setCounted] = useState<{ doc: object; s: DiffStats } | null>(null);
  const stats = counted?.doc === doc ? counted.s : null;
  const setStats = useCallback((s: DiffStats) => setCounted({ doc, s }), [doc]);
  const nav = useRef<CodeDiffHandle | null>(null);
  const setHandle = useCallback((h: CodeDiffHandle | null) => (nav.current = h), []);

  useActions([
    { id: "diff.next", title: "Next Change", keys: ["f7"], run: () => nav.current?.go(1) },
    { id: "diff.prev", title: "Previous Change", keys: ["shift+f7"], run: () => nav.current?.go(-1) },
  ]);

  const { name, dir } = splitPath(path);
  const old = texts?.old ?? null;
  const cur = texts?.new ?? null;
  let body: ReactNode;
  if (!texts) body = loading ? null : <div className="diff-empty">无法读取文件</div>;
  else if (texts.binary) body = <div className="diff-empty">二进制文件，无法显示文本差异</div>;
  else if (texts.tooLarge) body = <div className="diff-empty">文件超过 8 MB，未加载预览</div>;
  else if (old === null && cur === null) body = <div className="diff-empty">两侧都没有这个文件</div>;
  else if (old === cur) body = <div className="diff-empty">内容无差异（可能仅文件模式变化）</div>;
  else
    body = (
      <Suspense fallback={<div className="diff-empty"><span className="spinner" /></div>}>
      <CodeDiff
        key={`${path}:${layout}`}
        path={path}
        oldText={old ?? ""}
        newText={cur ?? ""}
        layout={layout}
        onStats={setStats}
        staging={staging}
        handle={setHandle}
      />
      </Suspense>
    );

  const status = old === null && cur !== null ? "新文件" : cur === null && old !== null ? "已删除" : null;

  return (
    <div className="diff-view">
      <div className="diff-header">
        <span className="diff-file">{name}</span>
        <span className="muted">{dir}</span>
        {status && <span className="diff-badge">{status}</span>}
        <span className="spacer" />
        {loading && <span className="spinner" />}
        {stats && (
          <span className="diff-stats" title={`${stats.changes} 处修改`}>
            <span className="add">+{stats.added}</span> <span className="del">−{stats.deleted}</span>
          </span>
        )}
        <span className="diff-nav">
          <button className="icon-btn" title="上一处修改（⇧F7）" onClick={() => nav.current?.go(-1)} disabled={!stats?.changes}>
            ↑
          </button>
          <button className="icon-btn" title="下一处修改（F7）" onClick={() => nav.current?.go(1)} disabled={!stats?.changes}>
            ↓
          </button>
        </span>
        <span className="seg" role="radiogroup" aria-label="Diff 布局">
          <button className={layout === "split" ? "on" : ""} onClick={() => setLayout("split")} role="radio" aria-checked={layout === "split"}>
            并排
          </button>
          <button className={layout === "unified" ? "on" : ""} onClick={() => setLayout("unified")} role="radio" aria-checked={layout === "unified"}>
            统一
          </button>
        </span>
        <span className="muted diff-label">{label}</span>
      </div>
      {body}
    </div>
  );
}

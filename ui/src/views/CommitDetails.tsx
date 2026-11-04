// Details of the commit selected in the Log: message and people, the files
// it changed (grouped by directory) and the diff of the selected file.
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { api, errorMessage } from "../api";
import type { ChangeKind } from "../bindings/ChangeKind";
import type { CommitDetails } from "../bindings/CommitDetails";
import type { CommitFile } from "../bindings/CommitFile";
import type { FileTexts } from "../bindings/FileTexts";
import { VirtualList } from "../components/VirtualList";
import { splitPath } from "../lib/format";
import { useRepo } from "../stores/repo";
import { TextsDiff } from "./DiffView";

const ROW_H = 24;
/** Wait this long after the selection settles before asking git. */
const SETTLE_MS = 60;

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

type Row = { type: "dir"; dir: string; count: number } | { type: "file"; file: CommitFile };

/** Files grouped under their directory, directories in path order. */
function groupByDir(files: CommitFile[]): Row[] {
  const byDir = new Map<string, CommitFile[]>();
  for (const f of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
    const { dir } = splitPath(f.path);
    const list = byDir.get(dir);
    if (list) list.push(f);
    else byDir.set(dir, [f]);
  }
  const rows: Row[] = [];
  for (const [dir, list] of [...byDir].sort(([a], [b]) => a.localeCompare(b))) {
    if (dir) rows.push({ type: "dir", dir, count: list.length });
    for (const file of list) rows.push({ type: "file", file });
  }
  return rows;
}

const fmtTime = (t: number) => new Date(t * 1000).toLocaleString();

export function CommitDetailsPane({ oid, refs, onJump }: { oid: string | null; refs: string[]; onJump: (oid: string) => void }) {
  const root = useRepo((s) => s.root)!;
  const toast = useRepo((s) => s.toast);
  const [details, setDetails] = useState<CommitDetails | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<FileTexts | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);

  // Load the commit; while flicking through the Log only the last one counts.
  useEffect(() => {
    if (!oid) return;
    let alive = true;
    const t = setTimeout(() => {
      api
        .commitDetails(root, oid)
        .then((d) => {
          if (!alive) return;
          setDetails(d);
          setError(null);
          setSelected((cur) => (cur && d.files.some((f) => f.path === cur) ? cur : (groupByDir(d.files).find((r) => r.type === "file") as { file: CommitFile } | undefined)?.file.path ?? null));
        })
        .catch((e) => alive && setError(errorMessage(e)));
    }, SETTLE_MS);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [root, oid]);

  const current = details && details.oid === oid ? details : null;
  const rows = useMemo(() => groupByDir(current?.files ?? []), [current]);
  const file = current?.files.find((f) => f.path === selected) ?? null;

  useEffect(() => {
    if (!current || !file) {
      setDiff(null);
      return;
    }
    let alive = true;
    setDiffLoading(true);
    api
      .commitFileTexts(root, current.oid, current.parents[0] ?? null, file.path, file.origPath)
      .then((d) => alive && setDiff(d))
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setDiffLoading(false));
    return () => {
      alive = false;
    };
  }, [root, current, file]);

  if (!oid) return <div className="cd-empty muted">选择一个提交查看详情</div>;
  if (error && !current) return <div className="cd-empty">{error}</div>;
  if (!current) {
    return (
      <div className="cd-empty muted">
        <span className="spinner inline" /> 正在读取提交…
      </div>
    );
  }

  const fileRows = rows.filter((r): r is Extract<Row, { type: "file" }> => r.type === "file");
  const fileIndex = fileRows.findIndex((r) => r.file.path === selected);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (!step || !fileRows.length) return;
    e.preventDefault();
    const next = fileRows[Math.max(0, Math.min(fileRows.length - 1, fileIndex + step))];
    setSelected(next.file.path);
  };

  const [subject, ...body] = current.message.split("\n");
  const added = current.files.reduce((n, f) => n + (f.added ?? 0), 0);
  const deleted = current.files.reduce((n, f) => n + (f.deleted ?? 0), 0);
  const sameCommitter =
    current.committerEmail === current.authorEmail && Math.abs(current.committerTime - current.authorTime) < 60;
  const copy = (text: string) =>
    void navigator.clipboard
      .writeText(text)
      .then(() => toast({ kind: "success", title: "已复制提交 hash" }))
      .catch(() => toast({ kind: "error", title: "无法访问剪贴板" }));

  return (
    <div className="cd">
      <div className="cd-left">
        <div className="cd-info">
          <div className="cd-subject">
            {refs.map((r) => (
              <span key={r} className={`ref ref-${r.startsWith("tag: ") ? "tag" : r.startsWith("HEAD") ? "head" : r.includes("/") ? "remote" : "local"}`}>
                {r.replace("HEAD -> ", "")}
              </span>
            ))}
            {subject}
          </div>
          <div className="cd-meta">
            <div>
              {current.authorName} &lt;{current.authorEmail}&gt; · {fmtTime(current.authorTime)}
            </div>
            {!sameCommitter && (
              <div className="muted">
                提交者 {current.committerName} &lt;{current.committerEmail}&gt; · {fmtTime(current.committerTime)}
              </div>
            )}
            <div className="cd-hashes">
              <button className="cd-hash mono" title="复制完整 hash" onClick={() => copy(current.oid)}>
                {current.oid.slice(0, 10)}
              </button>
              {current.parents.length > 0 && <span className="muted">{current.parents.length > 1 ? "父提交（合并）" : "父提交"}</span>}
              {current.parents.map((p) => (
                <button key={p} className="cd-parent mono" title="在 Log 中定位" onClick={() => onJump(p)}>
                  {p.slice(0, 8)}
                </button>
              ))}
            </div>
          </div>
          {body.join("\n").trim() && <pre className="cd-body">{body.join("\n").trim()}</pre>}
        </div>
        <div className="cd-files-head">
          <span>{current.files.length.toLocaleString()} 个文件</span>
          <span className="diff-stats">
            <span className="add">+{added.toLocaleString()}</span> <span className="del">−{deleted.toLocaleString()}</span>
          </span>
          {current.parents.length > 1 && <span className="muted">与第一个父提交比较</span>}
        </div>
        {rows.length === 0 ? (
          <div className="cd-empty muted">没有文件变化</div>
        ) : (
          <VirtualList
            items={rows}
            rowHeight={ROW_H}
            className="cd-files"
            tabIndex={0}
            onKeyDown={onKey}
            scrollToIndex={rows.findIndex((r) => r.type === "file" && r.file.path === selected)}
            render={(r) => {
              if (r.type === "dir") {
                return (
                  <div key={`d:${r.dir}`} className="row cd-dir" style={{ height: ROW_H }} title={r.dir}>
                    <span className="cd-dir-name">{r.dir}</span>
                    <span className="muted">{r.count}</span>
                  </div>
                );
              }
              const f = r.file;
              const { name } = splitPath(f.path);
              return (
                <div
                  key={f.path}
                  className={`row file-row cd-file ${f.path === selected ? "selected" : ""} ${splitPath(f.path).dir ? "nested" : ""}`}
                  style={{ height: ROW_H }}
                  onMouseDown={() => setSelected(f.path)}
                  title={f.origPath ? `${f.origPath} → ${f.path}` : f.path}
                >
                  <span className={`kind kind-${f.kind}`}>{KIND_LABEL[f.kind]}</span>
                  <span className={`fname kind-${f.kind}`}>{name}</span>
                  {f.origPath && <span className="fdir">← {f.origPath}</span>}
                  <span className="spacer" />
                  {f.added == null ? (
                    <span className="muted">二进制</span>
                  ) : (
                    <span className="diff-stats">
                      <span className="add">+{f.added}</span> <span className="del">−{f.deleted}</span>
                    </span>
                  )}
                </div>
              );
            }}
          />
        )}
      </div>
      <div className="cd-right">
        {file ? (
          <TextsDiff
            path={file.path}
            texts={diff && diff.path === file.path ? diff : null}
            loading={diffLoading}
            label={`${current.parents[0]?.slice(0, 8) ?? "空"} ↔ ${current.oid.slice(0, 8)}`}
          />
        ) : (
          <div className="diff-empty">选择一个文件查看差异</div>
        )}
      </div>
    </div>
  );
}

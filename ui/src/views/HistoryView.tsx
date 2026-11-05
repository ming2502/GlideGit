// History of a file (following renames), or of a few of its lines: the
// commits on the left, the selected commit's change to the file on the right.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { api, errorMessage } from "../api";
import type { FileRevision } from "../bindings/FileRevision";
import type { FileTexts } from "../bindings/FileTexts";
import { ContextMenu } from "../components/ContextMenu";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { TextsDiff } from "./DiffView";

const PAGE = 500;
const KIND: Record<string, string> = { added: "新增", deleted: "删除", renamed: "重命名", copied: "复制" };

export function HistoryView() {
  const target = useRepo((s) => s.history);
  const setHistory = useRepo((s) => s.setHistory);
  const setAnnotate = useRepo((s) => s.setAnnotate);
  const showInLog = useRepo((s) => s.showInLog);
  const root = useRepo((s) => s.root);
  const [revs, setRevs] = useState<FileRevision[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState(0);
  const [texts, setTexts] = useState<FileTexts | null>(null);
  const [loadingDiff, setLoadingDiff] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; rev: FileRevision } | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const loaded = revs != null;
  useEffect(() => {
    if (loaded) list.current?.focus();
  }, [loaded]);

  // Load the revisions. Line history comes as plain commits; the file's own
  // history tells which path each of them had (renames).
  useEffect(() => {
    if (!target || !root) return;
    let alive = true;
    setRevs(null);
    setError(null);
    setSel(0);
    const load = async () => {
      const file = await api.fileHistory(root, target.path, 0, target.range ? 100_000 : PAGE);
      if (!target.range) {
        setMore(file.length === PAGE);
        return file;
      }
      const [start, end] = target.range;
      const commits = await api.lineHistory(root, target.path, start, end, target.rev ?? null);
      const byOid = new Map(file.map((r) => [r.commit.oid, r]));
      setMore(false);
      return commits.map(
        (c): FileRevision => byOid.get(c.oid) ?? { commit: c, path: target.path, origPath: null, kind: "modified" },
      );
    };
    load()
      .then((r) => alive && setRevs(r))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [target, root]);

  const current = revs?.[sel] ?? null;
  useEffect(() => {
    if (!current || !root) {
      setTexts(null);
      return;
    }
    let alive = true;
    setLoadingDiff(true);
    api
      .commitFileTexts(root, current.commit.oid, current.commit.parents[0] ?? null, current.path, current.origPath)
      .then((t) => alive && setTexts(t))
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoadingDiff(false));
    return () => {
      alive = false;
    };
  }, [current, root]);

  const authors = useMemo(() => new Set(revs?.map((r) => r.commit.authorEmail)).size, [revs]);
  if (!target) return null;
  const close = () => setHistory(null);
  const loadMore = () => {
    if (!root || !revs) return;
    void api
      .fileHistory(root, target.path, revs.length, PAGE)
      .then((r) => {
        setRevs([...revs, ...r]);
        setMore(r.length === PAGE);
      })
      .catch((e) => setError(errorMessage(e)));
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!revs?.length) return;
    if (e.key === "ArrowDown") setSel((i) => Math.min(revs.length - 1, i + 1));
    else if (e.key === "ArrowUp") setSel((i) => Math.max(0, i - 1));
    else if (e.key === "Enter" && current) showInLog(current.commit.oid);
    else return;
    e.preventDefault();
  };

  const title = target.range
    ? `选区历史 ${target.path} 第 ${target.range[0]}–${target.range[1]} 行${target.rev ? `（@ ${target.rev.slice(0, 8)}）` : ""}`
    : `文件历史 ${target.path}`;

  return (
    <div className="mt-overlay" role="dialog" aria-modal="true" aria-label={title} onKeyDown={(e) => e.key === "Escape" && close()}>
      <div className="mt-window">
        <div className="mt-header">
          <b>{title}</b>
          {revs && (
            <span className="muted">
              {revs.length}
              {more ? "+" : ""} 个提交，{authors} 位作者
            </span>
          )}
          <span className="spacer" />
          <button className="btn btn-sm" onClick={close}>
            关闭
          </button>
        </div>
        {error && !revs ? (
          <div className="diff-empty">{error}</div>
        ) : !revs ? (
          <div className="diff-empty">
            <span className="spinner" /> 正在查找修改过{target.range ? "这些行" : "这个文件"}的提交…
          </div>
        ) : revs.length === 0 ? (
          <div className="diff-empty">没有找到提交{target.range ? "（这些行可能还没有提交过）" : ""}</div>
        ) : (
          <div className="hv-body">
            <div className="hv-list" tabIndex={0} onKeyDown={onKey} ref={list}>
              {revs.map((r, i) => (
                <div
                  key={r.commit.oid}
                  className={`hv-row ${i === sel ? "selected" : ""}`}
                  onMouseDown={() => setSel(i)}
                  onDoubleClick={() => showInLog(r.commit.oid)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setSel(i);
                    setMenu({ x: e.clientX, y: e.clientY, rev: r });
                  }}
                  title="双击或回车在 Log 中显示"
                >
                  <div className="hv-subject">{r.commit.subject}</div>
                  <div className="hv-meta muted">
                    <span className="mono">{r.commit.oid.slice(0, 7)}</span>
                    <span>{r.commit.authorName}</span>
                    <span>{relativeTime(r.commit.authorTime)}</span>
                    {r.kind && KIND[r.kind] && <span className="hv-kind">{KIND[r.kind]}</span>}
                    {r.path !== target.path && <span className="hv-path" title={r.path}>{r.path}</span>}
                  </div>
                </div>
              ))}
              {more && (
                <button className="btn btn-sm hv-more" onClick={loadMore}>
                  加载更多
                </button>
              )}
            </div>
            <div className="hv-diff">
              {current && (
                <TextsDiff
                  path={current.path}
                  texts={texts && texts.path === current.path ? texts : null}
                  loading={loadingDiff}
                  label={`${current.commit.parents[0]?.slice(0, 8) ?? "空"} ↔ ${current.commit.oid.slice(0, 8)}`}
                />
              )}
            </div>
          </div>
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "在 Log 中显示", onClick: () => showInLog(menu.rev.commit.oid) },
            {
              label: "注释此版本",
              disabled: menu.rev.kind === "deleted",
              onClick: () => {
                setHistory(null);
                setAnnotate({ path: menu.rev.path, rev: menu.rev.commit.oid });
              },
            },
            "separator",
            { label: "复制提交 hash", onClick: () => void navigator.clipboard.writeText(menu.rev.commit.oid) },
          ]}
        />
      )}
    </div>
  );
}

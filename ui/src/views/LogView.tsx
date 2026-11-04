import { useEffect, useState, type KeyboardEvent } from "react";
import { api, errorMessage } from "../api";
import type { CommitSummary } from "../bindings/CommitSummary";
import { VirtualList } from "../components/VirtualList";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";

const ROW_H = 26;
const LIMIT = 2000;

function RefBadge({ name }: { name: string }) {
  const cls = name.startsWith("tag: ")
    ? "tag"
    : name.startsWith("HEAD")
      ? "head"
      : name.includes("/")
        ? "remote"
        : "local";
  return <span className={`ref ref-${cls}`}>{name.replace("HEAD -> ", "")}</span>;
}

export function LogView() {
  const root = useRepo((s) => s.root)!;
  const version = useRepo((s) => s.logVersion);
  const [commits, setCommits] = useState<CommitSummary[]>([]);
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .log(root, LIMIT)
      .then((c) => {
        if (!alive) return;
        setCommits(c);
        setError(null);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [root, version]);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "ArrowDown") setSelected((i) => Math.min(commits.length - 1, i + 1));
    else if (e.key === "ArrowUp") setSelected((i) => Math.max(0, i - 1));
    else return;
    e.preventDefault();
  };

  const cur = commits[selected];

  return (
    <div className="log-view">
      <div className="log-head row">
        <span className="c-subject">Subject</span>
        <span className="c-author">Author</span>
        <span className="c-date">Date</span>
        <span className="c-hash">Hash</span>
      </div>
      {error ? (
        <div className="empty">{error}</div>
      ) : commits.length === 0 ? (
        <div className="empty">还没有提交</div>
      ) : (
        <VirtualList
          items={commits}
          rowHeight={ROW_H}
          tabIndex={0}
          className="log-list"
          scrollToIndex={selected}
          onKeyDown={onKey}
          render={(c, i) => (
            <div
              key={c.oid}
              className={`row log-row ${i === selected ? "selected" : ""}`}
              style={{ height: ROW_H }}
              onMouseDown={() => setSelected(i)}
            >
              <span className="c-subject">
                {c.refs.map((r) => (
                  <RefBadge key={r} name={r} />
                ))}
                <span className="subject-text">{c.subject}</span>
              </span>
              <span className="c-author">{c.authorName}</span>
              <span className="c-date">{relativeTime(c.authorTime)}</span>
              <span className="c-hash mono">{c.oid.slice(0, 8)}</span>
            </div>
          )}
        />
      )}
      {cur && (
        <div className="commit-details">
          <div className="mono">{cur.oid}</div>
          <div>
            {cur.authorName} &lt;{cur.authorEmail}&gt; · {new Date(cur.authorTime * 1000).toLocaleString()}
          </div>
          <div className="muted">
            {cur.parents.length > 1 ? "Merge commit · " : ""}
            parents: {cur.parents.map((p) => p.slice(0, 8)).join(", ") || "—"}
          </div>
          <div className="details-subject">{cur.subject}</div>
        </div>
      )}
      {commits.length >= LIMIT && <div className="diff-trunc">仅显示最近 {LIMIT} 个提交（完整 Log 索引见 M2）</div>}
    </div>
  );
}

import { useCallback, useEffect, useReducer, useRef, useState, type KeyboardEvent } from "react";
import { api, errorMessage, onLogStatus } from "../api";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { GraphRow } from "../bindings/GraphRow";
import type { LogStatus } from "../bindings/LogStatus";
import { VirtualRows } from "../components/VirtualList";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { LogGraph, graphPx, graphWidth } from "./LogGraph";

const ROW_H = 26;
/** Rows per request to the commit index. */
const PAGE = 200;
/** Rows loaded beyond the screen, so scrolling rarely shows blank rows. */
const PREFETCH = 150;

interface Page {
  version: number;
  rows: CommitSummary[];
  graph: GraphRow[];
}
/** Left padding of a row, where the graph column starts. */
const ROW_PAD = 8;

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

/** Status of the commit index for `root`, kept current from backend events. */
function useLogStatus(root: string): LogStatus | null {
  const [status, setStatus] = useState<LogStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const unlisten = onLogStatus((s) => {
      if (alive && s.root === root) setStatus(s);
    });
    api
      .logStatus(root)
      .then((s) => alive && setStatus((cur) => cur ?? s))
      .catch(() => {});
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, [root]);
  return status;
}

/**
 * Rows of the Log, read page by page around what is on screen. Pages read
 * under an older index version stay visible until their replacement arrives.
 */
function useLogRows(root: string, status: LogStatus | null, range: [number, number]) {
  const pages = useRef(new Map<number, Page>());
  const inflight = useRef(new Set<number>());
  const [tick, pageArrived] = useReducer((x: number) => x + 1, 0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!status || status.total === 0) return;
    const first = Math.floor(Math.max(0, range[0] - PREFETCH) / PAGE);
    const last = Math.floor(Math.min(status.total - 1, range[1] + PREFETCH) / PAGE);
    for (let p = first; p <= last; p++) {
      const have = pages.current.get(p);
      const want = Math.min(PAGE, status.total - p * PAGE);
      if (have && have.version >= status.version && have.rows.length >= want) continue;
      if (inflight.current.has(p)) continue;
      inflight.current.add(p);
      api
        .logPage(root, p * PAGE, PAGE)
        .then((page) => {
          pages.current.set(p, { version: page.version, rows: page.rows, graph: page.graph });
          setError(null);
          pageArrived();
        })
        .catch((e) => setError(errorMessage(e)))
        .finally(() => inflight.current.delete(p));
    }
    // `tick`: re-check after each arrival, in case the version moved meanwhile.
  }, [root, status, range, tick]);

  const row = useCallback(
    (i: number): CommitSummary | undefined => pages.current.get(Math.floor(i / PAGE))?.rows[i % PAGE],
    [tick],
  );
  const graph = useCallback(
    (i: number): GraphRow | undefined => pages.current.get(Math.floor(i / PAGE))?.graph[i % PAGE],
    [tick],
  );
  return { row, graph, tick, error };
}

export function LogView() {
  const root = useRepo((s) => s.root)!;
  const status = useLogStatus(root);
  const [range, setRange] = useState<[number, number]>([0, 40]);
  const onRangeChange = useCallback((s: number, e: number) => setRange([s, e]), []);
  const { row, graph, tick, error } = useLogRows(root, status, range);
  const total = status?.total ?? 0;
  const isMerge = useCallback((i: number) => (row(i)?.parents.length ?? 0) > 1, [row]);

  // Graph column wide enough for the rows on screen.
  let lanes = 1;
  for (let i = range[0]; i < range[1]; i++) {
    const g = graph(i);
    if (g) lanes = Math.max(lanes, graphWidth(g));
  }
  const graphStyle = { width: graphPx(lanes) };

  const [selected, setSelected] = useState(0);
  // The selected commit, so the selection can follow it when rows move.
  const selectedOid = useRef<string | null>(null);
  const select = (i: number) => {
    const n = Math.max(0, Math.min(total - 1, i));
    setSelected(n);
    selectedOid.current = row(n)?.oid ?? null;
  };
  const cur = row(selected);
  if (cur && selectedOid.current == null) selectedOid.current = cur.oid;

  const version = status?.version;
  useEffect(() => {
    const oid = selectedOid.current;
    if (!oid || version == null) return;
    let alive = true;
    api
      .logRowOf(root, oid)
      .then((r) => {
        if (alive && r != null) setSelected(r);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [root, version]);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const pageRows = Math.max(1, range[1] - range[0] - 1);
    if (e.key === "ArrowDown") select(selected + 1);
    else if (e.key === "ArrowUp") select(selected - 1);
    else if (e.key === "PageDown") select(selected + pageRows);
    else if (e.key === "PageUp") select(selected - pageRows);
    else if (e.key === "Home") select(0);
    else if (e.key === "End") select(total - 1);
    else return;
    e.preventDefault();
  };

  const indexing = status != null && !status.complete;

  return (
    <div className="log-view">
      <div className="log-head row">
        <span className="c-subject">Subject</span>
        <span className="c-author">Author</span>
        <span className="c-date">Date</span>
        <span className="c-hash">Hash</span>
      </div>
      {error && total === 0 ? (
        <div className="empty">{error}</div>
      ) : total === 0 ? (
        <div className="empty">{status?.complete ? "还没有提交" : "正在读取提交历史…"}</div>
      ) : (
        <VirtualRows
          count={total}
          rowHeight={ROW_H}
          tabIndex={0}
          className="log-list"
          scrollToIndex={selected}
          onKeyDown={onKey}
          onRangeChange={onRangeChange}
          overlay={(vp) => (
            <LogGraph
              viewport={vp}
              rowHeight={ROW_H}
              lanes={lanes}
              graph={graph}
              isMerge={isMerge}
              dataVersion={tick}
              left={ROW_PAD}
            />
          )}
          renderRow={(i) => {
            const c = row(i);
            if (!c) {
              return (
                <div key={`pending-${i}`} className="row log-row pending" style={{ height: ROW_H }}>
                  <span className="c-graph" style={graphStyle} />
                  <span className="c-subject">
                    <span className="placeholder" />
                  </span>
                </div>
              );
            }
            return (
              <div
                key={c.oid}
                className={`row log-row ${i === selected ? "selected" : ""}`}
                style={{ height: ROW_H }}
                onMouseDown={() => select(i)}
              >
                <span className="c-graph" style={graphStyle} />
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
            );
          }}
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
      <div className="log-foot">
        <span>{total.toLocaleString()} 个提交</span>
        {indexing && (
          <span className="log-indexing">
            <span className="spinner" /> 正在建立提交索引…
          </span>
        )}
        {status?.error && <span className="log-error" title={status.error}>索引更新失败：{status.error}</span>}
        {error && total > 0 && <span className="log-error">{error}</span>}
      </div>
    </div>
  );
}

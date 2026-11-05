import { useCallback, useEffect, useReducer, useRef, useState, type KeyboardEvent } from "react";
import { api, errorMessage, onLogStatus } from "../api";
import type { CommitSummary } from "../bindings/CommitSummary";
import type { GraphRow } from "../bindings/GraphRow";
import type { LogFilter } from "../bindings/LogFilter";
import type { LogStatus } from "../bindings/LogStatus";
import { VirtualRows } from "../components/VirtualList";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { load, save } from "../lib/storage";
import { CommitDetailsPane } from "./CommitDetails";
import { EMPTY_FILTER, LogFilterBar, filterKey } from "./LogFilterBar";
import { LogGraph, graphPx, graphWidth } from "./LogGraph";
import { ContextMenu, type MenuItem } from "../components/ContextMenu";
import type { ResetMode } from "../bindings/ResetMode";
import { NewRefDialog, ResetDialog } from "./LogDialogs";

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

interface RowCache {
  /** Filter the pages belong to ("" = unfiltered). */
  key: string;
  pages: Map<number, Page>;
  inflight: Set<number>;
  /** Rows matching the filter; null until the first page answers. */
  total: number | null;
  /** The filtered rows have no graph (a filter left holes in the history). */
  noGraph: boolean;
}

const newCache = (key: string): RowCache => ({ key, pages: new Map(), inflight: new Set(), total: null, noGraph: false });

/**
 * Rows of the Log, read page by page around what is on screen. Pages read
 * under an older index version stay visible until their replacement arrives.
 * A filter gets its own set of pages; its row count comes with each page.
 */
function useLogRows(root: string, status: LogStatus | null, range: [number, number], filter: LogFilter) {
  const key = filterKey(filter);
  const cache = useRef<RowCache>(newCache(key));
  if (cache.current.key !== key) cache.current = newCache(key);
  const c = cache.current;
  const [tick, pageArrived] = useReducer((x: number) => x + 1, 0);
  const [error, setError] = useState<string | null>(null);
  const filterRef = useRef(filter);
  filterRef.current = filter;

  const total = key === "" ? (status?.total ?? 0) : c.total;

  useEffect(() => {
    if (!status) return;
    const c = cache.current;
    // Unknown filtered count: the first page tells.
    const n = c.key === "" ? status.total : (c.total ?? 1);
    if (n === 0) return;
    const first = Math.floor(Math.max(0, range[0] - PREFETCH) / PAGE);
    const last = Math.floor(Math.min(n - 1, range[1] + PREFETCH) / PAGE);
    for (let p = Math.min(first, last); p <= last; p++) {
      const have = c.pages.get(p);
      const want = Math.min(PAGE, n - p * PAGE);
      if (have && have.version >= status.version && have.rows.length >= want) continue;
      if (c.inflight.has(p)) continue;
      c.inflight.add(p);
      api
        .logPage(root, p * PAGE, PAGE, c.key ? filterRef.current : null)
        .then((page) => {
          if (cache.current !== c) return; // the filter changed meanwhile
          c.pages.set(p, { version: page.version, rows: page.rows, graph: page.graph });
          if (c.key) {
            c.total = page.total;
            c.noGraph = page.rows.length > 0 && page.graph.length === 0;
          }
          setError(null);
          pageArrived();
        })
        .catch((e) => {
          if (cache.current === c && (e as { kind?: string })?.kind !== "cancelled") setError(errorMessage(e));
        })
        .finally(() => c.inflight.delete(p));
    }
    // `tick`: re-check after each arrival, in case the version moved meanwhile.
  }, [root, status, range, tick, key]);

  const row = useCallback(
    (i: number): CommitSummary | undefined => cache.current.pages.get(Math.floor(i / PAGE))?.rows[i % PAGE],
    [tick, key],
  );
  const graph = useCallback(
    (i: number): GraphRow | undefined => cache.current.pages.get(Math.floor(i / PAGE))?.graph[i % PAGE],
    [tick, key],
  );
  return { row, graph, tick, error, total, noGraph: c.noGraph, filtering: key !== "" && c.total === null };
}

export function LogView() {
  const root = useRepo((s) => s.root)!;
  const status = useLogStatus(root);
  const [range, setRange] = useState<[number, number]>([0, 40]);
  const onRangeChange = useCallback((s: number, e: number) => setRange([s, e]), []);
  const [filter, setFilter] = useState<LogFilter>(EMPTY_FILTER);
  const filtered = filterKey(filter) !== "";
  const { row, graph, tick, error, total: rowsTotal, noGraph, filtering } = useLogRows(root, status, range, filter);
  const total = rowsTotal ?? 0;
  const isMerge = useCallback((i: number) => (row(i)?.parents.length ?? 0) > 1, [row]);

  // Graph column wide enough for the rows on screen.
  let lanes = 1;
  for (let i = range[0]; i < range[1]; i++) {
    const g = graph(i);
    if (g) lanes = Math.max(lanes, graphWidth(g));
  }
  const graphStyle = { width: noGraph ? 0 : graphPx(lanes) };

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

  // Follow the selected commit when rows move or the filter changes; if the
  // filter hides it, select the first row.
  const version = status?.version;
  const key = filterKey(filter);
  const lastKey = useRef(key);
  useEffect(() => {
    const filterChanged = lastKey.current !== key;
    lastKey.current = key;
    const oid = selectedOid.current;
    if (!oid || version == null) return;
    let alive = true;
    api
      .logRowOf(root, oid, key ? filter : null)
      .then((r) => {
        if (!alive) return;
        if (r != null) setSelected(r);
        else if (filterChanged) {
          setSelected(0);
          selectedOid.current = null;
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [root, version, key]);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const pageRows = Math.max(1, range[1] - range[0] - 1);
    if (e.key === "ArrowDown") select(selected + 1);
    else if (e.key === "ArrowUp") select(selected - 1);
    else if (e.key === "PageDown") select(selected + pageRows);
    else if (e.key === "PageUp") select(selected - pageRows);
    else if (e.key === "Home") select(0);
    else if (e.key === "End") select(total - 1);
    else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      const c = row(selected);
      const el = (e.currentTarget as HTMLElement).querySelector(".log-row.selected");
      const r = el?.getBoundingClientRect();
      if (c && r) setMenu({ x: r.left + 40, y: r.bottom, commit: c });
    } else return;
    e.preventDefault();
  };

  // ---------------------------------------------------------------- actions
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const [menu, setMenu] = useState<{ x: number; y: number; commit: CommitSummary } | null>(null);
  const [dialog, setDialog] = useState<{ kind: "branch" | "tag" | "reset"; commit: CommitSummary } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  const currentBranch = snapshot?.head.branch ?? null;
  const localBranches = new Set(snapshot?.branches.local.map((b) => b.name) ?? []);
  const short = (c: CommitSummary) => c.oid.slice(0, 8);

  const menuItems = (c: CommitSummary): (MenuItem | "separator")[] => {
    // Branches pointing at this commit can be checked out by name.
    const names = c.refs.map((r) => r.replace(/^HEAD -> /, "")).filter((r) => !r.startsWith("tag: ") && r !== "HEAD");
    const checkouts: MenuItem[] = names
      .filter((n) => n !== currentBranch)
      .map((n) =>
        localBranches.has(n)
          ? { label: `Checkout ${n}`, onClick: () => void runTask(`切换到 ${n}`, (r) => api.checkoutBranch(r, n, false)) }
          : { label: `Checkout ${n}（新建跟踪分支）`, onClick: () => void runTask(`切换到 ${n}`, (r) => api.checkoutBranch(r, n, true)) },
      );
    return [
      ...checkouts,
      {
        label: "Checkout 此修订（游离 HEAD）",
        onClick: async () => {
          const ok = await ask({
            title: "切换到游离 HEAD？",
            message: `将检出 ${short(c)}，不在任何分支上。\n在此状态下的新提交需要新建分支才能保留。`,
            confirmLabel: "Checkout",
          });
          if (ok) void runTask(`检出 ${short(c)}`, (r) => api.checkoutDetached(r, c.oid));
        },
      },
      { label: "新建分支…", onClick: () => setDialog({ kind: "branch", commit: c }) },
      { label: "新建标签…", onClick: () => setDialog({ kind: "tag", commit: c }) },
      "separator",
      {
        label: "Cherry-pick",
        disabled: !!snapshot?.operation,
        onClick: () => void runTask(`Cherry-pick ${short(c)}`, (r) => api.cherryPick(r, c.oid), `已 cherry-pick ${short(c)}`),
      },
      {
        label: "Revert 提交",
        disabled: !!snapshot?.operation,
        onClick: () => void runTask(`Revert ${short(c)}`, (r) => api.revert(r, c.oid), `已 revert ${short(c)}`),
      },
      "separator",
      {
        label: "从此提交开始交互式变基…",
        disabled: !!snapshot?.operation,
        onClick: () => useRepo.getState().setRebaseFrom(c),
      },
      {
        label: currentBranch ? `将 ${currentBranch} 重置到此处…` : "将 HEAD 重置到此处…",
        disabled: !!snapshot?.operation,
        onClick: () => setDialog({ kind: "reset", commit: c }),
      },
      "separator",
      {
        label: "复制提交 hash",
        onClick: () =>
          void navigator.clipboard
            .writeText(c.oid)
            .then(() => useRepo.getState().toast({ kind: "success", title: "已复制提交 hash" })),
      },
    ];
  };

  const doReset = async (c: CommitSummary, mode: ResetMode) => {
    setDialog(null);
    if (mode === "hard") {
      const ok = await ask({
        title: "确定执行 Hard 重置？",
        message: `${currentBranch ?? "HEAD"} 将指向 ${short(c)}，工作区和暂存区中的所有修改都会被丢弃，无法恢复。`,
        confirmLabel: "丢弃修改并重置",
        danger: true,
      });
      if (!ok) return;
    }
    void runTask(`重置到 ${short(c)}`, (r) => api.reset(r, c.oid, mode), `已重置到 ${short(c)}`);
  };

  const indexing = status != null && !status.complete;

  // Jump to a commit (e.g. a parent link); it may be hidden by the filter.
  const jumpTo = (oid: string) => {
    void api
      .logRowOf(root, oid, key ? filter : null)
      .then((r) => {
        if (r == null) {
          useRepo.getState().toast({ kind: "info", title: "该提交不在当前列表中", detail: "可能被过滤条件隐藏" });
          return;
        }
        setSelected(r);
        selectedOid.current = oid;
      })
      .catch(() => {});
  };

  // Height of the details pane, dragged with the splitter above it.
  const [detailsH, setDetailsH] = useState(() => load("logDetailsHeight", 300));
  useEffect(() => save("logDetailsHeight", detailsH), [detailsH]);
  const startResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = detailsH;
    const view = (e.currentTarget as HTMLElement).parentElement!;
    const max = Math.max(160, view.clientHeight - 160);
    const onMove = (ev: MouseEvent) => setDetailsH(Math.max(140, Math.min(max, startH + startY - ev.clientY)));
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.classList.remove("resizing-v");
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.classList.add("resizing-v");
  };

  return (
    <div className="log-view">
      <LogFilterBar filter={filter} onChange={setFilter} />
      <div className="log-head row">
        <span className="c-subject">Subject</span>
        <span className="c-author">Author</span>
        <span className="c-date">Date</span>
        <span className="c-hash">Hash</span>
      </div>
      {error && total === 0 ? (
        <div className="empty">{error}</div>
      ) : filtering ? (
        <div className="empty">
          <span className="spinner inline" /> 正在过滤…
        </div>
      ) : total === 0 ? (
        <div className="empty">
          {filtered ? "没有符合过滤条件的提交" : status?.complete ? "还没有提交" : "正在读取提交历史…"}
        </div>
      ) : (
        <VirtualRows
          count={total}
          rowHeight={ROW_H}
          tabIndex={0}
          className="log-list"
          scrollToIndex={selected}
          onKeyDown={onKey}
          onRangeChange={onRangeChange}
          overlay={
            noGraph
              ? undefined
              : (vp) => (
            <LogGraph
              viewport={vp}
              rowHeight={ROW_H}
              lanes={lanes}
              graph={graph}
              isMerge={isMerge}
              dataVersion={tick}
              left={ROW_PAD}
            />
          )
          }
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
                onContextMenu={(e) => {
                  e.preventDefault();
                  select(i);
                  setMenu({ x: e.clientX, y: e.clientY, commit: c });
                }}
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
      {total > 0 && (
        <>
          <div className="log-split" onMouseDown={startResize} title="拖动调整高度" />
          <div className="log-details" style={{ height: detailsH }}>
            <CommitDetailsPane oid={cur?.oid ?? null} refs={cur?.refs ?? []} onJump={jumpTo} />
          </div>
        </>
      )}
      <div className="log-foot">
        <span>
          {filtered
            ? `${total.toLocaleString()} 个匹配提交（共 ${(status?.total ?? 0).toLocaleString()} 个）`
            : `${total.toLocaleString()} 个提交`}
        </span>
        {indexing && (
          <span className="log-indexing">
            <span className="spinner" /> 正在建立提交索引…
          </span>
        )}
        {status?.error && <span className="log-error" title={status.error}>索引更新失败：{status.error}</span>}
        {error && total > 0 && <span className="log-error">{error}</span>}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.commit)} onClose={closeMenu} />}
      {dialog && dialog.kind !== "reset" && (
        <NewRefDialog
          kind={dialog.kind}
          commit={dialog.commit}
          onCancel={() => setDialog(null)}
          onSubmit={(name, { checkout, message }) => {
            const c = dialog.commit;
            setDialog(null);
            if (dialog.kind === "branch")
              void runTask(`新建分支 ${name}`, (r) => api.createBranch(r, name, c.oid, checkout), `已创建分支 ${name}`);
            else void runTask(`新建标签 ${name}`, (r) => api.createTag(r, name, c.oid, message || null), `已创建标签 ${name}`);
          }}
        />
      )}
      {dialog?.kind === "reset" && (
        <ResetDialog
          commit={dialog.commit}
          branch={currentBranch}
          onCancel={() => setDialog(null)}
          onSubmit={(mode) => void doReset(dialog.commit, mode)}
        />
      )}
    </div>
  );
}

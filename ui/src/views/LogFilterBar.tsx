// Filter bar above the Log: text/hash search plus branch, author, date and
// path filters, each in a small dropdown. Filters combine with AND.
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "../api";
import { defineActions, useActions } from "../actions/registry";
import type { AuthorCount } from "../bindings/AuthorCount";
import type { LogFilter } from "../bindings/LogFilter";
import { useRepo } from "../stores/repo";

/** This view's actions (declared up front so the shortcut settings list them). */
const LOG_FILTER_ACTIONS = defineActions([
  { id: "log.find", title: "Find in Log", keys: ["mod+f"], keywords: "在 Log 中搜索 查找提交 search" },
] as const);

export const EMPTY_FILTER: LogFilter = { text: "", branches: [], authors: [], since: null, until: null, paths: [] };

export function isFilterEmpty(f: LogFilter): boolean {
  return !f.text.trim() && !f.branches.length && !f.authors.length && f.since == null && f.until == null && !f.paths.length;
}

/** Stable key for caching pages per filter ("" when unfiltered). */
export function filterKey(f: LogFilter): string {
  return isFilterEmpty(f) ? "" : JSON.stringify({ ...f, text: f.text.trim() });
}

/** Button that opens a panel below it; closes on outside click or Escape. */
function Dropdown({
  label,
  active,
  children,
  width = 280,
}: {
  label: string;
  active: boolean;
  children: (close: () => void) => ReactNode;
  width?: number;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  return (
    <div className="lf-dd" ref={ref}>
      <button className={`lf-chip ${active ? "active" : ""} ${open ? "open" : ""}`} onClick={() => setOpen((o) => !o)}>
        {label}
        <span className="lf-caret">▾</span>
      </button>
      {open && (
        // aria-modal pauses global shortcuts (⌘↩, ⌘F…) while the panel is open.
        <div className="lf-panel" style={{ width }} role="dialog" aria-modal="true" aria-label={label}>
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

function summary(name: string, values: string[]): string {
  if (!values.length) return name;
  return values.length === 1 ? `${name}: ${values[0]}` : `${name}: ${values[0]} 等 ${values.length} 项`;
}

const shortRef = (r: string) => r.replace(/^refs\/(heads|remotes)\//, "");

function BranchPanel({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const branches = useRepo((s) => s.snapshot?.branches);
  const [q, setQ] = useState("");
  const items = useMemo(() => {
    const all = [
      { ref: "HEAD", name: "HEAD（当前分支）" },
      ...(branches?.local ?? []).map((b) => ({ ref: b.fullRef, name: b.name })),
      ...(branches?.remote ?? []).map((b) => ({ ref: b.fullRef, name: b.name })),
    ];
    const needle = q.trim().toLowerCase();
    return needle ? all.filter((b) => b.name.toLowerCase().includes(needle)) : all;
  }, [branches, q]);
  const toggle = (ref: string) => onChange(value.includes(ref) ? value.filter((r) => r !== ref) : [...value, ref]);
  return (
    <>
      <input className="lf-input" autoFocus placeholder="搜索分支" value={q} onChange={(e) => setQ(e.target.value)} />
      <div className="lf-list">
        {items.map((b) => (
          <label key={b.ref} className="lf-item">
            <input type="checkbox" checked={value.includes(b.ref)} onChange={() => toggle(b.ref)} />
            <span className="lf-item-text">{b.name}</span>
          </label>
        ))}
        {!items.length && <div className="lf-none">没有匹配的分支</div>}
      </div>
    </>
  );
}

function AuthorPanel({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const root = useRepo((s) => s.root)!;
  const [q, setQ] = useState("");
  const [authors, setAuthors] = useState<AuthorCount[]>([]);
  useEffect(() => {
    let alive = true;
    const t = setTimeout(() => {
      api
        .logAuthors(root, q)
        .then((a) => alive && setAuthors(a))
        .catch(() => {});
    }, 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [root, q]);
  const toggle = (a: string) => onChange(value.includes(a) ? value.filter((x) => x !== a) : [...value, a]);
  return (
    <>
      <input
        className="lf-input"
        autoFocus
        placeholder="姓名或邮箱，回车按输入内容过滤"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && q.trim() && !value.includes(q.trim())) {
            onChange([...value, q.trim()]);
            setQ("");
          }
        }}
      />
      <div className="lf-list">
        {value
          .filter((v) => !authors.some((a) => a.email === v))
          .map((v) => (
            <label key={v} className="lf-item">
              <input type="checkbox" checked onChange={() => toggle(v)} />
              <span className="lf-item-text">“{v}”</span>
            </label>
          ))}
        {authors.map((a) => (
          <label key={`${a.name}<${a.email}>`} className="lf-item">
            <input type="checkbox" checked={value.includes(a.email)} onChange={() => toggle(a.email)} />
            <span className="lf-item-text">
              {a.name} <span className="muted">{a.email}</span>
            </span>
            <span className="lf-count">{a.commits.toLocaleString()}</span>
          </label>
        ))}
      </div>
    </>
  );
}

const DAY = 86400;
const DATE_PRESETS: [string, number][] = [
  ["最近 24 小时", DAY],
  ["最近 7 天", 7 * DAY],
  ["最近 30 天", 30 * DAY],
  ["最近一年", 365 * DAY],
];

const toDateInput = (t: number | null) => (t == null ? "" : new Date(t * 1000).toISOString().slice(0, 10));
const fromDateInput = (s: string, endOfDay: boolean) =>
  s ? Math.floor(new Date(`${s}T${endOfDay ? "23:59:59" : "00:00:00"}`).getTime() / 1000) : null;

function DatePanel({
  since,
  until,
  onChange,
  close,
}: {
  since: number | null;
  until: number | null;
  onChange: (since: number | null, until: number | null) => void;
  close: () => void;
}) {
  const now = Math.floor(Date.now() / 1000);
  return (
    <div className="lf-dates">
      <button className="lf-option" onClick={() => (onChange(null, null), close())}>
        不限
      </button>
      {DATE_PRESETS.map(([label, secs]) => (
        <button key={label} className="lf-option" onClick={() => (onChange(now - secs, null), close())}>
          {label}
        </button>
      ))}
      <div className="lf-range">
        <label>
          从 <input type="date" value={toDateInput(since)} onChange={(e) => onChange(fromDateInput(e.target.value, false), until)} />
        </label>
        <label>
          到 <input type="date" value={toDateInput(until)} onChange={(e) => onChange(since, fromDateInput(e.target.value, true))} />
        </label>
      </div>
    </div>
  );
}

function PathPanel({ value, onChange, close }: { value: string[]; onChange: (v: string[]) => void; close: () => void }) {
  const [text, setText] = useState(value.join("\n"));
  const apply = () => {
    onChange(
      text
        .split(/\n/)
        .map((p) => p.trim().replace(/^\.?\//, ""))
        .filter(Boolean),
    );
    close();
  };
  return (
    <>
      <textarea
        className="lf-input lf-paths"
        autoFocus
        rows={4}
        placeholder={"每行一个文件或目录，相对仓库根目录\n例如 src/main.rs"}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) apply();
        }}
      />
      <div className="lf-actions">
        <button className="btn" onClick={() => (onChange([]), close())}>
          清除
        </button>
        <button className="btn btn-primary" onClick={apply}>
          应用
        </button>
      </div>
    </>
  );
}

function dateSummary(since: number | null, until: number | null): string {
  if (since == null && until == null) return "日期";
  const now = Math.floor(Date.now() / 1000);
  if (until == null && since != null) {
    const preset = DATE_PRESETS.find(([, secs]) => Math.abs(now - secs - since) < 3600);
    if (preset) return `日期: ${preset[0]}`;
  }
  return `日期: ${toDateInput(since) || "…"} ~ ${toDateInput(until) || "今天"}`;
}

export function LogFilterBar({ filter, onChange }: { filter: LogFilter; onChange: (f: LogFilter) => void }) {
  const [text, setText] = useState(filter.text);
  const input = useRef<HTMLInputElement>(null);
  // Debounce typing; other controls apply at once.
  useEffect(() => {
    if (text === filter.text) return;
    const t = setTimeout(() => onChange({ ...filter, text }), 250);
    return () => clearTimeout(t);
  }, [text, filter, onChange]);
  useEffect(() => setText(filter.text), [filter.text]);

  useActions([
    {
      ...LOG_FILTER_ACTIONS["log.find"],
      run: () => {
        input.current?.focus();
        input.current?.select();
      },
    },
  ]);

  const set = (patch: Partial<LogFilter>) => onChange({ ...filter, ...patch });

  return (
    <div className="log-filter">
      <input
        ref={input}
        className="lf-search"
        placeholder="搜索提交信息或 hash（⌘F）"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && text) {
            e.stopPropagation();
            setText("");
            onChange({ ...filter, text: "" });
          }
        }}
      />
      <Dropdown label={summary("分支", filter.branches.map(shortRef))} active={filter.branches.length > 0}>
        {() => <BranchPanel value={filter.branches} onChange={(branches) => set({ branches })} />}
      </Dropdown>
      <Dropdown label={summary("作者", filter.authors)} active={filter.authors.length > 0} width={340}>
        {() => <AuthorPanel value={filter.authors} onChange={(authors) => set({ authors })} />}
      </Dropdown>
      <Dropdown label={dateSummary(filter.since, filter.until)} active={filter.since != null || filter.until != null} width={240}>
        {(close) => (
          <DatePanel since={filter.since} until={filter.until} onChange={(since, until) => set({ since, until })} close={close} />
        )}
      </Dropdown>
      <Dropdown label={summary("路径", filter.paths)} active={filter.paths.length > 0} width={320}>
        {(close) => <PathPanel value={filter.paths} onChange={(paths) => set({ paths })} close={close} />}
      </Dropdown>
      {!isFilterEmpty(filter) && (
        <button className="lf-clear" onClick={() => onChange(EMPTY_FILTER)} title="清除所有过滤条件">
          清除
        </button>
      )}
    </div>
  );
}

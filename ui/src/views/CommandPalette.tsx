// Command palette ("Find Action", ⌘⇧A or double Shift): every registered
// action, plus branches, changed files and recent repositories when typing.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { api } from "../api";
import { formatKeys, listActions } from "../actions/registry";
import { bestMatch, type Match } from "../lib/fuzzy";
import { repoName, splitPath } from "../lib/format";
import { load, save } from "../lib/storage";
import { useRepo } from "../stores/repo";

interface Item {
  key: string;
  title: string;
  /** Kind of item, shown on the right. */
  kind: string;
  detail?: string;
  shortcut?: string;
  aliases: string[];
  enabled: boolean;
  run: () => void;
  /** Only listed when the query is not empty (there are many of these). */
  searchOnly?: boolean;
}

const RECENT_KEY = "paletteRecent";
const MAX_RECENT = 8;
const MAX_ROWS = 60;

/** Items from the actions and the open repository. */
function collect(): Item[] {
  const st = useRepo.getState();
  const items: Item[] = listActions()
    .filter((a) => !a.hidden && a.id !== "palette.open")
    .map((a) => ({
      key: `action:${a.id}`,
      title: a.title,
      kind: "",
      shortcut: formatKeys(a.keys),
      aliases: [a.keywords ?? "", a.id],
      enabled: !a.enabled || a.enabled(),
      run: a.run,
    }));
  const snap = st.snapshot;
  if (snap && st.root) {
    const head = snap.head.branch;
    for (const b of snap.branches.local) {
      if (b.isHead) continue;
      items.push({
        key: `checkout:${b.name}`,
        title: b.name,
        kind: "切换分支",
        aliases: [`checkout 切换到 ${b.name}`],
        enabled: true,
        searchOnly: true,
        run: () => void st.runTask(`Checkout ${b.name}`, (r) => api.checkoutBranch(r, b.name, false), `已切换到 ${b.name}`),
      });
      if (head)
        items.push({
          key: `compare:${b.name}`,
          title: `${b.name} ⇄ ${head}`,
          kind: "比较分支",
          aliases: [`compare 比较 ${b.name}`],
          enabled: true,
          searchOnly: true,
          run: () => st.setCompare({ left: b.name, right: head, leftName: b.name, rightName: head }),
        });
    }
    for (const b of snap.branches.remote) {
      const local = b.name.split("/").slice(1).join("/");
      if (snap.branches.local.some((l) => l.name === local)) continue;
      items.push({
        key: `checkout-remote:${b.name}`,
        title: b.name,
        kind: "检出远程分支",
        aliases: [`checkout 检出 ${local}`],
        enabled: true,
        searchOnly: true,
        run: () => void st.runTask(`Checkout ${b.name}`, (r) => api.checkoutBranch(r, b.name, true), `已检出 ${local}`),
      });
    }
    for (const c of snap.changes) {
      const { name, dir } = splitPath(c.path);
      items.push({
        key: `file:${c.path}`,
        title: name,
        detail: dir,
        kind: "已修改的文件",
        aliases: [c.path],
        enabled: true,
        searchOnly: true,
        run: () => {
          st.setView("changes");
          st.select(c.path);
        },
      });
    }
  }
  for (const r of st.recent) {
    if (r === st.root) continue;
    items.push({
      key: `repo:${r}`,
      title: repoName(r),
      detail: r,
      kind: "打开仓库",
      aliases: [r],
      enabled: true,
      searchOnly: true,
      run: () => void st.openRepo(r),
    });
  }
  return items;
}

/** Items matching the query, best first; recently used ones get a boost. */
export function rankItems(items: Item[], query: string, recent: string[]): { item: Item; match: Match | null }[] {
  const recency = (key: string) => {
    const i = recent.indexOf(key);
    return i < 0 ? 0 : (recent.length - i) * 40;
  };
  if (!query.trim()) {
    const listed = items.filter((it) => !it.searchOnly);
    const used = recent.map((k) => listed.find((it) => it.key === k)).filter((it): it is Item => !!it);
    const rest = listed.filter((it) => !used.includes(it)).sort((a, b) => a.title.localeCompare(b.title));
    return [...used, ...rest].map((item) => ({ item, match: null }));
  }
  const out: { item: Item; match: Match | null; score: number }[] = [];
  for (const item of items) {
    const m = bestMatch(query, item.title, item.aliases);
    if (!m) continue;
    // Disabled actions stay listed, below the ones that can run.
    out.push({ item, match: m.title, score: m.score + recency(item.key) - (item.enabled ? 0 : 300) });
  }
  out.sort((a, b) => b.score - a.score);
  return out.slice(0, MAX_ROWS);
}

function Highlight({ text, positions }: { text: string; positions?: number[] }): ReactNode {
  if (!positions?.length) return text;
  const set = new Set(positions);
  return [...text].map((ch, i) => (set.has(i) ? <mark key={i}>{ch}</mark> : ch));
}

export function CommandPalette() {
  const open = useRepo((s) => s.palette);
  const setOpen = useRepo((s) => s.setPalette);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [items, setItems] = useState<Item[]>([]);
  const input = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Reset on close, not on open: keys typed right after the shortcut must not be lost.
  useEffect(() => {
    if (open) setItems(collect());
    else {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  const rows = useMemo(() => rankItems(items, query, load<string[]>(RECENT_KEY, [])), [items, query]);
  // Start on the first item that can run.
  useEffect(() => setActive(Math.max(0, rows.findIndex((r) => r.item.enabled))), [rows]);
  useEffect(() => {
    listRef.current?.querySelector(".cp-row.active")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (!open) return null;
  const close = () => setOpen(false);
  const run = (item: Item | undefined) => {
    if (!item || !item.enabled) return;
    const recent = load<string[]>(RECENT_KEY, []).filter((k) => k !== item.key);
    save(RECENT_KEY, [item.key, ...recent].slice(0, MAX_RECENT));
    close();
    // After the palette is gone: actions may open dialogs or move focus.
    requestAnimationFrame(() => item.run());
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowDown") setActive((i) => Math.min(rows.length - 1, i + 1));
    else if (e.key === "ArrowUp") setActive((i) => Math.max(0, i - 1));
    else if (e.key === "PageDown") setActive((i) => Math.min(rows.length - 1, i + 10));
    else if (e.key === "PageUp") setActive((i) => Math.max(0, i - 10));
    else if (e.key === "Enter") run(rows[active]?.item);
    else return;
    e.preventDefault();
  };

  return (
    <div className="backdrop clear" onMouseDown={close}>
      <div className="branch-popup palette" role="dialog" aria-modal aria-label="命令面板" onMouseDown={(e) => e.stopPropagation()}>
        <input
          ref={input}
          autoFocus
          className="bp-search"
          placeholder="搜索操作、分支、文件…（也可以用中文）"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
          spellCheck={false}
        />
        <div className="bp-list" ref={listRef}>
          {rows.length === 0 && <div className="cp-empty muted">没有匹配的操作</div>}
          {rows.map(({ item, match }, i) => (
            <div
              key={item.key}
              className={`cp-row ${i === active ? "active" : ""} ${item.enabled ? "" : "disabled"}`}
              onMouseMove={() => i !== active && setActive(i)}
              onClick={() => run(item)}
              title={item.enabled ? undefined : "当前不可用"}
            >
              <span className="cp-title">
                <Highlight text={item.title} positions={match?.positions} />
              </span>
              {item.detail && <span className="cp-detail muted">{item.detail}</span>}
              <span className="spacer" />
              {item.kind && <span className="cp-kind muted">{item.kind}</span>}
              {item.shortcut && <span className="kbd cp-key">{item.shortcut}</span>}
            </div>
          ))}
        </div>
        <div className="bp-footer muted">↑↓ 选择 · 回车 执行 · 双击 Shift 或 {formatKeys(["mod+shift+a"])} 打开</div>
      </div>
    </div>
  );
}

/** Open the palette on a double tap of Shift (with nothing else pressed). */
export function installDoubleShift(onDouble: () => void): () => void {
  let last = 0;
  let clean = false;
  const down = (e: globalThis.KeyboardEvent) => {
    clean = e.key === "Shift" && !e.repeat && !e.ctrlKey && !e.metaKey && !e.altKey;
    if (!clean) last = 0;
  };
  const up = (e: globalThis.KeyboardEvent) => {
    if (e.key !== "Shift" || !clean) return;
    const now = performance.now();
    if (now - last < 350) {
      last = 0;
      if (!document.querySelector('[aria-modal="true"]')) onDouble();
    } else last = now;
  };
  const blur = () => (last = 0);
  window.addEventListener("keydown", down, true);
  window.addEventListener("keyup", up, true);
  window.addEventListener("blur", blur);
  return () => {
    window.removeEventListener("keydown", down, true);
    window.removeEventListener("keyup", up, true);
    window.removeEventListener("blur", blur);
  };
}

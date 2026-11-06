import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { api } from "../api";
import type { Branch } from "../bindings/Branch";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

type Item =
  | { type: "header"; label: string }
  | { type: "new" }
  | { type: "branch"; branch: Branch; section: string };

export function BranchPopup() {
  const open = useRepo((s) => s.branchPopup);
  const setOpen = useRepo((s) => s.setBranchPopup);
  const snapshot = useRepo((s) => s.snapshot);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setCreating(false);
      setActive(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const items = useMemo(() => {
    if (!snapshot) return [];
    const q = query.trim().toLowerCase();
    const match = (b: Branch) => !q || b.name.toLowerCase().includes(q);
    const { local, remote, recent } = snapshot.branches;
    const out: Item[] = [];
    if (!q) out.push({ type: "new" });
    const recentBranches = recent
      .map((n) => local.find((b) => b.name === n))
      .filter((b): b is Branch => !!b && !b.isHead)
      .slice(0, 5);
    if (!q && recentBranches.length) {
      out.push({ type: "header", label: "Recent" });
      for (const b of recentBranches) out.push({ type: "branch", branch: b, section: "recent" });
    }
    const l = [...local].sort((a, b) => Number(b.isHead) - Number(a.isHead) || a.name.localeCompare(b.name)).filter(match);
    if (l.length) {
      out.push({ type: "header", label: "Local" });
      for (const b of l) out.push({ type: "branch", branch: b, section: "local" });
    }
    const r = [...remote].sort((a, b) => a.name.localeCompare(b.name)).filter(match);
    if (r.length) {
      out.push({ type: "header", label: "Remote" });
      for (const b of r) out.push({ type: "branch", branch: b, section: "remote" });
    }
    return out;
  }, [snapshot, query]);

  const selectable = items.map((it, i) => (it.type === "header" ? -1 : i)).filter((i) => i >= 0);
  const activeIndex = selectable.includes(active) ? active : (selectable[0] ?? -1);

  useEffect(() => {
    listRef.current?.querySelector(".bp-item.active")?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  if (!open || !snapshot) return null;

  const close = () => setOpen(false);

  async function checkout(b: Branch) {
    close();
    if (b.isHead) return;
    await runTask(`Checkout ${b.name}`, (r) => api.checkoutBranch(r, b.name, b.isRemote), t("已切换到 {branch}", { branch: b.isRemote ? b.name.split("/").slice(1).join("/") : b.name }));
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    close();
    await runTask(t("新建分支 {name}", { name }), (r) => api.createBranch(r, name, null, true), t("已创建并切换到 {name}", { name }));
  }

  async function remove(b: Branch) {
    const ok = await ask({
      title: t("删除分支"),
      message: t("删除本地分支 {name}？未合并的提交可通过 reflog 找回。", { name: b.name }),
      confirmLabel: t("删除"),
      danger: true,
    });
    if (!ok) return;
    await runTask(t("删除分支 {name}", { name: b.name }), (r) => api.deleteBranch(r, b.name, true), t("已删除 {name}", { name: b.name }));
  }

  function activate(i: number) {
    const it = items[i];
    if (!it) return;
    if (it.type === "new") {
      setCreating(true);
      setNewName(query);
      requestAnimationFrame(() => inputRef.current?.focus());
    } else if (it.type === "branch") void checkout(it.branch);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (creating) setCreating(false);
      else close();
      return;
    }
    if (creating) {
      if (e.key === "Enter") {
        e.preventDefault();
        void create();
      }
      return;
    }
    const pos = selectable.indexOf(activeIndex);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive(selectable[Math.min(selectable.length - 1, pos + 1)]);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive(selectable[Math.max(0, pos - 1)]);
    } else if (e.key === "Enter") {
      e.preventDefault();
      activate(activeIndex);
    }
  }

  return (
    <div className="backdrop clear" onMouseDown={close}>
      <div
        className="branch-popup"
        role="dialog"
        aria-modal
        aria-label="Branches"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKey}
      >
        {creating ? (
          <input
            ref={inputRef}
            className="bp-search"
            placeholder={t("新分支名称（从当前 HEAD 创建并切换）")}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
        ) : (
          <input
            ref={inputRef}
            className="bp-search"
            placeholder={t("搜索分支或执行动作…")}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
        )}
        {!creating && (
          <div className="bp-list" ref={listRef}>
            {items.map((it, i) =>
              it.type === "header" ? (
                <div key={`h${it.label}`} className="bp-header">
                  {it.label}
                </div>
              ) : it.type === "new" ? (
                <div
                  key="new"
                  className={`bp-item ${i === activeIndex ? "active" : ""}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => activate(i)}
                >
                  <span className="bp-plus">{t("＋")}</span> New Branch…
                </div>
              ) : (
                <div
                  key={`${it.section}:${it.branch.fullRef}`}
                  className={`bp-item ${i === activeIndex ? "active" : ""}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => activate(i)}
                >
                  <span className={`bp-mark ${it.branch.isHead ? "head" : ""}`}>{it.branch.isHead ? "●" : ""}</span>
                  <span className="bp-name">{it.branch.name}</span>
                  {(it.branch.ahead > 0 || it.branch.behind > 0) && (
                    <span className="bp-track">
                      {it.branch.ahead > 0 && `↑${it.branch.ahead}`}
                      {it.branch.behind > 0 && ` ↓${it.branch.behind}`}
                    </span>
                  )}
                  {it.branch.upstreamGone && <span className="bp-track warn">gone</span>}
                  <span className="bp-upstream">{it.branch.upstream ?? ""}</span>
                  {!it.branch.isHead && (
                    <button
                      className="bp-del bp-cmp"
                      title={t("与当前分支比较")}
                      onClick={(e) => {
                        e.stopPropagation();
                        close();
                        useRepo.getState().setCompare({
                          left: it.branch.fullRef,
                          right: "HEAD",
                          leftName: it.branch.name,
                          rightName: snapshot.head.branch ?? "HEAD",
                        });
                      }}
                    >
                      ⇄
                    </button>
                  )}
                  {!it.branch.isRemote && !it.branch.isHead && (
                    <button
                      className="bp-del"
                      title={t("删除分支")}
                      onClick={(e) => {
                        e.stopPropagation();
                        close();
                        void remove(it.branch);
                      }}
                    >
                      ✕
                    </button>
                  )}
                </div>
              ),
            )}
            {items.length === 0 && <div className="empty">{t("没有匹配的分支")}</div>}
          </div>
        )}
        <div className="bp-footer muted">{t("↑↓ 选择 · ↩ Checkout · Esc 关闭")}</div>
      </div>
    </div>
  );
}

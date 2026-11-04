import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { api } from "../api";
import { useActions, shortcutOf } from "../actions/registry";
import type { ChangeKind } from "../bindings/ChangeKind";
import type { FileChange } from "../bindings/FileChange";
import { ContextMenu, type MenuItem } from "../components/ContextMenu";
import { VirtualList } from "../components/VirtualList";
import { load, save } from "../lib/storage";
import { splitPath } from "../lib/format";
import { isIncluded, useRepo } from "../stores/repo";

type GroupId = "conflicts" | "changes" | "unversioned";

type Row =
  | { type: "group"; id: GroupId; title: string; changes: FileChange[] }
  | { type: "file"; change: FileChange; group: GroupId };

const ROW_H = 24;

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

/** The status shown for a file: worktree state wins, else index state. */
export function displayKind(c: FileChange): ChangeKind {
  if (c.conflicted) return "conflicted";
  if (c.staged === "added" || c.staged === "renamed" || c.staged === "copied") return c.staged;
  return c.unstaged ?? c.staged ?? "modified";
}

function Checkbox({ state, onToggle }: { state: boolean | "partial"; onToggle: () => void }) {
  return (
    <span
      role="checkbox"
      aria-checked={state === "partial" ? "mixed" : state}
      className={`check ${state === true ? "on" : ""} ${state === "partial" ? "partial" : ""}`}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
    />
  );
}

export function CommitPanel() {
  const root = useRepo((s) => s.root)!;
  const snapshot = useRepo((s) => s.snapshot)!;
  const pending = useRepo((s) => s.pendingStage);
  const selectedPath = useRepo((s) => s.selectedPath);
  const select = useRepo((s) => s.select);
  const setIncluded = useRepo((s) => s.setIncluded);
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const busy = useRepo((s) => s.tasks.length > 0);

  const draftKey = `draft:${root}`;
  const [message, setMessage] = useState(() => load(draftKey, ""));
  const [amend, setAmend] = useState(false);
  const [collapsed, setCollapsed] = useState<Partial<Record<GroupId, boolean>>>({});
  const [menu, setMenu] = useState<{ x: number; y: number; change: FileChange } | null>(null);
  const msgRef = useRef<HTMLTextAreaElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => save(draftKey, message), [draftKey, message]);

  const included = (c: FileChange) => (c.path in pending ? pending[c.path] : isIncluded(c));

  const groups = useMemo(() => {
    const sorted = [...snapshot.changes].sort((a, b) => a.path.localeCompare(b.path));
    return {
      conflicts: sorted.filter((c) => c.conflicted),
      changes: sorted.filter((c) => !c.conflicted && c.unstaged !== "untracked"),
      unversioned: sorted.filter((c) => c.unstaged === "untracked"),
    };
  }, [snapshot.changes]);

  const rows = useMemo(() => {
    const out: Row[] = [];
    const add = (id: GroupId, title: string, changes: FileChange[]) => {
      if (id !== "changes" && changes.length === 0) return;
      out.push({ type: "group", id, title, changes });
      if (!collapsed[id]) for (const c of changes) out.push({ type: "file", change: c, group: id });
    };
    add("conflicts", "Merge Conflicts", groups.conflicts);
    add("changes", "Changes", groups.changes);
    add("unversioned", "Unversioned Files", groups.unversioned);
    return out;
  }, [groups, collapsed]);

  const selectedIndex = rows.findIndex((r) => r.type === "file" && r.change.path === selectedPath);
  const selectedChange = selectedIndex >= 0 ? (rows[selectedIndex] as { change: FileChange }).change : null;

  const includedCount = snapshot.changes.filter((c) => included(c) !== false).length;
  const canCommit =
    !busy && message.trim().length > 0 && (includedCount > 0 || amend) && groups.conflicts.length === 0;

  const groupState = (changes: FileChange[]): boolean | "partial" => {
    if (changes.length === 0) return false;
    const states = changes.map(included);
    if (states.every((s) => s === true)) return true;
    if (states.every((s) => s === false)) return false;
    return "partial";
  };

  const toggle = (changes: FileChange[]) => {
    const st = changes.length === 1 ? included(changes[0]) : groupState(changes);
    void setIncluded(changes, st !== true);
  };

  async function rollback(changes: FileChange[]) {
    if (changes.length === 0) return;
    const untracked = changes.filter((c) => c.unstaged === "untracked").map((c) => c.path);
    // Newly added files are only un-added (kept on disk), like IDEA's default.
    const added = changes.filter((c) => c.staged === "added").map((c) => c.path);
    const tracked = changes
      .filter((c) => c.unstaged !== "untracked" && c.staged !== "added")
      .flatMap((c) => (c.origPath ? [c.path, c.origPath] : [c.path]));
    const names = changes.length === 1 ? changes[0].path : `${changes.length} 个文件`;
    const ok = await ask({
      title: "Rollback Changes",
      message:
        `将 ${names} 恢复到 HEAD 版本，本地修改会丢失。` +
        (untracked.length ? `\n其中 ${untracked.length} 个未跟踪文件将被删除。` : ""),
      confirmLabel: "Rollback",
      danger: true,
    });
    if (!ok) return;
    await runTask("Rollback", async (r) => {
      if (added.length) await api.unstage(r, added);
      await api.rollback(r, tracked, untracked);
    });
  }

  async function commit(andPush: boolean) {
    if (!canCommit) return;
    const msg = message;
    const oid = await runTask("提交", (r) => api.commit(r, msg, amend));
    if (!oid) return;
    setMessage("");
    setAmend(false);
    useRepo.getState().toast({ kind: "success", title: `已提交 ${oid.slice(0, 8)}`, detail: msg.split("\n")[0] });
    // Like IDEA: "Commit and Push…" commits, then opens the push dialog.
    if (andPush) useRepo.getState().setPushDialog(true);
  }

  async function toggleAmend(next: boolean) {
    setAmend(next);
    if (next && !message.trim()) {
      const last = await api.lastCommitMessage(root).catch(() => "");
      setMessage(last);
    }
  }

  useActions([
    {
      id: "commit.focus",
      title: "Commit…",
      keys: ["mod+k"],
      run: () => {
        useRepo.getState().setView("changes");
        msgRef.current?.focus();
      },
    },
    { id: "commit.run", title: "Commit", keys: ["mod+enter"], run: () => void commit(false), enabled: () => canCommit },
    {
      id: "commit.runAndPush",
      title: "Commit and Push",
      keys: ["mod+alt+k"],
      run: () => void commit(true),
      enabled: () => canCommit,
    },
    {
      id: "changes.rollback",
      title: "Rollback…",
      keys: ["mod+alt+z"],
      run: () => selectedChange && void rollback([selectedChange]),
      enabled: () => selectedChange != null,
    },
  ]);

  function onListKey(e: KeyboardEvent<HTMLDivElement>) {
    const files = rows
      .map((r, i) => ({ r, i }))
      .filter((x): x is { r: Extract<Row, { type: "file" }>; i: number } => x.r.type === "file");
    if (files.length === 0) return;
    const pos = files.findIndex((f) => f.i === selectedIndex);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next =
        pos < 0 ? 0 : Math.max(0, Math.min(files.length - 1, pos + (e.key === "ArrowDown" ? 1 : -1)));
      select(files[next].r.change.path);
    } else if (e.key === " " && selectedChange) {
      e.preventDefault();
      toggle([selectedChange]);
    }
  }

  function menuItems(c: FileChange): (MenuItem | "separator")[] {
    const inc = included(c);
    return [
      { label: inc === true ? "从提交中排除 (Unstage)" : "加入提交 (Stage)", onClick: () => toggle([c]) },
      { label: "Rollback…", shortcut: shortcutOf("changes.rollback"), danger: true, onClick: () => void rollback([c]) },
      "separator",
      { label: "复制路径", onClick: () => void navigator.clipboard.writeText(c.path) },
    ];
  }

  return (
    <div className="commit-panel">
      <div className="panel-header">
        <span className="panel-title">Changes</span>
        <span className="muted">{snapshot.changes.length ? `${includedCount}/${snapshot.changes.length} 已选` : ""}</span>
      </div>
      <div ref={listRef} className="changes-list-wrap">
        {snapshot.changes.length === 0 ? (
          <div className="empty">没有本地修改</div>
        ) : (
          <VirtualList
            items={rows}
            rowHeight={ROW_H}
            tabIndex={0}
            className="changes-list"
            scrollToIndex={selectedIndex}
            onKeyDown={onListKey}
            render={(row) =>
              row.type === "group" ? (
                <div key={`g:${row.id}`} className="row group-row" style={{ height: ROW_H }}>
                  <button
                    className={`twisty ${collapsed[row.id] ? "closed" : ""}`}
                    onClick={() => setCollapsed((c) => ({ ...c, [row.id]: !c[row.id] }))}
                    aria-label="toggle group"
                  />
                  {row.id !== "conflicts" && <Checkbox state={groupState(row.changes)} onToggle={() => toggle(row.changes)} />}
                  <span className="group-title">{row.title}</span>
                  <span className="muted">{row.changes.length} files</span>
                </div>
              ) : (
                <FileRow
                  key={row.change.path}
                  change={row.change}
                  included={included(row.change)}
                  selected={row.change.path === selectedPath}
                  onSelect={() => select(row.change.path)}
                  onToggle={() => toggle([row.change])}
                  onContext={(x, y) => {
                    select(row.change.path);
                    setMenu({ x, y, change: row.change });
                  }}
                />
              )
            }
          />
        )}
      </div>
      <div className="commit-box">
        <div className="commit-options">
          <label className="opt">
            <input type="checkbox" checked={amend} onChange={(e) => void toggleAmend(e.target.checked)} />
            Amend
          </label>
        </div>
        <textarea
          ref={msgRef}
          className="commit-message"
          placeholder={`提交信息（${shortcutOf("commit.run") || "⌘↩"} 提交）`}
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          spellCheck={false}
        />
        <div className="commit-actions">
          <button className="btn btn-primary" disabled={!canCommit} onClick={() => void commit(false)}>
            {amend ? "Amend Commit" : "Commit"}
          </button>
          <button className="btn" disabled={!canCommit} onClick={() => void commit(true)}>
            {amend ? "Amend" : "Commit"} and Push…
          </button>
        </div>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.change)} onClose={() => setMenu(null)} />}
    </div>
  );
}

function FileRow(props: {
  change: FileChange;
  included: boolean | "partial";
  selected: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onContext: (x: number, y: number) => void;
}) {
  const { change: c } = props;
  const kind = displayKind(c);
  const { name, dir } = splitPath(c.path);
  return (
    <div
      className={`row file-row ${props.selected ? "selected" : ""}`}
      style={{ height: ROW_H }}
      onMouseDown={props.onSelect}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onContext(e.clientX, e.clientY);
      }}
      title={c.origPath ? `${c.origPath} → ${c.path}` : c.path}
    >
      <span className="indent" />
      {c.conflicted ? <span className="check-spacer" /> : <Checkbox state={props.included} onToggle={props.onToggle} />}
      <span className={`kind kind-${kind}`}>{KIND_LABEL[kind]}</span>
      <span className={`fname kind-${kind}`}>{name}</span>
      <span className="fdir">{dir}</span>
    </div>
  );
}

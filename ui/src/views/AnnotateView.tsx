// Annotate (blame) view: a file with, per line, who last changed it and
// when. The gutter shows date and author on the first line of each run and
// colours every line by age; hover for details, click to show the commit in
// the Log, right-click to annotate the file as it was before that commit.
import { useEffect, useRef, useState } from "react";
import { Compartment, EditorState, StateEffect, StateField, type Range } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, gutter, type DecorationSet } from "@codemirror/view";
import { api, errorMessage } from "../api";
import type { Blame } from "../bindings/Blame";
import type { BlameCommit } from "../bindings/BlameCommit";
import { editorLook, loadLanguage } from "../components/CodeDiff";
import { ContextMenu } from "../components/ContextMenu";
import { relativeTime } from "../lib/format";
import { useRepo } from "../stores/repo";
import { locale, t } from "../i18n";

const isUncommitted = (c: BlameCommit) => /^0+$/.test(c.oid);

function shortDate(t: number): string {
  const d = new Date(t * 1000);
  return `${d.getFullYear()}/${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`;
}

class BlameMarker extends GutterMarker {
  constructor(
    readonly label: string,
    readonly heat: string,
    readonly title: string,
  ) {
    super();
  }
  eq(o: BlameMarker) {
    return o.label === this.label && o.heat === this.heat && o.title === this.title;
  }
  toDOM() {
    const el = document.createElement("div");
    el.className = "blame-cell";
    el.style.setProperty("--heat", this.heat);
    el.textContent = this.label;
    el.title = this.title;
    return el;
  }
}

/** Lines of the commit under the mouse, highlighted in the text. */
const setHover = StateEffect.define<number | null>();

export function AnnotateView() {
  const target = useRepo((s) => s.annotate);
  const setAnnotate = useRepo((s) => s.setAnnotate);
  const root = useRepo((s) => s.root);
  const showInLog = useRepo((s) => s.showInLog);
  // Earlier views when digging into older revisions, for "back".
  const [history, setHistory] = useState<{ path: string; rev: string | null }[]>([]);
  const [blame, setBlame] = useState<Blame | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; commit: BlameCommit } | null>(null);
  // Selected lines (1-based, inclusive), for "history of the selection".
  const [lines, setLines] = useState<[number, number] | null>(null);
  const openHistory = useRepo((s) => s.setHistory);
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!target || !root) return;
    let alive = true;
    setBlame(null);
    setError(null);
    api
      .blame(root, target.path, target.rev)
      .then((b) => alive && setBlame(b))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [target, root]);

  useEffect(() => {
    if (!blame || !host.current) return;
    const commits = blame.commits;
    // Heat by age: newest commits strongest, uncommitted lines their own colour.
    const times = commits.filter((c) => !isUncommitted(c)).map((c) => c.authorTime);
    const min = Math.min(...times);
    const max = Math.max(...times);
    const heat = (c: BlameCommit) =>
      isUncommitted(c)
        ? "var(--k-added)"
        : `color-mix(in srgb, var(--accent) ${Math.round(8 + 40 * (max > min ? (c.authorTime - min) / (max - min) : 1))}%, transparent)`;
    const title = (c: BlameCommit) =>
      isUncommitted(c)
        ? t("尚未提交的修改")
        : t("{hash} · {authorName} <{authorEmail}>\n{date}（{relativeTime}）\n\n{summary}\n\n点击在 Log 中查看，右键查看更多", { hash: c.oid.slice(0, 8), authorName: c.authorName, authorEmail: c.authorEmail, date: new Date(c.authorTime * 1000).toLocaleString(locale), relativeTime: relativeTime(c.authorTime), summary: c.summary });
    const markers = commits.map(
      (c) =>
        new BlameMarker(isUncommitted(c) ? t("未提交") : `${shortDate(c.authorTime)}  ${c.authorName}`, heat(c), title(c)),
    );
    const blank = commits.map((c) => new BlameMarker("", heat(c), title(c)));
    const commitAt = (view: EditorView, pos: number) => blame.lines[view.state.doc.lineAt(pos).number - 1];

    const hover = StateField.define<DecorationSet>({
      create: () => Decoration.none,
      update(v, tr) {
        for (const e of tr.effects) {
          if (!e.is(setHover)) continue;
          if (e.value == null) return Decoration.none;
          const out: Range<Decoration>[] = [];
          blame.lines.forEach((ci, i) => {
            if (ci === e.value && i < tr.state.doc.lines)
              out.push(Decoration.line({ class: "blame-hover" }).range(tr.state.doc.line(i + 1).from));
          });
          return Decoration.set(out);
        }
        return v;
      },
      provide: (f) => EditorView.decorations.from(f),
    });

    const lang = new Compartment();
    const view = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: blame.text,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          gutter({
            class: "cm-blame",
            lineMarker(v, line) {
              const n = v.state.doc.lineAt(line.from).number - 1;
              const ci = blame.lines[n];
              if (ci == null) return null;
              // Label only the first line of a run of the same commit.
              return n > 0 && blame.lines[n - 1] === ci ? blank[ci] : markers[ci];
            },
            initialSpacer: () => new BlameMarker("0000/00/00  Wwwwwwwwwwww", "", ""),
            domEventHandlers: {
              mousedown(v, line, e) {
                const ci = commitAt(v, line.from);
                const c = ci == null ? null : commits[ci];
                if (!c) return false;
                const me = e as MouseEvent;
                if (me.button === 2) return false;
                if (!isUncommitted(c)) showInLog(c.oid);
                return true;
              },
              contextmenu(v, line, e) {
                const ci = commitAt(v, line.from);
                if (ci == null) return false;
                e.preventDefault();
                const me = e as MouseEvent;
                setMenu({ x: me.clientX, y: me.clientY, commit: commits[ci] });
                return true;
              },
              mouseover(v, line) {
                v.dispatch({ effects: setHover.of(commitAt(v, line.from) ?? null) });
                return false;
              },
              mouseleave(v) {
                v.dispatch({ effects: setHover.of(null) });
                return false;
              },
            },
          }),
          editorLook,
          lang.of([]),
          hover,
          EditorView.updateListener.of((u) => {
            if (!u.selectionSet) return;
            const r = u.state.selection.main;
            const a = u.state.doc.lineAt(r.from).number;
            const b = u.state.doc.lineAt(r.to).number;
            setLines(r.empty ? null : [a, b]);
          }),
        ],
      }),
    });
    let alive = true;
    loadLanguage(blame.path, lang, [view], () => alive);
    return () => {
      alive = false;
      view.destroy();
    };
  }, [blame, showInLog]);

  if (!target) return null;
  const close = () => {
    setHistory([]);
    setAnnotate(null);
  };
  const open = (next: { path: string; rev: string | null }) => {
    setHistory((h) => [...h, target]);
    setAnnotate(next);
  };
  const back = () => {
    const prev = history[history.length - 1];
    setHistory((h) => h.slice(0, -1));
    if (prev) setAnnotate(prev);
  };
  const authors = blame ? new Set(blame.commits.filter((c) => !isUncommitted(c)).map((c) => c.authorEmail)).size : 0;

  return (
    <div className="mt-overlay" role="dialog" aria-modal="true" aria-label={t("注释 {path}", { path: target.path })} onKeyDown={(e) => e.key === "Escape" && close()}>
      <div className="mt-window">
        <div className="mt-header">
          {history.length > 0 && (
            <button className="btn btn-sm" onClick={back} title={t("回到上一个版本的注释")}>
              {t("← 返回")}
            </button>
          )}
          <b>{t("注释 {path}", { path: target.path })}</b>
          <span className="muted">
            {target.rev ? `@ ${target.rev.slice(0, 8)}` : t("工作区")}
            {blame && t(" · {lines} 行，{commits} 个提交，{authors} 位作者", { lines: blame.lines.length, commits: blame.commits.length, authors })}
          </span>
          <span className="spacer" />
          <button
            className="btn btn-sm"
            disabled={!lines}
            onClick={() => lines && openHistory({ path: target.path, range: lines, rev: target.rev })}
            title={
              lines
                ? t("修改过第 {from}–{to} 行的提交{note}", { from: lines[0], to: lines[1], note: target.rev ? "" : t("（按 HEAD 版本的行号计算）") })
                : t("先在文件中选中几行")
            }
          >
            {t("选区历史{note}", { note: lines ? t("（{from}–{to}）", { note: lines[0], to: lines[1] }) : "" })}
          </button>
          <button className="btn btn-sm" onClick={() => openHistory({ path: target.path })}>
            {t("文件历史")}
          </button>
          <button className="btn btn-sm" onClick={close} autoFocus>
            {t("关闭")}
          </button>
        </div>
        {error ? (
          <div className="diff-empty">{error}</div>
        ) : !blame ? (
          <div className="diff-empty">
            <span className="spinner" />{t("正在计算每一行的来源…")}
          </div>
        ) : (
          <div className="mt-editor annotate-editor" ref={host} />
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={
            isUncommitted(menu.commit)
              ? [{ label: t("这些修改尚未提交"), disabled: true, onClick: () => {} }]
              : [
                  { label: t("在 Log 中显示此提交"), onClick: () => showInLog(menu.commit.oid) },
                  {
                    label: t("注释此提交之前的版本"),
                    disabled: !menu.commit.previousOid,
                    onClick: () => open({ path: menu.commit.previousPath!, rev: menu.commit.previousOid }),
                  },
                  "separator",
                  { label: t("复制提交 hash"), onClick: () => void navigator.clipboard.writeText(menu.commit.oid) },
                ]
          }
        />
      )}
    </div>
  );
}

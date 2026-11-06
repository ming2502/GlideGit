// Three-way merge tool: ours (left, read-only) | result (centre, editable) |
// theirs (right, read-only). The result starts from the common base; each
// changed region can be taken from either side (or both), or ignored, and
// the "magic wand" applies every non-conflicting change at once.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Compartment,
  EditorState,
  RangeSet,
  StateEffect,
  StateField,
  type Extension,
  type Range,
} from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, gutter, type DecorationSet } from "@codemirror/view";
import { api, errorMessage } from "../api";
import type { ConflictTexts } from "../bindings/ConflictTexts";
import { editorLook, loadLanguage } from "../components/CodeDiff";
import { joinLines, merge3, type Merge3, type Region } from "../lib/merge3";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

type Side = "ours" | "theirs";

interface RegionState {
  /** Sides whose change is in the result, in the order they were applied. */
  applied: Side[];
  /** Sides dealt with (applied or ignored). */
  done: Side[];
}

const sidesOf = (r: Region): Side[] =>
  r.kind === "ours" ? ["ours"] : r.kind === "theirs" ? ["theirs"] : r.kind === "same" ? ["ours"] : ["ours", "theirs"];
const resolved = (r: Region, s: RegionState) => sidesOf(r).every((x) => s.done.includes(x));

// ------------------------------------------------------- result range field

/** Where each region is in the result document; follows every edit. */
const setRanges = StateEffect.define<{ from: number; to: number }[]>();
const replaceRegion = StateEffect.define<{ index: number; to: number }>();
const ranges = StateField.define<{ from: number; to: number }[]>({
  create: () => [],
  update(value, tr) {
    let v = tr.docChanged
      ? value.map((r) => ({ from: tr.changes.mapPos(r.from, -1), to: tr.changes.mapPos(r.to, 1) }))
      : value;
    for (const e of tr.effects) {
      if (e.is(setRanges)) v = e.value;
      if (e.is(replaceRegion)) v = v.map((r, i) => (i === e.value.index ? { ...r, to: e.value.to } : r));
    }
    return v;
  },
});

// ------------------------------------------------- decorations and gutters

const setDecos = StateEffect.define<DecorationSet>();
const decos = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(v, tr) {
    for (const e of tr.effects) if (e.is(setDecos)) return e.value;
    return v.map(tr.changes);
  },
  provide: (f) => EditorView.decorations.from(f),
});

class ButtonMarker extends GutterMarker {
  constructor(
    readonly label: string,
    readonly title: string,
    readonly cls: string,
  ) {
    super();
  }
  eq(o: ButtonMarker) {
    return o.label === this.label && o.cls === this.cls;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = `mt-btn ${this.cls}`;
    el.textContent = this.label;
    el.title = this.title;
    return el;
  }
}

const setButtons = StateEffect.define<RangeSet<GutterMarker>>();
const buttons = StateField.define<RangeSet<GutterMarker>>({
  create: () => RangeSet.empty,
  update(v, tr) {
    for (const e of tr.effects) if (e.is(setButtons)) return e.value;
    return v;
  },
});

function buttonGutter(onClick: (view: EditorView, line: number, x: number) => void): Extension {
  return [
    buttons,
    gutter({
      class: "cm-mt-buttons",
      markers: (v) => v.state.field(buttons),
      domEventHandlers: {
        mousedown(view, block, e) {
          const target = e.target as HTMLElement;
          if (!target.classList.contains("mt-btn")) return false;
          e.preventDefault();
          onClick(view, view.state.doc.lineAt(block.from).number, target.classList.contains("mt-ignore") ? 1 : 0);
          return true;
        },
      },
    }),
  ];
}

const lineDeco = (cls: string) => Decoration.line({ class: cls });

/** 1-based line a region starting at 0-based `line` shows its buttons on. */
const buttonLine = (state: EditorState, line: number) => Math.max(1, Math.min(state.doc.lines, line + 1));

/** Line decorations for line range [a, b) of a document (1-based lines). */
function lineRange(state: EditorState, a: number, b: number, cls: string, out: Range<Decoration>[]) {
  const doc = state.doc;
  if (b <= a) {
    // Empty range: mark the line where it sits (a thin bar via CSS).
    const n = Math.min(a + 1, doc.lines);
    out.push(lineDeco(`${cls} mt-empty${a >= doc.lines ? " mt-after" : ""}`).range(doc.line(n).from));
    return;
  }
  for (let n = a + 1; n <= Math.min(b, doc.lines); n++) out.push(lineDeco(cls).range(doc.line(n).from));
}

// --------------------------------------------------------------- component

export function MergeTool() {
  const path = useRepo((s) => s.mergePath);
  const close = useRepo((s) => s.setMergePath);
  const root = useRepo((s) => s.root);
  const [texts, setTexts] = useState<ConflictTexts | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!path || !root) return;
    let alive = true;
    setTexts(null);
    setError(null);
    api
      .conflictTexts(root, path)
      .then((t) => alive && setTexts(t))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [path, root]);

  if (!path) return null;
  return (
    <div className="mt-overlay" role="dialog" aria-modal="true" aria-label={t("合并 {path}", { path })}>
      {error ? (
        <MergeMessage path={path} message={error} onClose={() => close(null)} />
      ) : !texts ? (
        <MergeMessage path={path} message={t("正在读取冲突…")} onClose={() => close(null)} />
      ) : texts.binary || texts.tooLarge || texts.lossy || texts.ours == null || texts.theirs == null ? (
        <WholeFileChoice texts={texts} onClose={() => close(null)} />
      ) : (
        <MergeEditor texts={texts} onClose={() => close(null)} />
      )}
    </div>
  );
}

function MergeMessage({ path, message, onClose }: { path: string; message: string; onClose: () => void }) {
  return (
    <div className="mt-window">
      <div className="mt-header">
        <b>{path}</b>
        <span className="spacer" />
        <button className="btn btn-sm" onClick={onClose}>
          {t("关闭")}
        </button>
      </div>
      <div className="diff-empty">{message}</div>
    </div>
  );
}

/** Binary, oversized or delete/modify conflicts: pick a side as a whole. */
function WholeFileChoice({ texts, onClose }: { texts: ConflictTexts; onClose: () => void }) {
  const runTask = useRepo((s) => s.runTask);
  const describe = (text: string | null, label: string) =>
    t("{label}：{state}", { label, state: text == null ? t("已删除此文件") : t("修改了此文件") });
  const why = texts.binary
    ? t("二进制文件无法逐行合并。")
    : texts.tooLarge
      ? t("文件超过 8 MB，无法在合并工具中打开。")
      : texts.lossy
        ? t("文件不是 UTF-8 文本，或含有单独的 CR 换行，逐行合并可能损坏内容。")
        : t("一方修改了文件，另一方删除了它。");
  const take = (side: "ours" | "theirs") => {
    onClose();
    void runTask(t("解决 {path}", { path: texts.path }), (r) => api.resolveWithSide(r, texts.path, side), t("已解决 {path}", { path: texts.path }));
  };
  return (
    <div className="mt-window mt-small">
      <div className="mt-header">
        <b>{texts.path}</b>
      </div>
      <div className="mt-choice">
        <p>{t("{why}请选择保留哪一方的版本：", { why })}</p>
        <ul>
          <li>{describe(texts.ours, texts.oursLabel)}</li>
          <li>{describe(texts.theirs, texts.theirsLabel)}</li>
        </ul>
        <div className="dialog-actions">
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn" onClick={() => take("ours")}>
            {t("采用左侧（{oursLabel}）", { oursLabel: texts.oursLabel })}
          </button>
          <button className="btn btn-primary" onClick={() => take("theirs")}>
            {t("采用右侧（{theirsLabel}）", { theirsLabel: texts.theirsLabel })}
          </button>
        </div>
      </div>
    </div>
  );
}

function MergeEditor({ texts, onClose }: { texts: ConflictTexts; onClose: () => void }) {
  const runTask = useRepo((s) => s.runTask);
  const ask = useRepo((s) => s.ask);
  const m: Merge3 = useMemo(() => merge3(texts.base ?? "", texts.ours ?? "", texts.theirs ?? ""), [texts]);
  const [states, setStates] = useState<RegionState[]>(() => m.regions.map(() => ({ applied: [], done: [] })));
  const statesRef = useRef(states);
  statesRef.current = states;
  const hosts = { left: useRef<HTMLDivElement>(null), mid: useRef<HTMLDivElement>(null), right: useRef<HTMLDivElement>(null) };
  const views = useRef<{ left: EditorView; mid: EditorView; right: EditorView } | null>(null);
  const [dirty, setDirty] = useState(false);

  // --- region actions -----------------------------------------------------
  const act = useCallback(
    (index: number, side: Side, how: "apply" | "ignore") => {
      const v = views.current;
      if (!v) return;
      const r = m.regions[index];
      const st = statesRef.current[index];
      if (st.done.includes(side)) return;
      const next: RegionState = { applied: [...st.applied], done: [...st.done, side] };
      // Taking one copy of an identical change resolves both sides.
      if (r.kind === "same") next.done = ["ours", "theirs"];
      if (how === "apply") {
        next.applied.push(side);
        const lines = next.applied.flatMap((s) => (s === "ours" ? m.ours.slice(...r.ours) : m.theirs.slice(...r.theirs)));
        const { from, to } = v.mid.state.field(ranges)[index];
        const doc = v.mid.state.doc;
        const text = joinLines(lines);
        // Keep the line breaks around the region: after it when more follows,
        // before it when appending to a last line that had none.
        const needsEol = text !== "" && !text.endsWith("\n") && to < doc.length;
        const needsLead = text !== "" && from === doc.length && from > 0 && doc.sliceString(from - 1, from) !== "\n";
        const insert = (needsLead ? "\n" : "") + (needsEol ? text + "\n" : text);
        v.mid.dispatch({
          changes: { from, to, insert },
          effects: replaceRegion.of({ index, to: from + insert.length }),
        });
      }
      setStates((all) => all.map((s, i) => (i === index ? next : s)));
    },
    [m],
  );

  const wand = useCallback(() => {
    m.regions.forEach((r, i) => {
      const st = statesRef.current[i];
      if (r.kind === "conflict" || resolved(r, st)) return;
      act(i, r.kind === "theirs" ? "theirs" : "ours", "apply");
    });
  }, [m, act]);

  // --- editors ------------------------------------------------------------
  useEffect(() => {
    const lang = new Compartment();
    const ro = [EditorState.readOnly.of(true), EditorView.editable.of(false)];
    // Buttons sit on a region's first line in that pane (see the redraw below).
    const onSideButton = (side: Side) => (view: EditorView, line: number, which: number) => {
      const i = m.regions.findIndex(
        (r, k) =>
          buttonLine(view.state, r[side][0]) === line &&
          !statesRef.current[k].done.includes(side) &&
          !resolved(r, statesRef.current[k]),
      );
      if (i >= 0) act(i, side, which === 1 ? "ignore" : "apply");
    };
    const left = new EditorView({
      parent: hosts.left.current!,
      state: EditorState.create({
        doc: texts.ours ?? "",
        extensions: [ro, editorLook, lang.of([]), decos, buttonGutter(onSideButton("ours"))],
      }),
    });
    const right = new EditorView({
      parent: hosts.right.current!,
      state: EditorState.create({
        doc: texts.theirs ?? "",
        extensions: [ro, editorLook, lang.of([]), decos, buttonGutter(onSideButton("theirs"))],
      }),
    });
    // The result starts as the common base, with each region's range in it.
    const mid = new EditorView({
      parent: hosts.mid.current!,
      state: EditorState.create({
        doc: texts.base ?? "",
        extensions: [
          editorLook,
          lang.of([]),
          ranges,
          decos,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) setDirty(true);
          }),
        ],
      }),
    });
    const offsets: { from: number; to: number }[] = m.regions.map((r) => {
      const doc = mid.state.doc;
      const pos = (line: number) => (line >= doc.lines ? doc.length : doc.line(line + 1).from);
      // A region at the very end of a file without a final newline.
      const from = r.base[0] >= doc.lines ? doc.length : pos(r.base[0]);
      return { from, to: r.base[1] > r.base[0] ? pos(r.base[1]) : from };
    });
    mid.dispatch({ effects: setRanges.of(offsets) });
    views.current = { left, mid, right };
    // Keys (F7, ⌘↩, Esc) reach the tool through the focused editor.
    mid.focus();
    let alive = true;
    loadLanguage(texts.path, lang, [left, mid, right], () => alive);

    // Keep the side panes level with the result while it scrolls.
    const sync = () => {
      const v = views.current;
      if (!v) return;
      const top = v.mid.lineBlockAtHeight(v.mid.scrollDOM.scrollTop);
      const line = v.mid.state.doc.lineAt(top.from).number - 1;
      const rs = v.mid.state.field(ranges);
      for (const [view, side] of [
        [v.left, "ours"],
        [v.right, "theirs"],
      ] as const) {
        let target = line;
        for (let i = 0; i < m.regions.length; i++) {
          const r = m.regions[i];
          const rStart = v.mid.state.doc.lineAt(rs[i].from).number - 1;
          const rEnd = rs[i].to > rs[i].from ? v.mid.state.doc.lineAt(Math.max(rs[i].from, rs[i].to - 1)).number : rStart;
          if (line < rStart) {
            target = r[side][0] - (rStart - line);
            break;
          }
          if (line < rEnd) {
            target = r[side][0] + Math.min(line - rStart, r[side][1] - r[side][0]);
            break;
          }
          target = r[side][1] + (line - rEnd);
        }
        const doc = view.state.doc;
        const n = Math.max(1, Math.min(doc.lines, target + 1));
        view.scrollDOM.scrollTop = view.lineBlockAt(doc.line(n).from).top + (v.mid.scrollDOM.scrollTop - top.top);
      }
    };
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(sync);
    };
    mid.scrollDOM.addEventListener("scroll", onScroll);

    return () => {
      alive = false;
      cancelAnimationFrame(frame);
      mid.scrollDOM.removeEventListener("scroll", onScroll);
      views.current = null;
      left.destroy();
      mid.destroy();
      right.destroy();
    };
  }, [texts, m, act]);

  // Redraw highlights and buttons whenever a region changes state.
  useEffect(() => {
    const v = views.current;
    if (!v) return;
    const rs = v.mid.state.field(ranges);
    const sideDecos = (side: Side, state: EditorState) => {
      const out: Range<Decoration>[] = [];
      const btns: Range<GutterMarker>[] = [];
      m.regions.forEach((r, i) => {
        if (!sidesOf(r).includes(side) && r.kind !== "same") return;
        const st = states[i];
        const done = st.done.includes(side) || resolved(r, st);
        lineRange(state, r[side][0], r[side][1], `mt-${r.kind}${done ? " mt-done" : ""}`, out);
        if (!done) {
          const pos = state.doc.line(buttonLine(state, r[side][0])).from;
          btns.push(new ButtonMarker(side === "ours" ? "»" : "«", side === "ours" ? t("采用左侧的修改") : t("采用右侧的修改"), "mt-apply").range(pos));
          btns.push(new ButtonMarker("×", t("忽略这一侧的修改"), "mt-ignore").range(pos));
        }
      });
      return { decos: Decoration.set(out, true), btns: RangeSet.of(btns, true) };
    };
    const l = sideDecos("ours", v.left.state);
    const r = sideDecos("theirs", v.right.state);
    v.left.dispatch({ effects: [setDecos.of(l.decos), setButtons.of(l.btns)] });
    v.right.dispatch({ effects: [setDecos.of(r.decos), setButtons.of(r.btns)] });
    const mid: Range<Decoration>[] = [];
    m.regions.forEach((reg, i) => {
      const { from, to } = rs[i];
      const doc = v.mid.state.doc;
      const a = doc.lineAt(from).number - 1;
      const b = to > from ? doc.lineAt(to - 1).number : a;
      lineRange(v.mid.state, a, b, `mt-${reg.kind}${resolved(reg, states[i]) ? " mt-done" : ""}`, mid);
    });
    v.mid.dispatch({ effects: setDecos.of(Decoration.set(mid, true)) });
  }, [states, m]);

  // --- navigation and saving ---------------------------------------------
  const remaining = m.regions.filter((r, i) => r.kind === "conflict" && !resolved(r, states[i])).length;
  const pending = m.regions.filter((r, i) => r.kind !== "conflict" && !resolved(r, states[i])).length;
  const conflicts = m.regions.filter((r) => r.kind === "conflict").length;

  const go = (dir: 1 | -1) => {
    const v = views.current;
    if (!v) return;
    const rs = v.mid.state.field(ranges);
    const here = v.mid.lineBlockAtHeight(v.mid.scrollDOM.scrollTop + v.mid.scrollDOM.clientHeight / 2).from;
    const order = m.regions.map((_, i) => i).filter((i) => !resolved(m.regions[i], states[i]));
    const next = dir > 0 ? order.find((i) => rs[i].from > here + 1) : [...order].reverse().find((i) => rs[i].from < here - 1);
    const i = next ?? (dir > 0 ? order[0] : order[order.length - 1]);
    if (i == null) return;
    v.mid.dispatch({ effects: EditorView.scrollIntoView(rs[i].from, { y: "center" }) });
  };

  const save = async () => {
    const v = views.current;
    if (!v) return;
    if (remaining > 0) {
      const ok = await ask({
        title: t("还有冲突未处理"),
        message: t("还有 {remaining} 处冲突没有选择任何一方，这些位置将保留共同祖先的内容。确定要保存并标记为已解决吗？", { remaining }),
        confirmLabel: t("保存"),
      });
      if (!ok) return;
    }
    const content = v.mid.state.doc.toString();
    onClose();
    void runTask(t("解决 {path}", { path: texts.path }), (r) => api.resolveConflict(r, texts.path, content), t("已解决 {path}", { path: texts.path }));
  };

  const cancel = async () => {
    if (dirty) {
      const ok = await ask({ title: t("放弃合并结果？"), message: t("当前的合并结果不会保存，文件仍处于冲突状态。"), confirmLabel: t("放弃"), danger: true });
      if (!ok) return;
    }
    onClose();
  };

  // Global shortcuts pause while this modal is open, so keys are handled here.
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "F7") go(e.shiftKey ? -1 : 1);
    else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save();
    else if (e.key === "Escape") void cancel();
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div className="mt-window" onKeyDown={onKey}>
      <div className="mt-header">
        <b>{t("合并 {path}", { path: texts.path })}</b>
        <span className="muted">
          {t("{changes} 处修改，{conflicts} 个冲突", { changes: m.regions.length, conflicts })}
          {remaining > 0 ? t("，还剩 {remaining} 个未处理", { remaining }) : conflicts > 0 ? t("，冲突已全部处理") : ""}
        </span>
        <span className="spacer" />
        <button className="btn btn-sm" onClick={wand} disabled={pending === 0} title={t("一次性应用所有不冲突的修改")}>
          {t("✦ 应用不冲突的修改")}{pending > 0 ? t("（{pending}）", { pending }) : ""}
        </button>
        <button className="icon-btn" title={t("上一个未处理的修改（⇧F7）")} onClick={() => go(-1)}>
          ↑
        </button>
        <button className="icon-btn" title={t("下一个未处理的修改（F7）")} onClick={() => go(1)}>
          ↓
        </button>
        <button className="btn btn-sm" onClick={() => void cancel()}>
          {t("取消")}
        </button>
        <button className="btn btn-sm btn-primary" onClick={() => void save()} title="⌘↩ / Ctrl+Enter">
          {t("保存并标记为已解决")}
        </button>
      </div>
      <div className="mt-columns">
        <div className="mt-col">
          <div className="mt-col-head">{t("左侧（我们的）：{oursLabel}", { oursLabel: texts.oursLabel })}</div>
          <div className="mt-editor" ref={hosts.left} />
        </div>
        <div className="mt-col mt-result">
          <div className="mt-col-head">{t("结果（可编辑，从共同祖先开始）")}</div>
          <div className="mt-editor" ref={hosts.mid} />
        </div>
        <div className="mt-col">
          <div className="mt-col-head">{t("右侧（他们的）：{theirsLabel}", { theirsLabel: texts.theirsLabel })}</div>
          <div className="mt-editor" ref={hosts.right} />
        </div>
      </div>
    </div>
  );
}

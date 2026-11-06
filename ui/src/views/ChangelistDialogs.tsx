// Dialogs of the Changes panel: move files to another changelist, name a
// changelist, shelve changes.
import { useEffect, useRef, useState } from "react";
import type { Changelists } from "../bindings/Changelists";
import { findByName } from "../lib/changelists";
import { Dialog } from "./LogDialogs";
import { t } from "../i18n";

function useAutoFocus() {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    requestAnimationFrame(() => {
      ref.current?.focus();
      ref.current?.select();
    });
  }, []);
  return ref;
}

/** "Move to Another Changelist": pick a list, or name a new one. */
export function MoveDialog({
  state,
  count,
  current,
  onCancel,
  onMove,
}: {
  state: Changelists;
  count: number;
  /** The list the files are in now, when they share one. */
  current: string | null;
  onCancel: () => void;
  /** An existing list id, or a name for a new list. */
  onMove: (target: { id: string } | { name: string }) => void;
}) {
  const others = state.lists.filter((l) => l.id !== current);
  const [choice, setChoice] = useState<string>(others[0]?.id ?? "__new");
  const [name, setName] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (choice === "__new") requestAnimationFrame(() => input.current?.focus());
  }, [choice]);
  const canSubmit = choice !== "__new" || name.trim().length > 0;
  return (
    <Dialog
      title={t("移动到另一个 Changelist")}
      submitLabel={t("移动")}
      canSubmit={canSubmit}
      onCancel={onCancel}
      onSubmit={() => {
        if (choice !== "__new") onMove({ id: choice });
        else {
          const existing = findByName(state, name);
          onMove(existing ? { id: existing.id } : { name: name.trim() });
        }
      }}
    >
      <div className="muted cl-dialog-note">{count === 1 ? t("移动 1 个文件") : t("移动 {count} 个文件", { count })}</div>
      <div className="cl-choices" role="radiogroup">
        {others.map((l) => (
          <label key={l.id} className="log-dialog-check">
            <input type="radio" name="cl" checked={choice === l.id} onChange={() => setChoice(l.id)} />
            {l.name}
            {l.id === state.active && <span className="muted">{t("（活动）")}</span>}
          </label>
        ))}
        <label className="log-dialog-check">
          <input type="radio" name="cl" checked={choice === "__new"} onChange={() => setChoice("__new")} />
          {t("新建 Changelist")}
        </label>
      </div>
      {choice === "__new" && (
        <label className="log-dialog-field">
          <span>{t("名称")}</span>
          <input ref={input} className="lf-input" value={name} onChange={(e) => setName(e.target.value)} placeholder={t("例如：解析器重构")} />
        </label>
      )}
    </Dialog>
  );
}

/** New changelist, or rename one. */
export function NameDialog({
  title,
  initial,
  state,
  except,
  onCancel,
  onSubmit,
  activeOption,
}: {
  title: string;
  initial: string;
  state: Changelists;
  /** The list being renamed (its own name is not a clash). */
  except?: string;
  onCancel: () => void;
  onSubmit: (name: string, makeActive: boolean) => void;
  activeOption?: boolean;
}) {
  const [name, setName] = useState(initial);
  const [makeActive, setMakeActive] = useState(false);
  const input = useAutoFocus();
  const clash = findByName(state, name);
  const taken = !!clash && clash.id !== except;
  return (
    <Dialog
      title={title}
      submitLabel={t("确定")}
      canSubmit={name.trim().length > 0 && !taken}
      onCancel={onCancel}
      onSubmit={() => onSubmit(name.trim(), makeActive)}
    >
      <label className="log-dialog-field">
        <span>{t("名称")}</span>
        <input ref={input} className="lf-input" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      {taken && <div className="log-dialog-error">{t("已有同名的 Changelist")}</div>}
      {activeOption && (
        <label className="log-dialog-check">
          <input type="checkbox" checked={makeActive} onChange={(e) => setMakeActive(e.target.checked)} />
          {t("设为活动 Changelist（新的修改会归入它）")}
        </label>
      )}
    </Dialog>
  );
}

/** "Shelve Changes": a name for the shelf entry. */
export function ShelveDialog({
  initial,
  count,
  onCancel,
  onSubmit,
}: {
  initial: string;
  count: number;
  onCancel: () => void;
  onSubmit: (name: string) => void;
}) {
  const [name, setName] = useState(initial);
  const input = useAutoFocus();
  return (
    <Dialog title={t("搁置修改（Shelve）")} submitLabel={t("搁置")} canSubmit={name.trim().length > 0} onCancel={onCancel} onSubmit={() => onSubmit(name.trim())}>
      <div className="muted cl-dialog-note">
        {t("{count} 个文件的修改会保存到 Shelf，并从工作区还原到 HEAD。之后可在 Stash 视图的 Shelf 页取回（可在撤销中心撤销）。", { count })}
      </div>
      <label className="log-dialog-field">
        <span>{t("说明")}</span>
        <input ref={input} className="lf-input" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
    </Dialog>
  );
}

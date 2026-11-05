// Interactive rebase plan: the rows the dialog edits (newest first, like the
// Log) and what the branch will look like afterwards.
import type { CommitSummary } from "../bindings/CommitSummary";
import type { RebaseAction } from "../bindings/RebaseAction";
import type { RebaseStep } from "../bindings/RebaseStep";

export interface PlanRow {
  commit: CommitSummary;
  action: RebaseAction;
  /** New message for "reword" (full message, may span lines). */
  message: string | null;
}

/** A commit of the result: the row it starts from plus rows melded into it. */
export interface ResultCommit {
  row: PlanRow;
  melded: PlanRow[];
  /** The first line of the message it will have. */
  subject: string;
  /** The rebase stops after this commit ("edit"). */
  stops: boolean;
}

/** Rows (newest first) → steps for the backend (oldest first). */
export function toSteps(rows: PlanRow[]): RebaseStep[] {
  return [...rows].reverse().map((r) => ({
    action: r.action,
    oid: r.commit.oid,
    message: r.action === "reword" ? r.message : null,
  }));
}

/** The commits the branch will have after the rebase, newest first. */
export function preview(rows: PlanRow[]): ResultCommit[] {
  const out: ResultCommit[] = [];
  for (const r of [...rows].reverse()) {
    if (r.action === "drop") continue;
    if ((r.action === "squash" || r.action === "fixup") && out.length) {
      out[out.length - 1].melded.push(r);
      continue;
    }
    const subject = r.action === "reword" && r.message?.trim() ? r.message.trim().split("\n")[0] : r.commit.subject;
    out.push({ row: r, melded: [], subject, stops: r.action === "edit" });
  }
  return out.reverse();
}

/** Why the plan cannot run, or null. */
export function problem(rows: PlanRow[]): string | null {
  const oldestKept = [...rows].reverse().find((r) => r.action !== "drop");
  if (!oldestKept) return "不能丢弃所有提交（如需这样做请使用重置）";
  if (oldestKept.action === "squash" || oldestKept.action === "fixup")
    return "最早的提交前面没有可合并的提交，不能 squash / fixup";
  const empty = rows.find((r) => r.action === "reword" && !r.message?.trim());
  if (empty) return `${empty.commit.oid.slice(0, 7)} 的新提交信息不能为空`;
  return null;
}

/** Whether the plan changes anything at all. */
export function isNoop(rows: PlanRow[], original: CommitSummary[]): boolean {
  return (
    rows.every((r) => r.action === "pick") &&
    rows.map((r) => r.commit.oid).join() === [...original].reverse().map((c) => c.oid).join()
  );
}

/** Move row `from` to index `to`. */
export function move<T>(list: T[], from: number, to: number): T[] {
  const out = [...list];
  const [it] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(out.length, to)), 0, it);
  return out;
}

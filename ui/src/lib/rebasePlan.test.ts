import { describe, expect, it } from "vitest";
import type { CommitSummary } from "../bindings/CommitSummary";
import { isNoop, move, preview, problem, toSteps, type PlanRow } from "./rebasePlan";

const commit = (n: number): CommitSummary => ({
  oid: `${n}`.repeat(40).slice(0, 40),
  parents: [],
  authorName: "A",
  authorEmail: "a@x",
  authorTime: n,
  subject: `c${n}`,
  refs: [],
});
// Plan from the backend: oldest first.
const original = [commit(1), commit(2), commit(3)];
const rows = (...actions: PlanRow["action"][]): PlanRow[] =>
  // Newest first, as the dialog shows them.
  [...original].reverse().map((c, i) => ({ commit: c, action: actions[i] ?? "pick", message: null }));

describe("rebase plan", () => {
  it("steps go oldest first", () => {
    expect(toSteps(rows()).map((s) => s.oid[0])).toEqual(["1", "2", "3"]);
  });

  it("squash and fixup meld into the commit below them", () => {
    // c3 fixup, c2 squash → both into c1.
    const p = preview(rows("fixup", "squash", "pick"));
    expect(p).toHaveLength(1);
    expect(p[0].row.commit.subject).toBe("c1");
    expect(p[0].melded.map((r) => r.commit.subject)).toEqual(["c2", "c3"]);
  });

  it("reword shows the new subject; drop removes; edit stops", () => {
    const r = rows("drop", "reword", "edit");
    r[1].message = "New title\n\nbody";
    const p = preview(r);
    expect(p.map((c) => c.subject)).toEqual(["New title", "c1"]);
    expect(p[1].stops).toBe(true);
    expect(toSteps(r)[1]).toEqual({ action: "reword", oid: commit(2).oid, message: "New title\n\nbody" });
  });

  it("finds problems", () => {
    expect(problem(rows())).toBeNull();
    expect(problem(rows("pick", "pick", "squash"))).toMatch(/squash/);
    expect(problem(rows("drop", "drop", "drop"))).toMatch(/丢弃/);
    // The oldest kept commit counts, not the oldest one.
    expect(problem(rows("pick", "fixup", "drop"))).toMatch(/squash/);
    const r = rows("reword");
    expect(problem(r)).toMatch(/不能为空/);
  });

  it("detects a plan that changes nothing", () => {
    expect(isNoop(rows(), original)).toBe(true);
    expect(isNoop(move(rows(), 0, 2), original)).toBe(false);
    expect(isNoop(rows("drop"), original)).toBe(false);
  });
});

// Typed wrappers over Tauri IPC. Domain types are generated from Rust (ui/src/bindings).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AskRequest } from "./bindings/AskRequest";
import type { DiffMode } from "./bindings/DiffMode";
import type { FileTexts } from "./bindings/FileTexts";
import type { AuthorCount } from "./bindings/AuthorCount";
import type { Blame } from "./bindings/Blame";
import type { BranchCompare } from "./bindings/BranchCompare";
import type { CommitDetails } from "./bindings/CommitDetails";
import type { CommitFile } from "./bindings/CommitFile";
import type { FileRevision } from "./bindings/FileRevision";
import type { CommitSummary } from "./bindings/CommitSummary";
import type { ConflictSide } from "./bindings/ConflictSide";
import type { ConflictTexts } from "./bindings/ConflictTexts";
import type { LogFilter } from "./bindings/LogFilter";
import type { LogPage } from "./bindings/LogPage";
import type { LogStatus } from "./bindings/LogStatus";
import type { Progress } from "./bindings/Progress";
import type { PushPreview } from "./bindings/PushPreview";
import type { OpEntry } from "./bindings/OpEntry";
import type { Changelists } from "./bindings/Changelists";
import type { ShelfEntry } from "./bindings/ShelfEntry";
import type { RepoSnapshot } from "./bindings/RepoSnapshot";
import type { RebaseStep } from "./bindings/RebaseStep";
import type { ResetMode } from "./bindings/ResetMode";
import type { StashEntry } from "./bindings/StashEntry";
import type { StashOptions } from "./bindings/StashOptions";

export interface AppInfo {
  version: string;
  gitVersion: string;
  platform: string;
}

export interface AppError {
  kind: string;
  message: string;
}

export type PullMode = "merge" | "rebase" | "ffOnly";

export function isCancelled(e: unknown): boolean {
  return !!e && typeof e === "object" && (e as AppError).kind === "cancelled";
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as AppError).message);
  return String(e);
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  openRepo: (path: string) => invoke<RepoSnapshot>("open_repo", { path }),
  closeRepo: (root: string) => invoke<boolean>("close_repo", { root }),
  refresh: (root: string) => invoke<RepoSnapshot>("refresh", { root }),
  stage: (root: string, paths: string[]) => invoke<void>("stage", { root, paths }),
  unstage: (root: string, paths: string[]) => invoke<void>("unstage", { root, paths }),
  stageAll: (root: string) => invoke<void>("stage_all", { root }),
  rollback: (root: string, tracked: string[], untracked: string[]) =>
    invoke<void>("rollback", { root, tracked, untracked }),
  commit: (root: string, message: string, amend: boolean) =>
    invoke<string>("commit", { root, message, amend }),
  lastCommitMessage: (root: string) => invoke<string>("last_commit_message", { root }),
  logStatus: (root: string) => invoke<LogStatus>("log_status", { root }),
  logPage: (root: string, start: number, count: number, filter: LogFilter | null) =>
    invoke<LogPage>("log_page", { root, start, count, filter }),
  logRowOf: (root: string, oid: string, filter: LogFilter | null) =>
    invoke<number | null>("log_row_of", { root, oid, filter }),
  logAuthors: (root: string, query: string) => invoke<AuthorCount[]>("log_authors", { root, query }),
  commitDetails: (root: string, oid: string) => invoke<CommitDetails>("commit_details", { root, oid }),
  conflictTexts: (root: string, path: string) => invoke<ConflictTexts>("conflict_texts", { root, path }),
  resolveConflict: (root: string, path: string, content: string | null) =>
    invoke<void>("resolve_conflict", { root, path, content }),
  resolveWithSide: (root: string, path: string, side: ConflictSide) =>
    invoke<void>("resolve_with_side", { root, path, side }),
  stashList: (root: string) => invoke<StashEntry[]>("stash_list", { root }),
  stashPush: (root: string, options: StashOptions) => invoke<void>("stash_push", { root, options }),
  stashApply: (root: string, index: number, oid: string, pop: boolean, restoreIndex: boolean) =>
    invoke<void>("stash_apply", { root, index, oid, pop, restoreIndex }),
  stashDrop: (root: string, index: number, oid: string) => invoke<void>("stash_drop", { root, index, oid }),
  stashDetails: (root: string, oid: string) => invoke<CommitDetails>("stash_details", { root, oid }),
  stageContent: (root: string, path: string, content: string) =>
    invoke<void>("stage_content", { root, path, content }),
  writeWorktreeFile: (root: string, path: string, content: string, expected: string) =>
    invoke<void>("write_worktree_file", { root, path, content, expected }),
  commitFileTexts: (root: string, oid: string, parent: string | null, path: string, origPath: string | null) =>
    invoke<FileTexts>("commit_file_texts", { root, oid, parent, path, origPath }),
  fileTexts: (root: string, path: string, origPath: string | null, mode: DiffMode, untracked: boolean) =>
    invoke<FileTexts>("file_texts", { root, path, origPath, mode, untracked }),
  checkoutBranch: (root: string, name: string, remote: boolean) =>
    invoke<void>("checkout_branch", { root, name, remote }),
  checkoutDetached: (root: string, oid: string) => invoke<void>("checkout_detached", { root, oid }),
  createTag: (root: string, name: string, oid: string, message: string | null) =>
    invoke<void>("create_tag", { root, name, oid, message }),
  cherryPick: (root: string, oid: string) => invoke<void>("cherry_pick", { root, oid }),
  revert: (root: string, oid: string) => invoke<void>("revert", { root, oid }),
  reset: (root: string, oid: string, mode: ResetMode) => invoke<void>("reset", { root, oid, mode }),
  abortOperation: (root: string) => invoke<void>("abort_operation", { root }),
  continueOperation: (root: string) => invoke<void>("continue_operation", { root }),
  skipOperation: (root: string) => invoke<void>("skip_operation", { root }),
  compareBranches: (root: string, left: string, right: string) =>
    invoke<BranchCompare>("compare_branches", { root, left, right }),
  diffFiles: (root: string, from: string, to: string) => invoke<CommitFile[]>("diff_files", { root, from, to }),
  fileHistory: (root: string, path: string, skip: number, limit: number) =>
    invoke<FileRevision[]>("file_history", { root, path, skip, limit }),
  lineHistory: (root: string, path: string, start: number, end: number, rev: string | null) =>
    invoke<CommitSummary[]>("line_history", { root, path, start, end, rev }),
  blame: (root: string, path: string, rev: string | null) => invoke<Blame>("blame", { root, path, rev }),
  rebasePlan: (root: string, oid: string) => invoke<CommitSummary[]>("rebase_plan", { root, oid }),
  interactiveRebase: (root: string, from: string, steps: RebaseStep[], autostash: boolean) =>
    invoke<boolean>("interactive_rebase", { root, from, steps, autostash }),
  createBranch: (root: string, name: string, start: string | null, checkout: boolean) =>
    invoke<void>("create_branch", { root, name, start, checkout }),
  deleteBranch: (root: string, name: string, force: boolean) =>
    invoke<void>("delete_branch", { root, name, force }),
  fetch: (root: string, taskId: number) => invoke<void>("fetch", { root, taskId }),
  pull: (root: string, mode: PullMode, taskId: number) => invoke<void>("pull", { root, mode, taskId }),
  push: (root: string, forceWithLease: boolean, taskId: number) =>
    invoke<void>("push", { root, forceWithLease, taskId }),
  pushPreview: (root: string) => invoke<PushPreview>("push_preview", { root }),
  operationLog: (root: string) => invoke<OpEntry[]>("operation_log", { root }),
  undoOperation: (root: string, id: number) => invoke<string>("undo_operation", { root, id }),
  changelists: (root: string) => invoke<Changelists>("changelists", { root }),
  saveChangelists: (root: string, changelists: Changelists) => invoke<Changelists>("save_changelists", { root, changelists }),
  shelfList: (root: string) => invoke<ShelfEntry[]>("shelf_list", { root }),
  shelve: (root: string, paths: string[], name: string, changelist: string | null) =>
    invoke<ShelfEntry>("shelve", { root, paths, name, changelist }),
  unshelve: (root: string, id: string, oid: string, name: string, keep: boolean) =>
    invoke<boolean>("unshelve", { root, id, oid, name, keep }),
  dropShelf: (root: string, id: string, oid: string, name: string) => invoke<void>("drop_shelf", { root, id, oid, name }),
  cancelTask: (taskId: number) => invoke<boolean>("cancel_task", { taskId }),
  askpassRespond: (id: number, answer: string | null) => invoke<boolean>("askpass_respond", { id, answer }),
};

export function onAskpass(cb: (req: AskRequest) => void): Promise<UnlistenFn> {
  return listen<AskRequest>("askpass-request", (e) => cb(e.payload));
}

export function onAskpassDismiss(cb: (id: number) => void): Promise<UnlistenFn> {
  return listen<{ id: number }>("askpass-dismiss", (e) => cb(e.payload.id));
}

export function onTaskProgress(cb: (taskId: number, progress: Progress) => void): Promise<UnlistenFn> {
  return listen<{ taskId: number; progress: Progress }>("task-progress", (e) =>
    cb(e.payload.taskId, e.payload.progress),
  );
}

export function onSnapshot(
  cb: (root: string, snapshot: RepoSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<{ root: string; snapshot: RepoSnapshot }>("repo-snapshot", (e) =>
    cb(e.payload.root, e.payload.snapshot),
  );
}

export function onLogStatus(cb: (status: LogStatus) => void): Promise<UnlistenFn> {
  return listen<LogStatus>("log-status", (e) => cb(e.payload));
}

// Typed wrappers over Tauri IPC. Domain types are generated from Rust (ui/src/bindings).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CommitSummary } from "./bindings/CommitSummary";
import type { DiffMode } from "./bindings/DiffMode";
import type { FileDiff } from "./bindings/FileDiff";
import type { RepoSnapshot } from "./bindings/RepoSnapshot";

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
  diffFile: (root: string, path: string, mode: DiffMode, untracked: boolean) =>
    invoke<FileDiff>("diff_file", { root, path, mode, untracked }),
  log: (root: string, limit: number) => invoke<CommitSummary[]>("log", { root, limit }),
  checkoutBranch: (root: string, name: string, remote: boolean) =>
    invoke<void>("checkout_branch", { root, name, remote }),
  createBranch: (root: string, name: string, start: string | null, checkout: boolean) =>
    invoke<void>("create_branch", { root, name, start, checkout }),
  deleteBranch: (root: string, name: string, force: boolean) =>
    invoke<void>("delete_branch", { root, name, force }),
  fetch: (root: string) => invoke<void>("fetch", { root }),
  pull: (root: string, mode: PullMode) => invoke<void>("pull", { root, mode }),
  push: (root: string, forceWithLease: boolean) => invoke<void>("push", { root, forceWithLease }),
};

export function onSnapshot(
  cb: (root: string, snapshot: RepoSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<{ root: string; snapshot: RepoSnapshot }>("repo-snapshot", (e) =>
    cb(e.payload.root, e.payload.snapshot),
  );
}

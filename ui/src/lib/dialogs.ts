// System file dialogs, centered over the app window (see crates/app/src/dialogs.rs).
import { invoke } from "@tauri-apps/api/core";
import { open, type OpenDialogOptions } from "@tauri-apps/plugin-dialog";

/** Pick one file or folder; null when cancelled. */
export async function pickPath(options: Omit<OpenDialogOptions, "multiple">): Promise<string | null> {
  await invoke("center_next_dialog").catch(() => {});
  const picked = await open({ ...options, multiple: false });
  return typeof picked === "string" ? picked : null;
}

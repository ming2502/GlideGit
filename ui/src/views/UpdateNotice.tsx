// Automatic updates: a quiet check after start-up, a notice when a newer
// version is out, and "Check for Updates…" in the command palette.
import { useEffect, useState } from "react";
import { api, errorMessage, type UpdateInfo } from "../api";
import { useActions } from "../actions/registry";
import { Notes } from "../components/Notes";
import { load, save } from "../lib/storage";
import { useRepo } from "../stores/repo";
import { t } from "../i18n";

/** Wait this long after start-up before checking, so it never competes with opening a repository. */
const FIRST_CHECK_DELAY = 5_000;
/** And check again this often while the app stays open. */
const RECHECK_EVERY = 6 * 60 * 60 * 1000;

export function UpdateNotice() {
  const [enabled, setEnabled] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [notesOpen, setNotesOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api
      .updatesEnabled()
      .then((on) => alive && setEnabled(on))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  /** `manual`: say so when there is nothing new or the check fails. */
  const check = async (manual: boolean) => {
    const { toast } = useRepo.getState();
    try {
      const found = await api.checkUpdate();
      if (!found) {
        if (manual) toast({ kind: "success", title: t("已是最新版本") });
        return;
      }
      // A skipped version is only offered again when asked for.
      if (!manual && load<string | null>("skippedUpdate", null) === found.version) return;
      setUpdate(found);
    } catch (e) {
      if (manual) toast({ kind: "error", title: t("检查更新失败"), detail: errorMessage(e) });
    }
  };

  useEffect(() => {
    if (!enabled || !load("autoCheckUpdates", true)) return;
    const first = setTimeout(() => void check(false), FIRST_CHECK_DELAY);
    const again = setInterval(() => void check(false), RECHECK_EVERY);
    return () => {
      clearTimeout(first);
      clearInterval(again);
    };
  }, [enabled]);

  useActions([
    {
      id: "app.checkUpdates",
      title: "Check for Updates…",
      keywords: "检查更新 升级 新版本 update",
      enabled: () => enabled,
      run: () => void check(true),
    },
  ]);

  if (!update) return null;

  const install = async () => {
    setBusy(true);
    const st = useRepo.getState();
    const exiting = await st.runAppTask(t("更新到 {version}", { version: update.version }), (taskId) => api.installUpdate(taskId));
    // On Windows the installer takes over (and restarts the app); elsewhere restart into it.
    if (exiting === false) {
      await api.restartApp().catch((e) => st.toast({ kind: "error", title: t("请手动重启 GlideGit"), detail: errorMessage(e) }));
    }
    setBusy(false);
  };
  const skip = () => {
    save("skippedUpdate", update.version);
    setUpdate(null);
  };

  return (
    <div className="update-notice" role="status">
      <div className="un-title">
        <b>{t("GlideGit {version} 可以更新了", { version: update.version })}</b>
        <span className="muted">{t("当前 {currentVersion}", { currentVersion: update.currentVersion })}</span>
      </div>
      {update.notes && (
        <button className="un-link" onClick={() => setNotesOpen((o) => !o)}>
          {notesOpen ? t("收起更新说明") : t("查看更新说明")}
        </button>
      )}
      {notesOpen && update.notes && <Notes markdown={update.notes} className="notes un-notes" />}
      <div className="un-actions">
        <button className="btn btn-sm" onClick={skip} disabled={busy} title={t("不再提示这个版本（仍可在命令面板中手动检查）")}>
          {t("跳过此版本")}
        </button>
        <button className="btn btn-sm" onClick={() => setUpdate(null)} disabled={busy}>
          {t("稍后")}
        </button>
        <button className="btn btn-sm btn-primary" onClick={() => void install()} disabled={busy}>
          {busy ? t("正在更新…") : t("安装并重启")}
        </button>
      </div>
    </div>
  );
}

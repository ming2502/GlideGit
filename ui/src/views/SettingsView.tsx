// Settings: appearance, git, shortcuts, updates, about.
import { useEffect, useState, type KeyboardEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type AppInfo } from "../api";
import {
  chordFromEvent,
  chordOwner,
  formatKeys,
  isCustomized,
  resetAllShortcuts,
  runAction,
  setShortcut,
  shortcutCatalog,
  useKeymapVersion,
} from "../actions/registry";
import { load, save } from "../lib/storage";
import { currentTheme, setTheme, type Theme } from "../lib/theme";
import { useRepo, type SettingsSection } from "../stores/repo";
import { languagePref, setLanguagePref, t } from "../i18n";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "appearance", label: t("外观") },
  { id: "git", label: "Git" },
  { id: "keymap", label: t("快捷键") },
  { id: "updates", label: t("更新") },
  { id: "about", label: t("关于") },
];

export function SettingsView({ info }: { info: AppInfo | null }) {
  const section = useRepo((s) => s.settings);
  const setSection = useRepo((s) => s.setSettings);
  if (!section) return null;
  const close = () => setSection(null);
  return (
    <div
      className="mt-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("设置")}
      onKeyDown={(e) => {
        // The shortcut recorder handles its own Escape.
        if (e.key === "Escape" && !(e.target as HTMLElement).closest(".km-recording")) close();
      }}
    >
      <div className="mt-window settings">
        <div className="mt-header">
          <b>{t("设置")}</b>
          <span className="spacer" />
          <button className="btn btn-sm" onClick={close} autoFocus>
            {t("关闭")}
          </button>
        </div>
        <div className="st-body">
          <nav className="st-nav">
            {SECTIONS.map((s) => (
              <button key={s.id} className={s.id === section ? "on" : ""} onClick={() => setSection(s.id)}>
                {s.label}
              </button>
            ))}
          </nav>
          <div className="st-page">
            {section === "appearance" && <Appearance />}
            {section === "git" && <GitSettings info={info} />}
            {section === "keymap" && <Keymap />}
            {section === "updates" && <Updates />}
            {section === "about" && <About info={info} />}
          </div>
        </div>
      </div>
    </div>
  );
}

function Appearance() {
  const [theme, set] = useState<Theme>(currentTheme);
  const choose = (next: Theme) => {
    set(next);
    setTheme(next);
  };
  const lang = languagePref();
  return (
    <section>
      <h3>{t("主题")}</h3>
      <div className="st-choices" role="radiogroup">
        {(
          [
            ["system", t("跟随系统")],
            ["light", t("浅色")],
            ["dark", t("深色")],
          ] as const
        ).map(([value, label]) => (
          <label key={value} className="log-dialog-check">
            <input type="radio" name="theme" checked={theme === value} onChange={() => choose(value)} />
            {label}
          </label>
        ))}
      </div>
      <h3>{t("语言")}</h3>
      <div className="st-choices" role="radiogroup">
        {(
          [
            // Each language names itself, so it can be found whatever the current one is.
            ["system", t("跟随系统")],
            ["zh", "简体中文"], // i18n-ignore: a language names itself
            ["en", "English"],
          ] as const
        ).map(([value, label]) => (
          <label key={value} className="log-dialog-check">
            <input type="radio" name="language" checked={lang === value} onChange={() => setLanguagePref(value)} />
            {label}
          </label>
        ))}
      </div>
      <p className="muted st-note">{t("切换语言会重新加载窗口。")}</p>
    </section>
  );
}

function GitSettings({ info }: { info: AppInfo | null }) {
  const [saved, setSaved] = useState<string | null | undefined>(undefined);
  const [mode, setMode] = useState<"auto" | "custom">("auto");
  const [path, setPath] = useState("");
  const [check, setCheck] = useState<{ ok: boolean; text: string } | null>(null);
  const [changed, setChanged] = useState(false);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSaved(s.gitPath);
        setMode(s.gitPath ? "custom" : "auto");
        setPath(s.gitPath ?? "");
      })
      .catch(() => setSaved(null));
  }, []);

  const verify = async (p: string) => {
    setCheck(null);
    try {
      const v = await api.checkGit(p);
      setCheck({ ok: true, text: `git ${v}` });
      return true;
    } catch (e) {
      setCheck({ ok: false, text: errorMessage(e) });
      return false;
    }
  };
  const browse = async () => {
    const picked = await open({ multiple: false, directory: false, title: t("选择 git 可执行文件") });
    if (typeof picked === "string") {
      setPath(picked);
      void verify(picked);
    }
  };
  const apply = async () => {
    const gitPath = mode === "custom" ? path.trim() : null;
    if (gitPath && !(await verify(gitPath))) return;
    await api.saveSettings({ gitPath });
    setSaved(gitPath);
    setChanged(gitPath !== (info?.gitPath ?? null) || !!info?.gitError);
  };
  const dirty = saved !== undefined && (mode === "custom" ? path.trim() : null) !== (saved ?? null);

  return (
    <section>
      <h3>{t("正在使用")}</h3>
      {info?.gitError ? (
        <p className="st-error">{info.gitError}</p>
      ) : (
        <p>
          git {info?.gitVersion} <span className="muted mono">{info?.gitPath}</span>
          {info?.gitBundled && <span className="muted">{t("（GlideGit 自带的 MinGit）")}</span>}
        </p>
      )}
      <h3>{t("git 可执行文件")}</h3>
      <div className="st-choices" role="radiogroup">
        <label className="log-dialog-check">
          <input type="radio" name="git" checked={mode === "auto"} onChange={() => setMode("auto")} />
          {t("自动查找（PATH 和常见安装位置{note}）", { note: info?.platform === "windows" ? t("，找不到时使用自带的 MinGit") : "" })}
        </label>
        <label className="log-dialog-check">
          <input type="radio" name="git" checked={mode === "custom"} onChange={() => setMode("custom")} />
          {t("指定路径")}
        </label>
      </div>
      {mode === "custom" && (
        <div className="st-row">
          <input
            className="lf-input mono st-path"
            value={path}
            placeholder={info?.platform === "windows" ? "C:\\Program Files\\Git\\cmd\\git.exe" : "/usr/local/bin/git"}
            onChange={(e) => {
              setPath(e.target.value);
              setCheck(null);
            }}
            spellCheck={false}
          />
          <button className="btn btn-sm" onClick={() => void browse()}>
            {t("浏览…")}
          </button>
          <button className="btn btn-sm" onClick={() => void verify(path)} disabled={!path.trim()}>
            {t("检测")}
          </button>
        </div>
      )}
      {check && <p className={check.ok ? "st-ok" : "st-error"}>{check.ok ? `✓ ${check.text}` : check.text}</p>}
      <div className="st-row">
        <button className="btn btn-primary btn-sm" disabled={!dirty || (mode === "custom" && !path.trim())} onClick={() => void apply()}>
          {t("保存")}
        </button>
        {changed && (
          <>
            <span className="muted">{t("重启 GlideGit 后生效。")}</span>
            <button className="btn btn-sm" onClick={() => void api.restartApp()}>
              {t("立即重启")}
            </button>
          </>
        )}
      </div>
    </section>
  );
}

function Keymap() {
  useKeymapVersion();
  const [query, setQuery] = useState("");
  const [recording, setRecording] = useState<string | null>(null);
  const [conflict, setConflict] = useState<{ id: string; chord: string; owner: string } | null>(null);
  const all = shortcutCatalog();
  const title = (id: string) => all.find((a) => a.id === id)?.title ?? id;
  const q = query.trim().toLowerCase();
  const shown = all.filter((a) => !q || a.title.toLowerCase().includes(q) || formatKeys(a.keys).toLowerCase().includes(q));

  const onKey = (id: string, e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setRecording(null);
      return;
    }
    const chord = chordFromEvent(e.nativeEvent);
    if (!chord) return;
    // A plain key (no modifier) would fire while typing in other places; only F-keys are allowed bare.
    if (!/(^|\+)(mod|ctrl|alt)\+/.test(chord) && !/^(shift\+)?f\d+$/.test(chord)) return;
    const owner = chordOwner(chord, id);
    setRecording(null);
    if (owner) setConflict({ id, chord, owner });
    else setShortcut(id, [chord]);
  };

  return (
    <section>
      <div className="st-row">
        <input className="lf-input" placeholder={t("搜索操作或快捷键")} value={query} onChange={(e) => setQuery(e.target.value)} />
        <span className="spacer" />
        <button className="btn btn-sm" onClick={resetAllShortcuts}>
          {t("全部恢复默认")}
        </button>
      </div>
      <p className="muted st-note">{t("默认为 IntelliJ IDEA 的键位。点击快捷键后按下新的组合键（需要包含 {formatKeys}、⌥/Alt 或 F 键），Esc 取消。", { formatKeys: formatKeys(["mod"]) })}</p>
      {conflict && (
        <div className="st-conflict">
          {t("{formatKeys} 已用于「{title}」。", { formatKeys: formatKeys([conflict.chord]), title: title(conflict.owner) })}
          <button
            className="btn btn-sm"
            onClick={() => {
              const owner = all.find((a) => a.id === conflict.owner);
              setShortcut(conflict.owner, (owner?.keys ?? []).filter((k) => k !== conflict.chord));
              setShortcut(conflict.id, [conflict.chord]);
              setConflict(null);
            }}
          >
            {t("改用于「{title}」", { title: title(conflict.id) })}
          </button>
          <button className="btn btn-sm" onClick={() => setConflict(null)}>
            {t("取消")}
          </button>
        </div>
      )}
      <div className="km-list">
        {shown.map((a) => (
          <div key={a.id} className="km-row">
            <span className="km-title">{a.title}</span>
            {recording === a.id ? (
              <span className="km-keys km-recording" tabIndex={0} autoFocus ref={(el) => el?.focus()} onKeyDown={(e) => onKey(a.id, e)} onBlur={() => setRecording(null)}>
                {t("请按下组合键…")}
              </span>
            ) : (
              <button className="km-keys" onClick={() => setRecording(a.id)} title={t("点击后按下新的快捷键")}>
                {a.keys.length ? a.keys.map((k) => formatKeys([k])).join("  ") : <span className="muted">{t("无")}</span>}
              </button>
            )}
            <span className="km-actions">
              {a.keys.length > 0 && (
                <button className="btn-link" onClick={() => setShortcut(a.id, [])} title={t("移除快捷键")}>
                  {t("移除")}
                </button>
              )}
              {isCustomized(a.id) && (
                <button className="btn-link" onClick={() => setShortcut(a.id, null)} title={t("恢复为 {keys}", { keys: formatKeys(a.defaultKeys) || t("无") })}>
                  {t("恢复默认")}
                </button>
              )}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

function Updates() {
  const [auto, setAuto] = useState(() => load("autoCheckUpdates", true));
  const [enabled, setEnabled] = useState<boolean | null>(null);
  useEffect(() => {
    api.updatesEnabled().then(setEnabled).catch(() => setEnabled(false));
  }, []);
  return (
    <section>
      <h3>{t("自动更新")}</h3>
      {enabled === false ? (
        <p className="muted">{t("这个版本没有启用自动更新（开发版本或自行构建的版本）。")}</p>
      ) : (
        <>
          <label className="log-dialog-check">
            <input
              type="checkbox"
              checked={auto}
              onChange={(e) => {
                setAuto(e.target.checked);
                save("autoCheckUpdates", e.target.checked);
              }}
            />
            {t("启动时和每隔 6 小时自动检查新版本（重启后生效）")}
          </label>
          <div className="st-row">
            <button
              className="btn btn-sm"
              onClick={() => {
                // Close first: the result shows as a notice or toast on the main window.
                useRepo.getState().setSettings(null);
                runAction("app.checkUpdates");
              }}
            >
              {t("立即检查")}
            </button>
          </div>
        </>
      )}
    </section>
  );
}

function About({ info }: { info: AppInfo | null }) {
  return (
    <section>
      <h3>GlideGit {info?.version}</h3>
      <p className="muted">{t("像 IntelliJ IDEA 的 Git 界面一样顺手的跨平台 Git 客户端。")}</p>
      <p>
        git {info?.gitVersion || "—"} <span className="muted mono">{info?.gitPath}</span>
      </p>
      <p className="muted">{t("平台：{platform}", { platform: info?.platform })}</p>
      <p className="muted">
        {t("源代码与问题反馈：")}<span className="mono">github.com/ming2502/GlideGit</span>
      </p>
    </section>
  );
}

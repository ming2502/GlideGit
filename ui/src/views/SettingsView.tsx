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

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "appearance", label: "外观" },
  { id: "git", label: "Git" },
  { id: "keymap", label: "快捷键" },
  { id: "updates", label: "更新" },
  { id: "about", label: "关于" },
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
      aria-label="设置"
      onKeyDown={(e) => {
        // The shortcut recorder handles its own Escape.
        if (e.key === "Escape" && !(e.target as HTMLElement).closest(".km-recording")) close();
      }}
    >
      <div className="mt-window settings">
        <div className="mt-header">
          <b>设置</b>
          <span className="spacer" />
          <button className="btn btn-sm" onClick={close} autoFocus>
            关闭
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
  const choose = (t: Theme) => {
    set(t);
    setTheme(t);
  };
  return (
    <section>
      <h3>主题</h3>
      <div className="st-choices" role="radiogroup">
        {(
          [
            ["system", "跟随系统"],
            ["light", "浅色"],
            ["dark", "深色"],
          ] as const
        ).map(([t, label]) => (
          <label key={t} className="log-dialog-check">
            <input type="radio" name="theme" checked={theme === t} onChange={() => choose(t)} />
            {label}
          </label>
        ))}
      </div>
      <p className="muted st-note">界面语言目前只有简体中文。</p>
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
    const picked = await open({ multiple: false, directory: false, title: "选择 git 可执行文件" });
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
      <h3>正在使用</h3>
      {info?.gitError ? (
        <p className="st-error">{info.gitError}</p>
      ) : (
        <p>
          git {info?.gitVersion} <span className="muted mono">{info?.gitPath}</span>
          {info?.gitBundled && <span className="muted">（GlideGit 自带的 MinGit）</span>}
        </p>
      )}
      <h3>git 可执行文件</h3>
      <div className="st-choices" role="radiogroup">
        <label className="log-dialog-check">
          <input type="radio" name="git" checked={mode === "auto"} onChange={() => setMode("auto")} />
          自动查找（PATH 和常见安装位置{info?.platform === "windows" ? "，找不到时使用自带的 MinGit" : ""}）
        </label>
        <label className="log-dialog-check">
          <input type="radio" name="git" checked={mode === "custom"} onChange={() => setMode("custom")} />
          指定路径
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
            浏览…
          </button>
          <button className="btn btn-sm" onClick={() => void verify(path)} disabled={!path.trim()}>
            检测
          </button>
        </div>
      )}
      {check && <p className={check.ok ? "st-ok" : "st-error"}>{check.ok ? `✓ ${check.text}` : check.text}</p>}
      <div className="st-row">
        <button className="btn btn-primary btn-sm" disabled={!dirty || (mode === "custom" && !path.trim())} onClick={() => void apply()}>
          保存
        </button>
        {changed && (
          <>
            <span className="muted">重启 GlideGit 后生效。</span>
            <button className="btn btn-sm" onClick={() => void api.restartApp()}>
              立即重启
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
        <input className="lf-input" placeholder="搜索操作或快捷键" value={query} onChange={(e) => setQuery(e.target.value)} />
        <span className="spacer" />
        <button className="btn btn-sm" onClick={resetAllShortcuts}>
          全部恢复默认
        </button>
      </div>
      <p className="muted st-note">默认为 IntelliJ IDEA 的键位。点击快捷键后按下新的组合键（需要包含 {formatKeys(["mod"])}、⌥/Alt 或 F 键），Esc 取消。</p>
      {conflict && (
        <div className="st-conflict">
          {formatKeys([conflict.chord])} 已用于「{title(conflict.owner)}」。
          <button
            className="btn btn-sm"
            onClick={() => {
              const owner = all.find((a) => a.id === conflict.owner);
              setShortcut(conflict.owner, (owner?.keys ?? []).filter((k) => k !== conflict.chord));
              setShortcut(conflict.id, [conflict.chord]);
              setConflict(null);
            }}
          >
            改用于「{title(conflict.id)}」
          </button>
          <button className="btn btn-sm" onClick={() => setConflict(null)}>
            取消
          </button>
        </div>
      )}
      <div className="km-list">
        {shown.map((a) => (
          <div key={a.id} className="km-row">
            <span className="km-title">{a.title}</span>
            {recording === a.id ? (
              <span className="km-keys km-recording" tabIndex={0} autoFocus ref={(el) => el?.focus()} onKeyDown={(e) => onKey(a.id, e)} onBlur={() => setRecording(null)}>
                请按下组合键…
              </span>
            ) : (
              <button className="km-keys" onClick={() => setRecording(a.id)} title="点击后按下新的快捷键">
                {a.keys.length ? a.keys.map((k) => formatKeys([k])).join("  ") : <span className="muted">无</span>}
              </button>
            )}
            <span className="km-actions">
              {a.keys.length > 0 && (
                <button className="btn-link" onClick={() => setShortcut(a.id, [])} title="移除快捷键">
                  移除
                </button>
              )}
              {isCustomized(a.id) && (
                <button className="btn-link" onClick={() => setShortcut(a.id, null)} title={`恢复为 ${formatKeys(a.defaultKeys) || "无"}`}>
                  恢复默认
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
      <h3>自动更新</h3>
      {enabled === false ? (
        <p className="muted">这个版本没有启用自动更新（开发版本或自行构建的版本）。</p>
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
            启动时和每隔 6 小时自动检查新版本（重启后生效）
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
              立即检查
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
      <p className="muted">像 IntelliJ IDEA 的 Git 界面一样顺手的跨平台 Git 客户端。</p>
      <p>
        git {info?.gitVersion || "—"} <span className="muted mono">{info?.gitPath}</span>
      </p>
      <p className="muted">平台：{info?.platform}</p>
      <p className="muted">
        源代码与问题反馈：<span className="mono">github.com/ming2502/GlideGit</span>
      </p>
    </section>
  );
}

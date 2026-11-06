import { shortcutOf } from "../actions/registry";
import type { AppInfo } from "../api";
import { repoName } from "../lib/format";
import { useRepo } from "../stores/repo";
import { pickRepository } from "./Chrome";
import { t } from "../i18n";

export function Welcome({ info }: { info: AppInfo | null }) {
  const gitVersion = info?.gitVersion ?? "";
  const recent = useRepo((s) => s.recent);
  const openRepo = useRepo((s) => s.openRepo);
  return (
    <div className="welcome">
      <div className="welcome-card">
        <div className="logo" aria-hidden>
          <svg viewBox="0 0 1024 1024" width="56" height="56">
            <defs>
              <linearGradient id="lg" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" stopColor="#4F7CFF" />
                <stop offset="1" stopColor="#7B4DFF" />
              </linearGradient>
            </defs>
            <rect x="64" y="64" width="896" height="896" rx="200" fill="url(#lg)" />
            <g fill="none" stroke="#fff" strokeWidth="64" strokeLinecap="round">
              <path d="M352 260 V764" />
              <path d="M352 640 C352 520 672 540 672 400" />
            </g>
            <g fill="#fff">
              <circle cx="352" cy="260" r="72" />
              <circle cx="352" cy="764" r="72" />
              <circle cx="672" cy="360" r="72" />
            </g>
          </svg>
        </div>
        <h1>GlideGit</h1>
        {info?.gitError ? (
          <div className="welcome-git-error">
            <p>{info.gitError}</p>
            <button className="btn btn-sm" onClick={() => useRepo.getState().setSettings("git")}>
              {t("打开 Git 设置")}
            </button>
          </div>
        ) : (
          <p className="muted">{gitVersion ? `git ${gitVersion}` : t("正在检测 git…")}</p>
        )}
        <button className="btn btn-primary btn-lg" onClick={() => void pickRepository()}>
          {t("打开仓库…")}<span className="kbd">{shortcutOf("repo.open")}</span>
        </button>
        {recent.length > 0 && (
          <div className="recent">
            <div className="recent-title muted">{t("最近打开")}</div>
            {recent.map((r) => (
              <button key={r} className="recent-item" onClick={() => void openRepo(r)}>
                <b>{repoName(r)}</b>
                <span className="muted">{r}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

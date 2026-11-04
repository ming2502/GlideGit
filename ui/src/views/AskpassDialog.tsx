// Credential prompts from git/ssh (username, password/token, key passphrase,
// host-key confirmation). Answers go straight back to git and are never stored.
import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { api, onAskpass, onAskpassDismiss } from "../api";
import type { AskRequest } from "../bindings/AskRequest";

const useAsk = create<{ queue: AskRequest[] }>(() => ({ queue: [] }));

function remove(id: number) {
  useAsk.setState((s) => ({ queue: s.queue.filter((r) => r.id !== id) }));
}

function respond(id: number, answer: string | null) {
  remove(id);
  void api.askpassRespond(id, answer);
}

/** `https://min@github.com` → `github.com` for titles. */
function hostOf(target: string | null): string {
  if (!target) return "远端仓库";
  try {
    return new URL(target).host || target;
  } catch {
    return target;
  }
}

export function AskpassDialog() {
  const queue = useAsk((s) => s.queue);
  const req = queue[0];
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const a = onAskpass((r) => useAsk.setState((s) => ({ queue: [...s.queue, r] })));
    const d = onAskpassDismiss(remove);
    return () => {
      void a.then((f) => f());
      void d.then((f) => f());
    };
  }, []);

  useEffect(() => {
    setValue("");
    requestAnimationFrame(() => (inputRef.current ?? confirmRef.current)?.focus());
  }, [req?.id]);

  if (!req) return null;

  const cancel = () => respond(req.id, null);
  const submit = () => respond(req.id, value);

  let title: string;
  let label = "";
  let hint: string | null = null;
  let secret = false;
  switch (req.kind) {
    case "username":
      title = `登录 ${hostOf(req.target)}`;
      label = "用户名";
      break;
    case "password":
      title = `登录 ${hostOf(req.target)}`;
      label = "密码或访问令牌";
      secret = true;
      hint =
        "GitHub、GitLab 等平台需要使用个人访问令牌（Personal Access Token）代替密码。" +
        "如果配置了凭据助手（如 macOS 钥匙串、Git Credential Manager），验证成功后会由它保存。";
      break;
    case "passphrase":
      title = "SSH 密钥口令";
      label = req.target ? `密钥 ${req.target}` : "口令";
      secret = true;
      break;
    case "confirm":
      title = req.target ? `确认 SSH 主机 ${req.target}` : "需要确认";
      break;
    default:
      title = "git 需要输入";
      label = req.prompt.trim();
  }

  return (
    <div className="backdrop">
      <div
        className="dialog ask-dialog"
        role="dialog"
        aria-modal
        aria-label={title}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          }
        }}
      >
        <h3>
          {title}
          {queue.length > 1 && <span className="muted"> （还有 {queue.length - 1} 个请求）</span>}
        </h3>
        {req.kind === "confirm" ? (
          <>
            <pre className="ask-prompt">{req.prompt.trim()}</pre>
            <p className="muted">首次连接该主机。请核对指纹与服务商公布的一致后再信任。</p>
            <div className="dialog-actions">
              <button className="btn" onClick={() => respond(req.id, "no")}>
                拒绝
              </button>
              <button ref={confirmRef} className="btn btn-primary" onClick={() => respond(req.id, "yes")}>
                信任并继续
              </button>
            </div>
          </>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            {req.target && req.kind !== "passphrase" && <div className="muted ask-target">{req.target}</div>}
            <label className="ask-label">
              {label}
              <input
                ref={inputRef}
                className="ask-input"
                type={secret ? "password" : "text"}
                autoComplete="off"
                spellCheck={false}
                value={value}
                onChange={(e) => setValue(e.target.value)}
              />
            </label>
            {hint && <p className="muted ask-hint">{hint}</p>}
            <div className="dialog-actions">
              <button type="button" className="btn" onClick={cancel}>
                取消
              </button>
              <button type="submit" className="btn btn-primary">
                继续
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}

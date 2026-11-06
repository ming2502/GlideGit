import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Tooltips } from "./components/Tooltips";
import { applyTheme } from "./lib/theme";
import { language } from "./i18n";
import { api } from "./api";
import "./styles.css";

async function boot() {
  applyTheme();
  document.documentElement.lang = language === "en" ? "en" : "zh-CN";
  // Outside Tauri (plain browser during `pnpm dev`) serve IPC from a fake repo.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window && (window as any).__TAURI_INTERNALS__?.invoke)) {
    const { installMock } = await import("./dev/mock");
    installMock();
  }
  // Text the backend writes (operation titles, undo descriptions) follows the UI.
  await api.setLanguage(language).catch(() => {});
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
      <Tooltips />
    </StrictMode>,
  );
}

void boot();

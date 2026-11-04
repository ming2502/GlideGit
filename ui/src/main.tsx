import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

async function boot() {
  // Outside Tauri (plain browser during `pnpm dev`) serve IPC from a fake repo.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window && (window as any).__TAURI_INTERNALS__?.invoke)) {
    const { installMock } = await import("./dev/mock");
    installMock();
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void boot();

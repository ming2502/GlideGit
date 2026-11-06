// Theme: follow the system, or force light / dark. styles.css reads
// `data-theme` on <html>; without it, `prefers-color-scheme` decides.
import { load, save } from "./storage";

export type Theme = "system" | "light" | "dark";

export function currentTheme(): Theme {
  const t = load<Theme>("theme", "system");
  return t === "light" || t === "dark" ? t : "system";
}

export function applyTheme(theme: Theme = currentTheme()): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

export function setTheme(theme: Theme): void {
  save("theme", theme);
  applyTheme(theme);
}

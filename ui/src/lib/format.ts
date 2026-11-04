export function splitPath(path: string): { name: string; dir: string } {
  const i = path.lastIndexOf("/");
  return i < 0 ? { name: path, dir: "" } : { name: path.slice(i + 1), dir: path.slice(0, i) };
}

export function repoName(root: string): string {
  const parts = root.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? root;
}

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

export function relativeTime(unixSeconds: number): string {
  const diff = unixSeconds - Date.now() / 1000;
  const abs = Math.abs(diff);
  if (abs < 60) return rtf.format(Math.round(diff), "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 7) return rtf.format(Math.round(diff / 86400), "day");
  return new Date(unixSeconds * 1000).toLocaleDateString();
}

export const isMac = navigator.platform.toLowerCase().includes("mac");

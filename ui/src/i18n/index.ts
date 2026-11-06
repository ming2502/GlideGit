// Interface language. Strings are written in Chinese in the code and looked
// up as-is: `t("已切换到 {name}", { name })`. English comes from en.ts; a
// string without a translation shows in Chinese (a test keeps en.ts complete).
//
// The language is fixed for the life of the page: changing it reloads.
import { load, save } from "../lib/storage";
import { en } from "./en";

export type Language = "zh" | "en";
export type LanguagePref = "system" | Language;

export function languagePref(): LanguagePref {
  const p = load<string>("language", "system");
  return p === "zh" || p === "en" ? p : "system";
}

function systemLanguage(): Language {
  // Unit tests check the Chinese source strings.
  if (import.meta.env?.MODE === "test") return "zh";
  const first = typeof navigator === "undefined" ? "zh" : (navigator.languages?.[0] ?? navigator.language ?? "");
  return first.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export const language: Language = ((p) => (p === "system" ? systemLanguage() : p))(languagePref());

/** For dates, times and numbers in the interface language. */
export const locale = language === "en" ? "en-US" : "zh-CN";

/** Store the choice and reload the window in that language. */
export function setLanguagePref(pref: LanguagePref): void {
  save("language", pref);
  location.reload();
}

/**
 * Look up `text` and fill in `{name}` placeholders. English plurals:
 * `{count:file|files}` picks a word by whether `count` is 1.
 */
export function t(text: string, params?: Record<string, string | number | null | undefined>): string {
  const s = language === "en" ? (en[text] ?? text) : text;
  if (!params) return s;
  return s
    .replace(/\{(\w+):([^|{}]*)\|([^{}]*)\}/g, (m, k: string, one: string, many: string) =>
      k in params ? (Number(params[k]) === 1 ? one : many) : m,
    )
    .replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k] ?? "") : m));
}

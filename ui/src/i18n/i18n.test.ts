import { describe, expect, it } from "vitest";
import { en } from "./en";
import { t } from "./index";

/** The interface code as text: path (relative to src) → source. */
const files: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(["../**/*.{ts,tsx}", "!../**/*.test.{ts,tsx}", "!../bindings/**", "!../i18n/**", "!../dev/**"], {
      query: "?raw",
      import: "default",
      eager: true,
    }),
  ).map(([path, text]) => [path.replace(/^\.\.\//, ""), text]),
);

/** Every t("…") key in the interface code. */
function usedKeys(): Set<string> {
  const keys = new Set<string>();
  for (const text of Object.values(files)) {
    for (const m of text.matchAll(/\bt\(("(?:[^"\\]|\\.)*")/g)) keys.add(JSON.parse(m[1]));
  }
  return keys;
}

/** Placeholder names; a plural form `{count:one|many}` counts as `{count}`. */
const placeholders = (s: string) => [...new Set([...s.matchAll(/\{(\w+)(?::[^{}]*)?\}/g)].map((m) => m[1]))].sort();

describe("i18n", () => {
  const used = usedKeys();

  it("translates every interface string, with the same placeholders", () => {
    expect(Object.keys(files).length).toBeGreaterThan(40);
    expect(used.size).toBeGreaterThan(400);
    const missing = [...used].filter((k) => !(k in en));
    expect(missing).toEqual([]);
    const mismatched = [...used].filter((k) => k in en && placeholders(k).join() !== placeholders(en[k]).join());
    expect(mismatched).toEqual([]);
  });

  it("has no leftover translations", () => {
    expect(Object.keys(en).filter((k) => !used.has(k))).toEqual([]);
  });

  it("leaves no Chinese text outside t()", () => {
    const left: string[] = [];
    for (const [f, text] of Object.entries(files)) {
      text
        .split("\n")
        .forEach((line, i) => {
          // Deliberate exceptions say so on the line.
          if (line.includes("i18n-ignore")) return;
          const code = line
            .replace(/\/\/.*$/, "")
            .replace(/\bt\("(?:[^"\\]|\\.)*"/g, "")
            // Search aliases for the command palette stay Chinese on purpose.
            .replace(/(keywords|aliases): .*/, "");
          if (/^\s*(\*|\/\*)/.test(code)) return;
          if (/[一-鿿]/.test(code) && !/\[一-鿿\]/.test(code)) left.push(`${f}:${i + 1}: ${line.trim()}`);
        });
    }
    expect(left).toEqual([]);
  });

  it("fills in placeholders", () => {
    expect(t("已切换到 {branch}", { branch: "main" })).toBe("已切换到 main");
    expect(t("{count} 个文件", { count: 3 })).toBe("3 个文件");
    expect(t("没有找到提交{note}", { note: null })).toBe("没有找到提交");
    // Plural forms are for English strings; Chinese ones never use them.
    expect(t("{n} {n:file|files}", { n: 1 })).toBe("1 file");
    expect(t("{n} {n:file|files}", { n: 2 })).toBe("2 files");
  });
});

// What changed in a version, from CHANGELOG.md (bundled at build time, so it
// works offline and always matches the running version).
import changelog from "../../../CHANGELOG.md?raw";

export type Block = { kind: "heading" | "item" | "text"; text: string };

/** The section for `version` (without its own heading), or null. */
export function changelogSection(version: string, text: string = changelog): string | null {
  const lines = text.split("\n");
  const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
  if (start < 0) return null;
  const end = lines.findIndex((l, i) => i > start && l.startsWith("## "));
  const body = lines.slice(start + 1, end < 0 ? undefined : end).join("\n").trim();
  return body || null;
}

/** Just enough Markdown for a changelog: `###` headings, `-` items, paragraphs. */
export function changelogBlocks(section: string): Block[] {
  const out: Block[] = [];
  for (const raw of section.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith("#")) out.push({ kind: "heading", text: line.replace(/^#+\s*/, "") });
    else if (/^[-*]\s/.test(line)) out.push({ kind: "item", text: line.slice(2).trim() });
    else if (out.length && out[out.length - 1].kind !== "heading" && /^\s{2,}/.test(raw)) {
      // A wrapped continuation of the item or paragraph above.
      out[out.length - 1].text += " " + line;
    } else out.push({ kind: "text", text: line });
  }
  return out;
}

/** Split `code` spans out of a line: odd indexes are code. */
export const codeSpans = (text: string): string[] => text.split("`");

export type Span = { kind: "text" | "code" | "strong"; text: string };

/** Inline Markdown in a line: `code`, **bold**, and [links](url) reduced to their text. */
export function inlineSpans(text: string): Span[] {
  const out: Span[] = [];
  const push = (kind: Span["kind"], t: string) => {
    if (!t) return;
    const last = out[out.length - 1];
    if (last?.kind === kind && kind !== "code") last.text += t;
    else out.push({ kind, text: t });
  };
  codeSpans(text).forEach((part, i) => {
    if (i % 2) return push("code", part);
    const plain = part.replace(/\[([^\]]+)\]\([^)\s]*\)/g, "$1");
    plain.split(/\*\*(.+?)\*\*/).forEach((p, j) => push(j % 2 ? "strong" : "text", p));
  });
  return out;
}

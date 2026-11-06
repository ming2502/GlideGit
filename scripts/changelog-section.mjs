// Print the CHANGELOG.md section for a version (without its heading): the
// release notes on GitHub and in the in-app update notice.
//
//   node scripts/changelog-section.mjs 1.0.0
import { readFileSync } from "node:fs";

const version = process.argv[2];
if (!version) {
  console.error("usage: node scripts/changelog-section.mjs <version>");
  process.exit(2);
}
const text = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8");
const lines = text.split("\n");
const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
if (start < 0) {
  console.error(`CHANGELOG.md has no section for ${version}`);
  process.exit(1);
}
const end = lines.findIndex((l, i) => i > start && l.startsWith("## "));
const body = lines.slice(start + 1, end < 0 ? undefined : end).join("\n").trim();
if (!body) {
  console.error(`the ${version} section of CHANGELOG.md is empty`);
  process.exit(1);
}
process.stdout.write(body + "\n");

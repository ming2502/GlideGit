// Fail when the version being released (argument, e.g. "0.2.0") differs from
// the one the app is built with: tauri.conf.json, the Cargo workspace and
// package.json must all agree.
import { readFileSync } from "node:fs";

const want = process.argv[2];
if (!want) {
  console.error("usage: node scripts/check-version.mjs <version>");
  process.exit(2);
}
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");
const found = {
  "crates/app/tauri.conf.json": JSON.parse(read("crates/app/tauri.conf.json")).version,
  "Cargo.toml [workspace.package]": /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(read("Cargo.toml"))?.[1],
  "package.json": JSON.parse(read("package.json")).version,
};
let ok = true;
for (const [where, v] of Object.entries(found)) {
  const same = v === want;
  ok &&= same;
  console.log(`${same ? "ok  " : "DIFF"} ${where}: ${v}`);
}
if (!ok) {
  console.error(`version ${want} does not match everywhere`);
  process.exit(1);
}

// Build `latest.json`, the manifest the in-app updater reads from the latest
// GitHub release (see crates/app/src/updater.rs).
//
//   node scripts/updater-manifest.mjs <version> <dir> <notes-file> > latest.json
//
// <dir> holds the signed update packages and their `.sig` files:
//   GlideGit_<version>_universal.app.tar.gz(.sig)   macOS, both architectures
//   GlideGit_<version>_x64-setup.exe(.sig)          Windows (NSIS)
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const [version, dir, notesFile] = process.argv.slice(2);
if (!version || !dir) {
  console.error("usage: node scripts/updater-manifest.mjs <version> <dir> [notes-file]");
  process.exit(2);
}
const repo = process.env.GITHUB_REPOSITORY ?? "ming2502/GlideGit";
// UPDATER_BASE_URL: where the packages are served instead (local testing).
const base = process.env.UPDATER_BASE_URL ?? `https://github.com/${repo}/releases/download/v${version}`;

function entry(file) {
  const sig = join(dir, `${file}.sig`);
  if (!existsSync(join(dir, file)) || !existsSync(sig)) throw new Error(`missing ${file} or its .sig in ${dir}`);
  return { signature: readFileSync(sig, "utf8").trim(), url: `${base}/${encodeURIComponent(file)}` };
}

const mac = entry(`GlideGit_${version}_universal.app.tar.gz`);
const notes = notesFile && existsSync(notesFile) ? readFileSync(notesFile, "utf8").trim() : "";
const manifest = {
  version,
  notes: notes || `https://github.com/${repo}/releases/tag/v${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    // One universal app serves both Mac architectures.
    "darwin-aarch64": mac,
    "darwin-x86_64": mac,
    "windows-x86_64": entry(`GlideGit_${version}_x64-setup.exe`),
  },
};
process.stdout.write(JSON.stringify(manifest, null, 2) + "\n");

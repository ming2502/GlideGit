// Download MinGit (a minimal Git for Windows, GPLv2) into target/mingit, for
// the Windows installer to ship as a fallback when no git is installed.
// The archive is pinned and checked against its SHA-256 before use.
//
//   node scripts/fetch-mingit.mjs
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "2.56.0.4";
const URL = `https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.4/MinGit-${VERSION}-64-bit.zip`;
const SHA256 = "e3bf78351fe777fba47ddee5950082f8a98bb0f02604fdc7ba3ee2cb68ffa877";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const cache = join(root, "target", "mingit-cache");
const zip = join(cache, `MinGit-${VERSION}-64-bit.zip`);
const out = join(root, "target", "mingit");
const stamp = join(out, ".glidegit-mingit-version");

if (existsSync(stamp) && readFileSync(stamp, "utf8").trim() === VERSION) {
  console.log(`MinGit ${VERSION} already in ${out}`);
  process.exit(0);
}

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
mkdirSync(cache, { recursive: true });
let data = existsSync(zip) ? readFileSync(zip) : null;
if (!data || sha256(data) !== SHA256) {
  console.log(`downloading ${URL}`);
  const res = await fetch(URL);
  if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText}`);
  data = Buffer.from(await res.arrayBuffer());
  const got = sha256(data);
  if (got !== SHA256) throw new Error(`checksum mismatch: expected ${SHA256}, got ${got}`);
  writeFileSync(zip, data);
}

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
// bsdtar reads zip files: built into Windows 10+ and macOS.
execFileSync("tar", ["-xf", zip, "-C", out], { stdio: "inherit" });
if (!existsSync(join(out, "cmd", "git.exe"))) throw new Error("cmd/git.exe missing from the archive");
writeFileSync(stamp, VERSION + "\n");
console.log(`MinGit ${VERSION} extracted to ${out}`);

#!/usr/bin/env node
// Release a new version: bump the version everywhere, turn the changelog's
// "Unreleased" section into this version, commit, tag and push. The release
// workflow then builds, signs and publishes (docs/RELEASING.md).
//
//   pnpm release 1.1.0            asks before committing and pushing
//   pnpm release                  asks for the version first
//
// Options:
//   --dry-run     show what would change, touch nothing
//   --yes         do not ask for confirmation
//   --no-checks   skip the local type check, tests and build
//   --no-push     commit and tag locally only (push later yourself)
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const dryRun = flag("--dry-run");
const noPush = flag("--no-push");
const skipChecks = flag("--no-checks");
const assumeYes = flag("--yes");

const color = (code) => (s) => (process.stdout.isTTY ? `\x1b[${code}m${s}\x1b[0m` : s);
const bold = color("1");
const red = color("31");
const green = color("32");
const dim = color("2");

function fail(msg) {
  console.error(red(`✗ ${msg}`));
  process.exit(1);
}
const step = (msg) => console.log(`${green("•")} ${msg}`);

/** Run a command in the repo; output shown unless `quiet`. */
function run(cmd, cmdArgs, { quiet = false } = {}) {
  return execFileSync(cmd, cmdArgs, {
    cwd: root,
    encoding: "utf8",
    stdio: quiet ? ["ignore", "pipe", "pipe"] : "inherit",
    // pnpm and cargo are .cmd shims on Windows.
    shell: process.platform === "win32",
  });
}
const git = (...a) => run("git", a, { quiet: true }).trim();

// ---------------------------------------------------------------- version

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/;

/** Negative when a < b. A pre-release sorts before its release. */
function compareVersions(a, b) {
  const pa = SEMVER.exec(a);
  const pb = SEMVER.exec(b);
  for (let i = 1; i <= 3; i++) {
    const d = Number(pa[i]) - Number(pb[i]);
    if (d) return d;
  }
  if (pa[4] === pb[4]) return 0;
  if (!pa[4]) return 1;
  if (!pb[4]) return -1;
  return pa[4] < pb[4] ? -1 : 1;
}

const read = (p) => readFileSync(join(root, p), "utf8");
const current = JSON.parse(read("crates/app/tauri.conf.json")).version;

let version = args.find((a) => !a.startsWith("--"))?.replace(/^v/, "");
if (!version) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  version = (await rl.question(`新版本号（当前 ${current}）：`)).trim().replace(/^v/, "");
  rl.close();
}
if (!SEMVER.test(version ?? "")) fail(`版本号格式不对：${version}（应为 1.2.3 或 1.2.3-beta.1）`);
if (compareVersions(version, current) <= 0) fail(`${version} 不比当前版本 ${current} 新`);
const tag = `v${version}`;
const prerelease = version.includes("-");

console.log(bold(`\n发布 GlideGit ${version}${prerelease ? "（预发布版本，不推送给自动更新）" : ""}\n`));

// ----------------------------------------------------------- repository state

step("检查仓库状态");
if (git("rev-parse", "--abbrev-ref", "HEAD") !== "main") fail("请在 main 分支上发布");
if (git("status", "--porcelain")) fail("有未提交的修改，请先提交或暂存（git stash）");
git("fetch", "--quiet", "origin");
const behind = Number(git("rev-list", "--count", "HEAD..origin/main"));
if (behind > 0) fail(`本地落后远端 ${behind} 个提交，请先 git pull`);
if (git("tag", "-l", tag)) fail(`标签 ${tag} 已经存在`);
if (git("ls-remote", "--tags", "origin", `refs/tags/${tag}`)) fail(`远端已经有标签 ${tag}`);

// ------------------------------------------------------------------ changes

const today = new Date();
const date = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;

/** path → [before, after] */
const edits = new Map();
function edit(path, change) {
  const before = edits.get(path)?.[1] ?? read(path);
  const after = change(before);
  if (after === before) fail(`${path} 没有变化（找不到版本号？）`);
  edits.set(path, [read(path), after]);
}

// CHANGELOG: "Unreleased" becomes this version, a new empty one goes on top.
edit("CHANGELOG.md", (text) => {
  const lines = text.split("\n");
  const start = lines.findIndex((l) => /^## \[Unreleased\]/i.test(l));
  if (start < 0) fail("CHANGELOG.md 里没有 ## [Unreleased] 一节");
  const end = lines.findIndex((l, i) => i > start && l.startsWith("## "));
  const body = lines.slice(start + 1, end < 0 ? undefined : end).join("\n").trim();
  if (!body) fail("CHANGELOG.md 的 Unreleased 一节是空的：先写好这个版本的更新内容");
  if (lines.some((l) => l.startsWith(`## [${version}]`))) fail(`CHANGELOG.md 已经有 ${version} 一节`);
  lines.splice(start, 1, "## [Unreleased]", "", `## [${version}] - ${date}`);
  return lines.join("\n");
});

const jsonVersion = (text) => text.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`);
edit("crates/app/tauri.conf.json", jsonVersion);
edit("package.json", jsonVersion);
edit("Cargo.toml", (text) => text.replace(/(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${version}$2`));
// Path dependencies between the crates name the version too.
for (const dir of readdirSync(join(root, "crates"))) {
  const path = `crates/${dir}/Cargo.toml`;
  if (!existsSync(join(root, path))) continue;
  const text = read(path);
  const next = text.replaceAll(`version = "${current}", path`, `version = "${version}", path`);
  if (next !== text) edits.set(path, [text, next]);
}

console.log(`\n将修改：`);
for (const path of edits.keys()) console.log(`  ${path}`);
console.log(`  Cargo.lock`);
const notes = edits.get("CHANGELOG.md")[1];
const section = notes.split("\n");
const from = section.findIndex((l) => l.startsWith(`## [${version}]`));
const to = section.findIndex((l, i) => i > from && l.startsWith("## "));
console.log(`\n${bold(`更新说明（${version}）：`)}\n${dim(section.slice(from + 1, to < 0 ? undefined : to).join("\n").trim())}\n`);

if (dryRun) {
  console.log(dim("--dry-run：没有修改任何文件。"));
  process.exit(0);
}

// ------------------------------------------------------------------- confirm

if (!assumeYes) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const what = noPush ? "修改版本号、提交并打标签（不推送）" : "修改版本号、提交、打标签并推送（推送后自动构建并发布）";
  const answer = (await rl.question(`${what}？[y/N] `)).trim().toLowerCase();
  rl.close();
  if (answer !== "y" && answer !== "yes") {
    console.log("已取消，没有修改任何文件。");
    process.exit(0);
  }
}

// ---------------------------------------------------------------------- do it

step("修改版本号和更新日志");
for (const [path, [, after]] of edits) writeFileSync(join(root, path), after);
/** Put every file back: used when a later step fails. */
const undo = () => {
  for (const [path, [before]] of edits) writeFileSync(join(root, path), before);
  try {
    git("checkout", "--", "Cargo.lock");
  } catch {
    /* unchanged */
  }
};

try {
  step("更新 Cargo.lock");
  run("cargo", ["update", "--workspace", "--quiet"]);
  run("node", ["scripts/check-version.mjs", version], { quiet: true });
  run("node", ["scripts/changelog-section.mjs", version], { quiet: true });

  if (!skipChecks) {
    step("本地检查：类型检查、前端测试、构建");
    run("pnpm", ["exec", "tsc", "--noEmit", "-p", "ui"]);
    run("pnpm", ["test"]);
    run("cargo", ["build", "--workspace", "--quiet"]);
  }
} catch (e) {
  undo();
  fail(`${e.message.split("\n")[0]}（已还原所有修改）`);
}

step("提交并打标签");
git("add", "-A");
git("commit", "--quiet", "-m", `Release ${tag}`);
git("tag", "-a", tag, "-m", `GlideGit ${version}`);

if (noPush) {
  console.log(green(`\n✓ 已在本地提交并打标签 ${tag}。准备好后推送：`));
  console.log(`  git push origin main\n  git push origin ${tag}`);
  process.exit(0);
}

step("推送");
run("git", ["push", "--quiet", "origin", "main"]);
run("git", ["push", "--quiet", "origin", tag]);

const repo = git("remote", "get-url", "origin").replace(/^git@github\.com:|^https:\/\/github\.com\//, "").replace(/\.git$/, "");
console.log(green(`\n✓ 已推送 ${tag}。发布工作流正在构建，全部成功后自动发布：`));
console.log(`  进度：https://github.com/${repo}/actions`);
console.log(`  发布：https://github.com/${repo}/releases/tag/${tag}`);

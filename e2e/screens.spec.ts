// One screenshot per main screen, light and dark. Run `pnpm e2e`; after an
// intended UI change, update the baselines (see docs/TESTING.md).
import { expect, test, type Page } from "@playwright/test";

/** Every run sees the same moment: relative times ("3 小时前") stay put. */
const NOW = new Date("2026-11-06T09:00:00+08:00");

async function start(page: Page, opts: { theme?: "light" | "dark"; language?: "zh" | "en" } = {}) {
  await page.clock.setFixedTime(NOW);
  await page.addInitScript(
    ({ theme, language }) => {
      localStorage.clear();
      // No update notice popping up in the middle of a screenshot.
      localStorage.setItem("autoCheckUpdates", "false");
      if (theme) localStorage.setItem("theme", JSON.stringify(theme));
      if (language) localStorage.setItem("language", JSON.stringify(language));
    },
    { theme: opts.theme ?? null, language: opts.language ?? null },
  );
  await page.goto("/");
  await expect(page.locator(".welcome")).toBeVisible();
}

/** Open the first demo repository and wait for its fresh snapshot. */
async function openRepo(page: Page) {
  await page.getByRole("button", { name: /打开仓库|Open Repository/ }).first().click();
  await expect(page.locator(".file-row").first()).toBeVisible();
  await expect(page.locator(".sb-stale")).toHaveCount(0);
}

const mod = "ControlOrMeta";

/** Nothing still loading: no spinner, the commit index fully built. */
async function settle(page: Page) {
  await expect(page.locator(".spinner:visible")).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByText(/正在建立提交索引|Indexing commits/)).toHaveCount(0, { timeout: 15_000 });
}

test("welcome", async ({ page }) => {
  await start(page);
  await settle(page);
  await expect(page).toHaveScreenshot("welcome.png");
});

test("changes and diff", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.locator(".file-row", { hasText: "session.rs" }).click();
  await expect(page.locator(".cm-mergeView, .cm-editor").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("changes-diff.png");
});

test("changes and diff, dark", async ({ page }) => {
  await start(page, { theme: "dark" });
  await openRepo(page);
  await page.locator(".file-row", { hasText: "session.rs" }).click();
  await expect(page.locator(".cm-mergeView, .cm-editor").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("changes-diff-dark.png");
});

test("log with commit details", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+9`);
  const rows = page.locator(".log-row:not(.pending)");
  await expect(rows.nth(20)).toBeVisible();
  await rows.nth(2).click();
  await expect(page.locator(".cd-subject")).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("log.png");
});

test("log, dark", async ({ page }) => {
  await start(page, { theme: "dark" });
  await openRepo(page);
  await page.keyboard.press(`${mod}+9`);
  await expect(page.locator(".log-row:not(.pending)").nth(20)).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("log-dark.png");
});

test("branch popup", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+b`);
  await expect(page.locator(".branch-popup")).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("branch-popup.png");
});

test("command palette", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+Shift+a`);
  await page.keyboard.type("push");
  await expect(page.locator(".cp-row").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("palette.png");
});

test("stash and shelf", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+8`);
  await expect(page.locator(".stash-row").first()).toBeVisible();
  await expect(page.locator(".cd-subject")).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("stash.png");
});

test("undo center", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+Shift+u`);
  await expect(page.locator(".uc-item").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("undo-center.png");
});

test("settings, shortcuts", async ({ page }) => {
  await start(page);
  await openRepo(page);
  await page.keyboard.press(`${mod}+Alt+s`);
  await page.locator(".st-nav button", { hasText: "快捷键" }).click();
  await expect(page.locator(".km-row").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("settings-shortcuts.png");
});

test("changes and diff, English", async ({ page }) => {
  await start(page, { language: "en" });
  await openRepo(page);
  await page.locator(".file-row", { hasText: "session.rs" }).click();
  await expect(page.locator(".cm-mergeView, .cm-editor").first()).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("changes-diff-en.png");
});

test("log with commit details, English", async ({ page }) => {
  await start(page, { language: "en" });
  await openRepo(page);
  await page.keyboard.press(`${mod}+9`);
  const rows = page.locator(".log-row:not(.pending)");
  await expect(rows.nth(20)).toBeVisible();
  await rows.nth(2).click();
  await expect(page.locator(".cd-subject")).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("log-en.png");
});

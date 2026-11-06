// Screenshot regression tests: the real UI on the mock backend (ui/src/dev/mock.ts),
// in the engine each platform's app uses (macOS: WebKit like WKWebView;
// Windows: Chromium like WebView2). Fonts render differently per platform,
// so each keeps its own baselines, made on the CI machines (see ci.yml).
import { defineConfig, devices } from "@playwright/test";

const port = 1431;
const project =
  process.platform === "win32"
    ? { name: "chromium", use: { ...devices["Desktop Chrome"] } }
    : { name: "webkit", use: { ...devices["Desktop Safari"] } };

export default defineConfig({
  testDir: "e2e",
  snapshotPathTemplate: "{testDir}/__screenshots__/{platform}-{projectName}/{arg}{ext}",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  expect: {
    toHaveScreenshot: {
      // Repeated runs on one machine give identical images; this only allows
      // a little antialiasing noise. A small shift (4 px) changes far more.
      maxDiffPixels: 100,
      animations: "disabled",
      caret: "hide",
      stylePath: "e2e/screenshot.css",
    },
  },
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    locale: "zh-CN",
    timezoneId: "Asia/Shanghai",
  },
  projects: [project],
  webServer: {
    command: `pnpm exec vite --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});

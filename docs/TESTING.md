# 测试

| 类别 | 命令 | 内容 |
|---|---|---|
| Rust 单元与集成测试 | `cargo test --workspace` | 后端、Log 索引、会话、操作日志等，在临时目录中的真实仓库上运行 |
| 前端单元测试 | `pnpm test` | vitest：暂存计算、三方合并、rebase 计划、changelist、模糊匹配、快捷键等纯逻辑 |
| 截图回归测试 | `pnpm e2e` | Playwright：真实界面加 mock 后端，逐屏截图并与基准比对 |
| 性能基准 | `cargo run --release -p glide-bench` | 见 ROADMAP 的 M4-1 和 DESIGN §1.2 |

CI（`.github/workflows/ci.yml`）在 macOS 和 Windows 上运行以上全部。

## 截图回归测试

`e2e/screens.spec.ts` 打开界面中的主要画面（欢迎页、Changes 与 diff、Log 与提交详情、分支弹窗、命令面板、Stash/Shelf、撤销中心、设置），浅色主题下各截一张，Changes 和 Log 另有深色主题的截图。

- **渲染**：界面跑在 Vite 开发服务器上，后端换成 `ui/src/dev/mock.ts`，数据每次都一样。macOS 用 WebKit（与应用的 WKWebView 同一引擎），Windows 用 Chromium（与 WebView2 同一引擎）。
- **确定性**：时钟固定在同一时刻（相对时间不变），关闭动画和光标，隐藏悬浮提示（`e2e/screenshot.css`，点击后指针停留处可能弹出提示），关闭自动检查更新；每次截图前等待加载完成（没有转圈、提交索引已建完）。同一台机器上重复运行得到完全相同的图片，所以只容许 100 个像素的差异，4 像素的位移就会被发现。
- **基准**：每个平台一套（字体渲染不同），放在 `e2e/__screenshots__/<平台>-<引擎>/`，由 CI 机器生成并提交，不使用开发者本机生成的图片。

### 改了界面之后

1. 本地确认改动符合预期：`pnpm e2e` 会失败，`playwright-report/` 和 `test-results/` 中有期望、实际和差异图。
2. 提交时在提交信息中加上 `[update-screenshots]`，推送到 main 后，CI 的 screenshots 任务会重新生成两个平台的基准并自动提交（作者为 github-actions[bot]）。
3. **检查 bot 提交里的图片差异**：在 GitHub 上打开那两个提交，图片的前后对比可以直接查看（不需要登录）。重新生成基准会让所有截图「通过」，包括意外的变化：界面语言改造时少了几个空格，就是因为同一次提交更新了基准，测试没有报出来。
4. 新平台或新增截图第一次运行时，没有基准的平台会自动生成并提交，不需要额外操作。

CI 比对失败时，任务的附件（`screenshots-<平台>`）中有报告和差异图（需要登录 GitHub 下载）。

本地第一次运行前需要安装浏览器：`pnpm exec playwright install webkit`（Windows 上为 `chromium`）。本地生成的基准只用于自己比对，不要提交（`pnpm e2e:update` 会写到同一目录，提交前用 `git checkout e2e/__screenshots__` 还原）。

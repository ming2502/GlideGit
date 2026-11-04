# GlideGit 工作计划

> 设计详见 [DESIGN.md](DESIGN.md)。每个任务完成后在此勾选，并在 PR 中引用任务编号。

**当前进度（2026-11-03）**：M0 基本完成（CI 待首次运行）；M1 后端和核心 UI 已完成，剩余认证回调、远程操作进度、多标签页和推送预览。下一步按顺序做：M1-17 → M1-9 → M1-10 → M1-18 → M1-11。

## 里程碑总览

| 里程碑 | 周期 | 目标 | 退出标准 |
|---|---|---|---|
| **M0 基建** | 第 1 周 | 工程骨架可构建、可测试 | `cargo test` 和 `pnpm build` 通过，`pnpm tauri dev` 能启动空壳；CI 在 macOS 与 Windows 上通过 |
| **M1 日常提交闭环** | 第 2–6 周 | 打开仓库 → 看变更 → 提交 → 切分支 → 推拉 | 不用命令行也能完成一天的日常工作 |
| **M2 Log 与精细提交** | 第 7–12 周 | 提交图、索引、过滤、行级提交、Stash | 在 100 万+ 提交仓库中满足性能目标 |
| **M3 高级操作** | 第 13–18 周 | 合并工具、交互式 rebase、Blame、撤销中心 | 能完成冲突解决和历史整理 |
| **M4 发布** | 第 19–22 周 | 性能打磨、打包签名、自动更新、平台集成 | 发布 v1.0 公测版 |

---

## M0 基建

- [x] M0-1 设计文档与工作计划（`docs/`）
- [x] M0-2 Cargo workspace：`git-backend`、`core`、`app`
- [x] M0-3 前端：Vite + React + TS，加入 Tauri 2 的 `crates/app`
- [x] M0-4 Rust → TS 类型生成（ts-rs，输出到 `ui/src/bindings`）
- [x] M0-5 代码规范：rustfmt、clippy，以及 TS 的 `tsc --noEmit`
- [x] M0-7 浏览器预览模式：`pnpm dev` 在普通浏览器里用 mock IPC 渲染 UI，方便调样式
- [ ] M0-6 GitHub Actions：macOS 与 Windows 上运行 `cargo test`、`clippy` 和前端构建 — *workflow 已编写，需推送后首次运行确认*

## M1 日常提交闭环

**后端（git-backend / core）**
- [x] M1-1 `GitCli`：定位 git 可执行文件，注入安全环境变量，捕获错误，设置超时与取消，Windows 下不弹出控制台窗口
- [x] M1-2 `status --porcelain=v2 -z` 解析：分支头、ahead/behind、普通 / 重命名 / 未合并 / 未跟踪文件
- [x] M1-3 分支列表（`for-each-ref`）：本地、远程、upstream、最近 checkout
- [x] M1-4 写操作：stage、unstage、discard、commit（含 amend）、checkout、新建分支
- [x] M1-5 文件 diff（工作区 / 暂存区 / 未跟踪文件）
- [x] M1-6 `RepoActor`：写操作串行、刷新后推送快照事件
- [x] M1-7 Watcher：notify + 150ms 去抖，过滤 `.git` 噪声，自身写操作期间抑制
- [x] M1-8 简单 Log（最近 N 个提交，无图）
- [ ] M1-9 Fetch / Pull / Push，带进度事件，支持 `--force-with-lease` — *基础版已完成（首次推送自动设置 upstream，Pull 可选 merge/rebase/ff-only）；还缺进度事件与取消*
- [ ] M1-10 认证：`GIT_ASKPASS` 回调到 UI，复用 SSH agent
- [ ] M1-11 最近仓库持久化与多仓库标签页 — *最近仓库已完成；多标签页未做*

**前端**
- [x] M1-12 应用外壳：左侧 Commit 面板，右侧 diff，底部状态栏
- [x] M1-13 Commit 面板：变更列表，staged / unstaged 分组，勾选即 stage，提交信息框与 Amend
- [x] M1-14 Diff 视图：统一视图，带行号和颜色（M2 换成 CodeMirror 并排视图）
- [x] M1-15 分支弹窗：搜索、本地 / 远程分组、checkout、新建分支
- [x] M1-16 Action 系统与快捷键（⌘K、⌘⇧`、⌘R……）
- [ ] M1-17 后台任务进度栏与错误通知 — *状态栏任务指示与错误 toast 已完成；还缺进度百分比与取消按钮*
- [ ] M1-18 Pull / Push 对话框（推送前预览提交）

## M2 Log 与精细提交

- [ ] M2-1 `log-index` crate：流式首屏加载 + 后台全量索引落盘（SQLite）
- [ ] M2-2 图布局（lane 分配）及增量更新
- [ ] M2-3 Canvas 提交图 + 虚拟列表
- [ ] M2-4 Log 过滤：分支、作者、日期、路径、文本、hash
- [ ] M2-5 提交详情：变更文件树 + diff
- [ ] M2-6 Log 右键菜单：checkout、新建分支或标签、cherry-pick、revert、reset
- [ ] M2-7 CodeMirror 6 diff：并排视图、字符级差异、折叠
- [ ] M2-8 块级 / 行级 stage 与 discard（`git apply --cached`）
- [ ] M2-9 Stash 列表、应用、弹出、查看 diff
- [ ] M2-10 用 gix 读取热点数据（status、revwalk）；在大仓库上跑基准对比
- [ ] M2-11 快照缓存（stale-while-revalidate 启动）

## M3 高级操作

- [ ] M3-1 三方合并工具（含"魔棒"自动合并）
- [ ] M3-2 冲突引导横幅：Continue / Abort / Skip
- [ ] M3-3 图形化交互式 rebase
- [ ] M3-4 Annotate / Blame gutter
- [ ] M3-5 文件历史与选区历史
- [ ] M3-6 分支比较
- [ ] M3-7 OpLog 与撤销中心
- [ ] M3-8 Shelve 与多 Changelist
- [ ] M3-9 命令面板（Search Actions）

## M4 发布

- [ ] M4-1 性能基准接入 CI，并对照设计文档 §1.2 的目标逐项达标
- [ ] M4-2 macOS 签名与公证、Universal 二进制
- [ ] M4-3 Windows NSIS/MSI 安装包与代码签名，附带 MinGit 作为后备
- [ ] M4-4 Tauri updater 自动更新
- [ ] M4-5 设置页：git 路径、键位预设、主题、语言
- [ ] M4-6 双平台截图回归测试
- [ ] M4-7 v1.0 公测发布

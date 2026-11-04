# GlideGit 工作计划

> 设计详见 [DESIGN.md](DESIGN.md)。每个任务完成后在此勾选，并在 PR 中引用任务编号。

**当前进度（2026-11-04）**：M0、M1 已完成。M2 进行中：M2-1 至 M2-6（Log 索引、图布局、Canvas 提交图、Log 过滤、提交详情、Log 右键菜单）已完成，下一步是 M2-7（CodeMirror diff）。

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
- [x] M0-6 GitHub Actions：macOS 与 Windows 上运行 `cargo test`、`clippy` 和前端构建

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
- [x] M1-9 Fetch / Pull / Push，带进度事件，支持 `--force-with-lease`；fetch/push 不阻塞本地写操作，取消时连同 ssh 等子进程一起终止（Unix）
- [x] M1-10 认证：GlideGit 自身作为 `GIT_ASKPASS` / `SSH_ASKPASS`，通过本机回环端口和随机令牌，把提问转给 UI 弹窗。支持用户名、密码/令牌、SSH 密钥口令、主机指纹确认；仍会先用已配置的凭据助手，SSH agent 照常生效。*SSH 场景和 Windows 发布版尚未实机验证*
- [x] M1-11 最近仓库持久化与多仓库标签页：后台标签实时更新；切换时恢复各自选中的文件；下次启动恢复上次打开的标签；快捷键 ⌘⇧] / ⌘⇧[ 或 ⌃Tab 切换，⌘⌥W 关闭；中键也可关闭

**前端**
- [x] M1-12 应用外壳：左侧 Commit 面板，右侧 diff，底部状态栏
- [x] M1-13 Commit 面板：变更列表，staged / unstaged 分组，勾选即 stage，提交信息框与 Amend
- [x] M1-14 Diff 视图：统一视图，带行号和颜色（M2 换成 CodeMirror 并排视图）
- [x] M1-15 分支弹窗：搜索、本地 / 远程分组、checkout、新建分支
- [x] M1-16 Action 系统与快捷键（⌘K、⌘⇧`、⌘R……）
- [x] M1-17 后台任务进度栏与错误通知：状态栏显示阶段、百分比和进度条，可取消，多任务时可展开列表
- [x] M1-18 推送对话框：推送前列出将要推送的提交和目标分支，提示远端领先，支持 Force Push（with-lease）。Pull 暂用工具栏下拉选择模式，没有单独对话框

## M2 Log 与精细提交

- [x] M2-1 `log-index` crate：流式首屏加载 + 后台全量索引落盘（SQLite）
  - 30 万提交的合成仓库（M1 Mac，release）：首次建索引 3.7 秒（`git log` 本身 2.8 秒）。有 commit-graph 时首屏 0.3 秒，没有时 2.3 秒。再次打开 0.14 秒，增量更新约 0.4 秒，读一页 100 行不到 1 毫秒，索引文件 40 MiB。
  - 基准：`cargo run --release -p glide-log-index --example bench -- <仓库> [缓存目录]`
  - 遗留：仓库没有 commit-graph 时，第一次打开的首屏要等 git 走完整个历史，留给 M2-10 处理。超过约 129 万行时，DOM 滚动高度会超过浏览器上限，已在 M2-3 用缩放滚动条解决。
- [x] M2-2 图布局（lane 分配）及增量更新
  - 每 256 行保存一次 lane 状态（检查点），读一页时从最近的检查点重算，30 万提交仓库上一页 100 行约 0.5 毫秒。
  - 有新提交时，从顶部重算到 lane 状态与旧检查点一致为止，通常只算几百行。颜色按列分配，否则主干颜色会随每个新提交整条改变，检查点也就永远对不上。
  - 再次打开时要扫描全表重建检查点，30 万提交约 0.28 秒（M2-1 时为 0.14 秒）。检查点可以随 M2-11 的快照缓存一起落盘。
- [x] M2-3 Canvas 提交图 + 虚拟列表
  - 一块 Canvas 叠在列表可视区上，滚动时只重绘屏幕上的行；跨列的线画成贝塞尔曲线，合并提交画空心节点，颜色按列取自主题调色板（亮色、暗色各 8 色）。
  - 虚拟列表的总高度超过 800 万像素（约 30 万行）时，把滚动条按比例缩放，滚轮仍按正常速度滚动。浏览器预览中用 `?commits=2000000` 验证过 200 万行：End、PageUp 和滚轮都能准确定位。
  - 关闭了列表容器的滚动锚定（`overflow-anchor: none`），否则 Chrome 会在占位块高度变化时自行改动滚动位置。
  - 图列最多画 24 列，更宽的部分被裁掉；IDEA 式「折叠长边」以后再做。
- [x] M2-4 Log 过滤：分支、作者、日期、路径、文本、hash
  - 过滤栏：搜索框（提交信息或 hash，⌘F 聚焦）和「分支」「作者」「日期」「路径」四个下拉；条件之间是「且」的关系。
  - 文本、hash、作者、日期用 SQL 查索引；分支和路径交给 git（`rev-list`、`log -- <路径>`），得到的提交集合再与 SQL 结果取交集。
  - 只按分支过滤时，结果对父提交封闭，图照常绘制；其他条件会让历史出现缺口，只显示列表。
  - 换了过滤条件，会取消上一个条件还在运行的 git 命令。选中的提交在新结果中仍存在时保持选中。
  - 30 万提交：文本、hash、作者过滤 60–90 毫秒，路径约 0.4 秒，分支约 1.1 秒（主要是 `git rev-list`）。分支过滤可以改为在索引里沿父提交遍历来提速，留给 M2-10。
- [x] M2-5 提交详情：变更文件树 + diff
  - Log 下方是详情面板，与列表之间有可拖动的分隔条，高度会记住。左侧显示标题、正文、作者，提交者与作者不同时也显示提交者；hash 点一下即复制，父提交点一下在 Log 中定位。
  - 变更文件按目录分组，显示类型、重命名前的路径和增删行数，↑↓ 切换文件；右侧显示所选文件的 diff。
  - 后端用 `git log -1` 取提交信息，`git diff-tree -r -z -M --raw --numstat` 取文件和行数，各一次进程调用。合并提交与第一个父提交比较，根提交与空树比较。
  - 在 Log 中快速翻动时，只为停下来的那个提交读取详情（60 毫秒去抖，旧请求结果作废）。
- [x] M2-6 Log 右键菜单：checkout、新建分支或标签、cherry-pick、revert、reset
  - 右键菜单（也可按菜单键或 Shift+F10）：checkout 该提交上的分支、checkout 游离 HEAD（先确认）、新建分支、新建标签（填写说明即为附注标签）、cherry-pick、revert、将当前分支重置到此处、复制 hash。
  - 重置对话框可选 Soft / Mixed / Keep / Hard 四种模式，选 Hard 还要再确认一次。合并提交做 cherry-pick 或 revert 时，以第一个父提交为基准（`-m 1`）。
  - 新增全局横幅：合并、变基、cherry-pick 或 revert 中途停下时显示，带「中止」和「继续」。仍有冲突时「继续」不可用；继续时不打开编辑器，沿用 git 准备好的提交信息。
  - 新分支名、标签名和提交 id 在传给 git 前检查，不能被当作命令行选项。
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

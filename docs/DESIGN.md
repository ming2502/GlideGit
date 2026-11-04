# GlideGit 设计文档

> 一款支持 macOS 与 Windows 的 Git 图形客户端，交互体验对标 IntelliJ IDEA 的 Git 集成：快、就地操作、键盘优先、不怕误操作。

- 状态：Draft v0.1
- 更新：2026-11-03

---

## 1. 产品目标

### 1.1 "丝滑"的可落地定义

| 体验 | IDEA 的做法 | GlideGit 的落地 |
|---|---|---|
| 从不卡 UI | 所有 Git 操作在后台任务中执行，可取消，状态栏显示进度 | UI 线程只做渲染；所有 Git 操作异步、排队、可取消 |
| 打开即可用 | Log 有持久化索引，增量更新 | 启动时先展示上次快照（stale-while-revalidate），后台刷新 |
| 不用手动刷新 | 文件系统监听 | Watcher 去抖合并 + 增量刷新 |
| 键盘流 | Cmd+K 提交、Cmd+Shift+K 推送、VCS 快捷弹窗、Search Everywhere | 统一 Action 系统：菜单 / 右键 / 命令面板 / 快捷键同源 |
| 就地操作 | Log 右键即可 cherry-pick/reset/rebase，状态栏分支弹窗 | 上下文菜单 + 弹出层优先，尽量不用模态对话框 |
| 不怕误操作 | Undo Commit、Local History、危险操作确认 | 操作日志 + 一键撤销（基于 reflog / ORIG_HEAD） |

### 1.2 性能目标（CI 基准）

| 指标 | 目标 |
|---|---|
| 冷启动到可交互 | < 800ms（有快照时 < 300ms） |
| Log 首屏（100 万+ 提交仓库） | < 500ms |
| Log 滚动 | 稳定 60fps（高刷屏 120fps） |
| 分支弹窗打开 | < 50ms |
| 选中提交 → 显示 diff | < 100ms |
| 保存文件 → Commit 面板更新 | < 300ms |

---

## 2. 功能设计

### P0 — MVP（可替代命令行完成日常工作）

1. **仓库管理**：打开 / 克隆 / 最近仓库，多仓库标签页。
2. **Commit 面板**
   - 变更树（按目录分组 / 平铺），staged 与 unstaged 两种视图，以及 IDEA 式"无暂存区"模式
   - 文件级与**块级 / 行级**勾选提交（partial commit）
   - 提交信息编辑器：历史消息、Amend、提交前检查（hooks、行尾）
   - Rollback / Discard（文件级、块级）
3. **Diff 查看器**：并排 / 统一视图、语法高亮、字符级差异、折叠未改动区域、工作区一侧可直接编辑。
4. **Log 视图**
   - 提交图（Canvas 绘制，几十万提交流畅滚动）
   - 过滤：分支、作者、日期、路径、文本 / 正则、hash
   - 选中提交 → 变更文件树 + diff
   - 右键：checkout、新建分支 / 标签、cherry-pick、revert、reset（soft/mixed/hard）、从此处交互式 rebase、与本地比较
5. **分支弹窗**（状态栏 / 快捷键）：搜索、最近分支置顶、本地 / 远程分组；单分支动作：checkout、从此新建、与当前比较、合并到当前、将当前变基到此、push、重命名、删除。
6. **远程**：Fetch / Pull（merge 或 rebase）/ Push（推送前预览提交）/ `--force-with-lease`。
7. **Stash**：列表、应用、弹出、查看 diff。

### P1 — 高级体验

- **三方合并工具**：ours / result / theirs 三栏，"魔棒"自动合并非冲突块，逐块接受。
- **图形化交互式 Rebase**：拖拽排序，pick / reword / squash / fixup / drop，执行前预览。
- **Annotate / Blame**：gutter 显示作者与时间、按时间色阶、点击跳转提交。
- **选区历史**：Show History for Selection。
- **分支比较**：A 有 B 没有的提交 + 文件差异。
- **Shelve 与多 Changelist**。
- **冲突引导**：合并 / 变基进行中时显示横幅：Continue / Abort / Skip。
- **撤销中心**：最近 commit / reset / rebase / checkout 一键撤销。

### P2 — 差异化

Worktree、Submodule、Git LFS、GitHub/GitLab 集成（PR、Review、CI 状态）、AI 提交信息 / 冲突解释、插件系统。

---

## 3. 技术选型

**Tauri 2 + Rust 核心 + React/TypeScript 前端**

| 方案 | 结论 |
|---|---|
| **Tauri 2 + Rust**（采用） | 包小、内存低；Rust 承担 Git / 索引 / diff 计算；前端生态成熟 |
| Electron + Node | 开发快但内存重，参见 GitKraken 的体感问题 |
| Qt / Avalonia / GPU 原生 UI | 上限高，但 diff 编辑器等组件需大量自研 |

注意：macOS 使用 WKWebView、Windows 使用 WebView2，需要双平台 CI 截图回归。

### 3.1 Git 后端：读写分离

- **写操作走系统 git CLI**（commit / merge / rebase / push / pull / checkout 等）：hooks、credential helper、GPG/SSH 签名、LFS、用户 `.gitconfig` 100% 兼容。IDEA 采用同样策略。
- **读操作同样走 CLI，只用机器可读格式**（`--porcelain=v2 -z`、`--format` 加 NUL 分隔），重点是减少进程数。M2-10 在大仓库上对比了 gitoxide (gix)：status、引用读取、遍历提交的速度都与 git 持平，只省下启动进程的开销，却会带来编译时间、体积和行为差异，所以没有引入。改为用 refs 指纹（只做文件 `stat`）判断引用是否变化，没变就复用分支列表、跳过 Log 索引更新，编辑文件触发的刷新只需 1 个 `git status` 进程。gix 留作以后的选项，例如 blame 或 Windows 上进程开销特别大的场景。
- Windows 优先使用系统 git，缺失时可使用随包 MinGit。

---

## 4. 架构

```
┌──────────────────────── Frontend (WebView, React + TS) ────────────────────────┐
│ Views: CommitPanel | LogView(Canvas) | DiffViewer | MergeTool | BranchPopup     │
│ State: Zustand stores ← RepoSnapshot（规范化）                                   │
│ Infra: 虚拟列表 | Action/快捷键系统 | 命令面板 | 任务进度栏 | 通知/撤销中心        │
│ Workers: 语法高亮 | 大 diff 分段                                                 │
└──────────────▲───────────────────────────────────────────┬─────────────────────┘
               │ events（增量推送）                         │ commands（类型化 RPC）
┌──────────────┴──────── IPC：Tauri commands + events/Channels ────────────────────┐
│ TS 类型由 Rust 生成（ts-rs）；大数据分块传输                                        │
└──────────────▲───────────────────────────────────────────┬─────────────────────┘
┌──────────────┴──────────────────── Rust Core ────────────▼─────────────────────┐
│ RepositoryManager ── 每仓库一个 RepoActor（权威状态 + 串行写队列）                 │
│  ├─ TaskScheduler  读并发 / 写串行 / 可取消 / 优先级                                │
│  ├─ GitCli         进程执行、环境注入、进度解析、askpass 回调                        │
│  ├─ RefsFingerprint 引用变化检测（只做 stat，不启动进程）                           │
│  ├─ LogIndex       持久化提交索引 + 图布局（lane）增量计算                           │
│  ├─ DiffEngine     imara-diff + 字符级 diff + 三方合并                              │
│  ├─ Watcher        notify，过滤 + 去抖，区分 worktree / index / refs                 │
│  ├─ OpLog          操作记录与撤销策略                                               │
│  └─ Auth           SSH agent、Keychain / Credential Manager                        │
│ Storage: SQLite（索引、设置、最近仓库、提交消息历史）+ 快照缓存                       │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Crate / 目录划分

```
GlideGit/
├─ crates/
│  ├─ git-backend/   # GitCli、porcelain 解析、GitBackend trait、领域模型
│  ├─ core/          # RepositoryManager、RepoActor、Watcher、OpLog
│  ├─ log-index/     # 提交索引、图布局（M2）
│  ├─ diff/          # diff / merge 算法（M2/M3）
│  └─ app/           # Tauri 入口、IPC 命令、事件桥
├─ ui/               # Vite + React + TS
│  ├─ src/bindings/  # Rust 生成的 TS 类型（勿手改）
│  ├─ src/actions/   # Action 注册表与键位
│  ├─ src/views/
│  ├─ src/stores/
│  └─ src/components/
└─ docs/
```

### 4.2 RepoActor 并发模型

- 每个仓库一个 actor，持有权威 `RepoState`（HEAD、refs、status、进行中的操作）。
- **写操作串行**（避免 `index.lock` 争用）；读操作可并发。
- 自身执行写命令期间抑制 watcher，命令结束后主动刷新一次。
- 状态变化以事件推送前端；M1 推整份快照，M2 起改为增量 patch。

### 4.3 任务系统

```rust
struct Task { id, repo, kind, priority, cancel: CancellationToken, progress: Sender<Progress> }
// 优先级：用户交互（diff、选中切换） > 写操作 > 刷新 > 后台索引
```

选中切换时取消上一个未完成的 diff 请求（switchMap 语义），快速翻看提交不会堆积。

### 4.4 LogIndex（M2）

- 范围：从本地分支、远程分支、标签和 HEAD 出发能到达的全部提交；不含 stash 和 notes。顺序与 `git log --date-order` 相同：子提交在父提交之前，其余按时间从新到旧。
- 存储：每个仓库一个 SQLite 文件，放在应用缓存目录的 `log-index/` 下，文件名取仓库路径的哈希。`commits` 表的整数主键就是显示位置；作者单独成表去重；提交 id 和父提交以二进制保存。
- 首次打开：流式读取 `git log`，一个后台线程读 git 输出，写入线程分批落盘。第一批约 300 行写入后就通知前端，其余每 2 万行提交一次。全量构建期间不维护 oid 索引，结束时一次建好。
- 增量更新：每次刷新快照后，用一次 `for-each-ref` 加读取 `HEAD` 文件比较起点集合。
  - 起点集合没变、只有标签变了：只更新标签，不碰数据库。
  - 起点集合变了：`rev-list 旧起点 ^新起点` 得到不再可达的提交并删除，`log --date-order 新起点 ^旧起点` 得到新提交，放到最前面（主键递减）。新提交不可能是旧提交的祖先，所以顺序仍然合法，旧行的相对位置不变。
  - 旧起点已被 gc 清掉、上次构建没完成、表结构版本不符时：全量重建。
- 读取：内存里保存一份按显示顺序排列的主键数组（100 万行约 8 MB），行号先换算成主键区间，再做一次范围查询。前端按每页 200 行请求，只取可视区附近的页面。数据库变动和内存数组的更新在同一把锁里提交，读者看到的两者总是一致。
- 图布局（`log-index/src/graph.rs`）：自上而下逐行处理，维护「lane 状态」，即两行之间仍在等待某个提交的 lane 列表。
  - 等待本行提交的 lane 汇入节点，最左的一条就是节点所在列；没有 lane 等待的提交（分支顶端）在最右边新开一条。
  - 第一个父提交沿用节点所在的 lane；其余父提交若已有 lane 在等，就连过去，否则紧挨节点右侧新开一条。结束的 lane 移除，其余向左靠拢。
  - 每行分上下两半绘制。上半行 lane 不移动，所以线不会穿过节点；横向移动都在下半行完成。
  - lane 状态只取决于提交本身，因此可以从任意检查点继续。索引每 256 行保存一次状态，读一页时从最近的检查点重算。新提交加入后从顶部重算，直到状态与某个位于所有改动之下的旧检查点相同，之后的检查点直接沿用。
  - 颜色由前端按列分配，不进入布局数据。若颜色跟随 lane 起点的提交，每次在主干上提交都会改变整条主干的颜色，重算也永远无法与旧检查点对上。
- 过滤：文本（提交信息或 hash 前缀）、作者、日期由 SQL 在索引中查询；分支、路径交给 git，返回的提交集合再与 SQL 结果取交集。结果是一份按显示顺序排列的主键列表（视图），按「过滤条件 + 索引版本」缓存最近一次。只有分支条件时，视图对父提交封闭，用同一套布局算法和检查点画图；其他条件只显示列表。新的过滤条件会取消旧条件尚未完成的 git 命令。
- 版本号：行发生移动或标签变化时递增。前端在新页面到达前继续显示旧页面，然后按提交 id 找回选中行。
- 计划中：
  - 利用 `git commit-graph write --reachable --changed-paths` 加速首屏和路径过滤（M2-4 / M2-10）。

### 4.5 Status 与 Watcher

- 监听工作区与 `.git/{HEAD,index,refs,packed-refs,MERGE_HEAD,rebase-merge,rebase-apply}`。
- 150ms 去抖合并；忽略 `.git/objects`、`*.lock` 等噪声。
- 大仓库建议开启 `core.fsmonitor` 与 `core.untrackedCache`。

### 4.6 前端渲染

- 所有列表虚拟化（变更树、Log、分支、diff 行）。
- 提交图：Canvas 2D、固定行高、只重绘可视区。画布以 sticky 方式叠在列表可视区上，不接收鼠标事件；颜色按列分配，取自 CSS 变量 `--graph-0` 至 `--graph-7`，主题切换时重绘。
- 超长列表：虚拟列表以「虚拟像素」（行号 × 行高）记录滚动位置。总高度超过 800 万像素时，滚动条按比例缩放，滚轮事件改为按虚拟像素滚动，以免浏览器的元素高度上限（约 3350 万像素）截断列表。
- Diff：CodeMirror 6（`@codemirror/merge`），并排 / 统一两种布局、字符级差异、折叠未改动区域、按文件名懒加载语法高亮。后端提供文件两侧的全文（单侧上限 8 MB），由前端计算差异。行号旁有勾选框，可以按块或按行暂存。前端由「HEAD + 选中的块和行」算出暂存区应有的完整内容，后端用 `hash-object` + `update-index --cacheinfo` 写入；不生成补丁，也不用 `git apply`。
- 乐观更新：如 stage 立即生效，失败回滚并提示。

### 4.7 Action / 快捷键系统

所有功能注册为 `Action { id, title, keybinding, when(context), run }`；菜单、右键、命令面板、快捷键全部由注册表生成。提供 "IntelliJ" 键位预设（macOS: Cmd，Windows: Ctrl）。

| Action | macOS | Windows |
|---|---|---|
| 提交 | ⌘K | Ctrl+K |
| 推送 | ⌘⇧K | Ctrl+Shift+K |
| 拉取 / 更新 | ⌘T | Ctrl+T |
| 分支弹窗 | ⌘⇧` | Ctrl+Shift+` |
| 命令面板 | ⌘⇧A | Ctrl+Shift+A |
| 刷新 | ⌘R | Ctrl+R |

---

## 5. 核心数据模型

```ts
interface RepoSnapshot {
  path: string;
  head: { branch?: string; detached?: string; upstream?: { name: string; ahead: number; behind: number } };
  operation?: 'merge' | 'rebase' | 'cherry-pick' | 'revert';
  branches: { local: Branch[]; remote: Branch[] };
  changes: FileChange[]; // staged / unstaged / untracked / conflicted
}

interface LogRow {
  hash: string; subject: string; author: string; time: number; refs: string[];
  graph: { lane: number; color: number; edges: Edge[] }; // Rust 预计算（M2）
}
```

---

## 6. 跨平台注意事项

- **Windows**：长路径（`core.longpaths`）、CRLF / `autocrlf`、大小写不敏感、杀毒软件拖慢进程启动（减少 git 调用次数）、OpenSSH vs Pageant、Git Credential Manager；创建子进程使用 `CREATE_NO_WINDOW` 避免闪黑框。
- **macOS**：公证、Keychain、Universal 二进制（arm64 + x86_64）、文件名 Unicode NFD。
- **认证**：通过 `GIT_ASKPASS` / `SSH_ASKPASS` 回调到应用弹窗，不自己存凭据。实现方式：把 GlideGit 自身的可执行文件设为 askpass 程序。git 或 ssh 提问时会以「提示语」为参数启动它；进程检测到 `GLIDEGIT_ASKPASS_ADDR` 后进入客户端模式，连接应用内监听在 127.0.0.1 随机端口上的服务，出示每次启动随机生成的令牌，等待用户在弹窗中回答，然后把答案打印给 git。设置了 `SSH_ASKPASS_REQUIRE=force`，避免 ssh 在终端上提问而卡住。git 仍会先询问已配置的凭据助手（macOS 钥匙串、Git Credential Manager），验证成功后由助手保存密码。
- **发布**：Tauri updater；Windows MSI/NSIS + 代码签名。
- **子进程环境**：强制 `LC_ALL=C`（稳定解析）、`GIT_TERMINAL_PROMPT=0`（禁止阻塞在终端提示）、`GIT_OPTIONAL_LOCKS=0`（只读命令不抢锁）。

---

## 7. 测试策略

- `git-backend`：用临时目录脚本化构造仓库（冲突、重命名、子模块、空仓库、detached HEAD），覆盖 porcelain 解析与各命令。
- `core`：actor 串行性、watcher 去抖、取消语义。
- UI：组件测试 + Playwright（WebView 层）冒烟；双平台截图回归（M4）。
- 基准：大仓库 fixtures（linux / chromium 镜像）跑第 1.2 节指标。

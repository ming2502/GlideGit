# GlideGit

跨平台（macOS / Windows）Git 图形客户端，交互体验对标 IntelliJ IDEA 的 Git 集成：快、就地操作、键盘优先。

- 设计文档：[docs/DESIGN.md](docs/DESIGN.md)
- 工作计划：[docs/ROADMAP.md](docs/ROADMAP.md)
- 发布与签名：[docs/RELEASING.md](docs/RELEASING.md)
- 测试：[docs/TESTING.md](docs/TESTING.md)

## 技术栈

Tauri 2 · Rust（`crates/`）· React + TypeScript（`ui/`）· 系统 git CLI

```
crates/git-backend  git CLI 封装、porcelain 解析、领域模型（导出 TS 类型）
crates/log-index    Log 提交索引：流式首屏、SQLite 落盘、增量更新
crates/core         RepoSession：串行写、文件监听、快照发布、索引更新、操作日志与撤销
crates/bench        性能基准：生成大仓库，对照设计文档的性能目标逐项检查
crates/app          Tauri 入口与 IPC 命令
ui/                 前端（Vite + React + Zustand）
```

## 开发

前置：Rust stable、Node 24+、pnpm、git ≥ 2.30。

```bash
pnpm install
pnpm tauri dev        # 启动桌面应用
pnpm dev              # 仅前端，在浏览器中用 mock 数据预览 UI
cargo test --workspace
pnpm test             # 前端单元测试（vitest）
pnpm e2e              # 截图回归测试（Playwright，见 docs/TESTING.md）
pnpm bindings         # Rust 模型变更后重新生成 ui/src/bindings
cargo run --release -p glide-log-index --example bench -- <仓库路径>   # Log 索引基准
cargo run --release -p glide-git-backend --example refresh -- <仓库路径>   # 刷新耗时基准
cargo run --release -p glide-bench -- [--commits N] [--only log,open,branches,diff,save]   # 性能目标逐项检查（DESIGN §1.2）
```

## 快捷键（IntelliJ 预设）

以下为默认键位，可在「设置 → 快捷键」中修改。

| 动作 | macOS | Windows |
|---|---|---|
| 聚焦提交信息 | ⌘K | Ctrl+K |
| 提交 | ⌘↩ | Ctrl+Enter |
| 提交并推送 | ⌘⌥K | Ctrl+Alt+K |
| 推送 | ⌘⇧K | Ctrl+Shift+K |
| 拉取（Update Project） | ⌘T | Ctrl+T |
| 分支弹窗 | ⌘⇧\` / ⌘B | Ctrl+Shift+\` / Ctrl+B |
| Commit / Log / Stash·Shelf 视图 | ⌘0 / ⌘9 / ⌘8 | Ctrl+0 / Ctrl+9 / Ctrl+8 |
| Stash 修改 | ⌘⇧S | Ctrl+Shift+S |
| Log 中搜索 | ⌘F | Ctrl+F |
| 下一处 / 上一处修改（diff） | F7 / ⇧F7 | F7 / Shift+F7 |
| 刷新 | ⌘R / F5 | Ctrl+R / F5 |
| Rollback 选中文件 | ⌘⌥Z | Ctrl+Alt+Z |
| 命令面板（Find Action） | ⌘⇧A 或双击 Shift | Ctrl+Shift+A 或双击 Shift |
| 设置 | ⌘, / ⌘⌥S | Ctrl+, / Ctrl+Alt+S |
| 撤销中心（撤销最近的操作） | ⌘⇧U | Ctrl+Shift+U |
| 移动到另一个 Changelist | F6 | F6 |
| 下一个 / 上一个仓库标签 | ⌘⇧] / ⌘⇧[ 或 ⌃Tab / ⌃⇧Tab | Ctrl+Shift+] / Ctrl+Shift+[ 或 Ctrl+Tab |
| 关闭仓库标签 | ⌘⌥W | Ctrl+Alt+W |
| 变更列表：切换 / 勾选 | ↑↓ / 空格 | ↑↓ / Space |

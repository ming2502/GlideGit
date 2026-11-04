# GlideGit

跨平台（macOS / Windows）Git 图形客户端，交互体验对标 IntelliJ IDEA 的 Git 集成：快、就地操作、键盘优先。

- 设计文档：[docs/DESIGN.md](docs/DESIGN.md)
- 工作计划：[docs/ROADMAP.md](docs/ROADMAP.md)

## 技术栈

Tauri 2 · Rust（`crates/`）· React + TypeScript（`ui/`）· 系统 git CLI

```
crates/git-backend  git CLI 封装、porcelain 解析、领域模型（导出 TS 类型）
crates/core         RepoSession：串行写、文件监听、快照发布
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
pnpm bindings         # Rust 模型变更后重新生成 ui/src/bindings
```

## 快捷键（IntelliJ 预设）

| 动作 | macOS | Windows |
|---|---|---|
| 聚焦提交信息 | ⌘K | Ctrl+K |
| 提交 | ⌘↩ | Ctrl+Enter |
| 提交并推送 | ⌘⌥K | Ctrl+Alt+K |
| 推送 | ⌘⇧K | Ctrl+Shift+K |
| 拉取（Update Project） | ⌘T | Ctrl+T |
| 分支弹窗 | ⌘⇧\` / ⌘B | Ctrl+Shift+\` / Ctrl+B |
| Commit / Log 视图 | ⌘0 / ⌘9 | Ctrl+0 / Ctrl+9 |
| 刷新 | ⌘R / F5 | Ctrl+R / F5 |
| Rollback 选中文件 | ⌘⌥Z | Ctrl+Alt+Z |
| 下一个 / 上一个仓库标签 | ⌘⇧] / ⌘⇧[ 或 ⌃Tab / ⌃⇧Tab | Ctrl+Shift+] / Ctrl+Shift+[ 或 Ctrl+Tab |
| 关闭仓库标签 | ⌘⌥W | Ctrl+Alt+W |
| 变更列表：切换 / 勾选 | ↑↓ / 空格 | ↑↓ / Space |

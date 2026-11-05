# 发布 GlideGit

发布由 `.github/workflows/release.yml` 完成：推送 `vX.Y.Z` 标签后，构建安装包并上传到一个**草稿** release，确认无误后再手动发布。也可以在 Actions 页面手动运行这个工作流，此时只构建，安装包作为附件下载。

目前包含 macOS（M4-2）；Windows 安装包和签名见 M4-3，自动更新见 M4-4。

## 发布步骤

1. 统一版本号。以下三处必须一致，工作流会检查（`node scripts/check-version.mjs X.Y.Z`）：
   - `crates/app/tauri.conf.json` 的 `version`
   - `Cargo.toml` 的 `[workspace.package] version`
   - `package.json` 的 `version`
2. 提交并推送，确认 CI 通过。
3. 打标签并推送：

   ```bash
   git tag v0.2.0
   ```

   ```bash
   git push origin v0.2.0
   ```

4. 等 Release 工作流完成，在 GitHub 的 Releases 页面打开草稿，补充发布说明，然后发布。

## macOS 安装包

- 一个通用二进制（Universal：Apple 芯片 + Intel），产物为 `GlideGit_X.Y.Z_universal.dmg`。
- 最低系统版本 macOS 11。
- 启用 Hardened Runtime（公证的前提）。权限声明在 `crates/app/entitlements.plist`，目前为空：GlideGit 只运行系统的 git 和它自己（作为 askpass 助手），通过本地 socket 通信，这些都不需要额外权限；WKWebView 在自己的进程里编译 JavaScript，也不需要 JIT 权限。
- 本地构建：

  ```bash
  APPLE_SIGNING_IDENTITY=- pnpm tauri build --target universal-apple-darwin --bundles app,dmg
  ```

  需要先安装两个目标：`rustup target add aarch64-apple-darwin x86_64-apple-darwin`。`APPLE_SIGNING_IDENTITY=-` 表示临时签名（ad-hoc）。

### 签名与公证（需要 Apple Developer Program 账号）

没有配置下面这些 secrets 时，工作流仍会构建，但只做临时签名：下载后 macOS 会提示「无法验证开发者」，用户需要在「系统设置 → 隐私与安全性」中允许打开。正式发布前应配置签名和公证。

在 GitHub 仓库的 **Settings → Secrets and variables → Actions** 中添加：

| Secret | 内容 |
|---|---|
| `APPLE_CERTIFICATE` | 「Developer ID Application」证书（含私钥）导出的 `.p12` 文件，再做 base64 编码 |
| `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 时设置的密码 |
| `APPLE_SIGNING_IDENTITY` | 证书名称，如 `Developer ID Application: Your Name (TEAMID1234)` |
| `APPLE_API_ISSUER` | App Store Connect API 密钥的 Issuer ID |
| `APPLE_API_KEY` | API 密钥的 Key ID |
| `APPLE_API_KEY_P8` | 下载的 `AuthKey_<KeyID>.p8` 文件的完整内容 |

前三项用于签名，后三项用于公证。只配置前三项时会签名但不公证。

准备方法：

1. **证书**：在 [Apple Developer → Certificates](https://developer.apple.com/account/resources/certificates/list) 创建「Developer ID Application」证书（需要账号持有人权限），下载后双击导入「钥匙串访问」。在钥匙串中右键该证书 → 导出为 `.p12`，设置一个密码。然后编码：

   ```bash
   base64 -i certificate.p12 | pbcopy
   ```

   把剪贴板内容填入 `APPLE_CERTIFICATE`。证书名称可以这样查看：

   ```bash
   security find-identity -v -p codesigning
   ```

2. **公证用的 API 密钥**：在 [App Store Connect → 用户和访问 → 集成 → App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api) 创建一个密钥（角色选「Developer」即可），记下 Issuer ID 和 Key ID，下载 `.p8` 文件（只能下载一次）。

这些都是敏感信息：只放在 GitHub secrets 中，不要提交到仓库，也不要发到聊天或 issue 里。

### 工作流做了什么

1. 有证书时，Tauri 把它导入一个临时钥匙串，签名 `.app`（包括 Hardened Runtime 和上面的权限声明）；有 API 密钥时，再提交公证并把公证票据装订（staple）到 `.app` 上。
2. 检查：`lipo -archs` 确认包含 `x86_64 arm64`，`codesign --verify --deep --strict` 校验签名；公证后再用 `spctl` 和 `stapler validate` 确认 Gatekeeper 接受。
3. 公证后，DMG 本身也提交公证并装订，离线打开时也不会出现警告。
4. 打标签时，把 DMG 上传到草稿 release。

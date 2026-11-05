# 发布 GlideGit

发布由 `.github/workflows/release.yml` 完成：推送 `vX.Y.Z` 标签后，构建安装包并上传到一个**草稿** release，确认无误后再手动发布。也可以在 Actions 页面手动运行这个工作流，此时只构建，安装包作为附件下载。

目前包含 macOS（M4-2）和 Windows（M4-3），自动更新见 M4-4。

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

草稿中会有三个文件：`GlideGit_X.Y.Z_universal.dmg`（macOS）、`GlideGit_X.Y.Z_x64-setup.exe`（Windows，NSIS）和 `GlideGit_X.Y.Z_x64_zh-CN.msi`（Windows，MSI）。

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

## Windows 安装包

- **NSIS 安装程序**（`*-setup.exe`）：按当前用户安装到 `%LOCALAPPDATA%\GlideGit`，不需要管理员权限，界面语言按系统语言选择简体中文或英文。推荐大多数用户使用。
- **MSI**：给需要用组策略或软件分发工具批量部署的场合，按机器安装。
- WebView2：Windows 10/11 一般已经自带；没有时安装程序会下载微软的引导程序来安装。

### 附带 MinGit

安装包里带一份 [MinGit](https://github.com/git-for-windows/git/wiki/MinGit)（精简版 Git for Windows，约 93 MB、376 个文件，安装后放在安装目录的 `mingit\` 下）。GlideGit 启动时依次查找 `GLIDEGIT_GIT` 环境变量、`PATH` 中的 git 和 `C:\Program Files\Git`，**都找不到时**才使用这份 MinGit；用户自己装的 git 总是优先，这样他们的版本和配置都照常生效。状态栏的 git 版本后会标注「（内置）」，鼠标悬停显示路径。

- 版本固定在 `scripts/fetch-mingit.mjs` 中（当前 2.56.0.4，64 位），下载后先核对 SHA-256 再解压到 `target/mingit`。升级时修改版本号、下载地址和 SHA-256 三处（SHA-256 可从 GitHub release 页面或 API 的 `digest` 字段获得）。
- 许可：MinGit 是 GPLv2 软件，其 `LICENSE.txt` 和各组件的许可文件随附在 `mingit\` 目录中。我们原样分发官方构建，没有修改；对应的源代码由 Git for Windows 项目在其 GitHub 上公开提供。
- 本地在 Windows 上打包前先运行 `node scripts/fetch-mingit.mjs`。不运行也能构建（构建脚本会建一个空目录占位），只是安装包里没有 MinGit。

### 代码签名

没有签名时安装包也能用，但首次运行时 SmartScreen 会提示「Windows 已保护你的电脑」，用户需要点「更多信息 → 仍要运行」。签名后由证书信誉决定是否还会提示（EV 证书立即建立信誉，普通 OV 证书需要一段时间的下载量）。

在仓库 secrets 中添加：

| Secret | 内容 |
|---|---|
| `WINDOWS_CERTIFICATE` | 代码签名证书（含私钥）的 `.pfx` 文件，做 base64 编码 |
| `WINDOWS_CERTIFICATE_PASSWORD` | `.pfx` 的密码 |

编码方法（PowerShell）：

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("certificate.pfx")) | Set-Clipboard
```

配置后，工作流把证书导入运行机器的证书存储，打包时用 signtool 签名 `glidegit.exe`、NSIS 安装程序和 MSI（SHA-256，DigiCert 时间戳），并检查三者的签名状态均为 Valid。

注意：2023 年 6 月以后签发的 OV/EV 代码签名证书，私钥必须存放在硬件（USB 令牌或云 HSM）中，无法导出为 `.pfx`。这种情况下需要改用云签名服务（如 Azure Trusted Signing、DigiCert KeyLocker），通过 Tauri 的 `bundle.windows.signCommand` 调用它们的签名工具；到时按所选服务调整「Configure signing」这一步。

### 工作流的检查

Windows 任务在打包后会静默安装 NSIS 安装包（`/S`），确认 `glidegit.exe` 已安装、自带的 `mingit\cmd\git.exe --version` 能运行，签名时再检查签名状态。

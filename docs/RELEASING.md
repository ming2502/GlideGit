# 发布 GlideGit

发布由 `.github/workflows/release.yml` 完成：推送 `vX.Y.Z` 标签后，构建两个平台的安装包和更新包，上传到草稿 release（说明取自 `CHANGELOG.md` 中这个版本的一节），全部成功后自动发布。也可以在 Actions 页面手动运行这个工作流，或在 main 上修改打包配置，此时只构建，安装包作为附件下载。

包含 macOS（M4-2）、Windows（M4-3）和应用内自动更新（M4-4）。

## 发布步骤

### 用发布脚本（推荐）

先把这个版本的更新内容写进 `CHANGELOG.md` 的 `## [Unreleased]` 一节并提交，然后在仓库根目录运行：

```bash
pnpm release 1.1.0
```

不带版本号时脚本会询问。它会依次：

1. 检查：在 main 分支、没有未提交的修改、不落后于远端、版本号比当前新、标签还不存在、`Unreleased` 一节不是空的。
2. 修改：三处版本号（以及 crate 之间路径依赖的版本），把 `## [Unreleased]` 改成 `## [1.1.0] - 今天的日期` 并在上面留一个新的空 `Unreleased`，更新 `Cargo.lock`。
3. 本地检查：类型检查、前端测试、编译。任何一步失败都会还原所有修改。
4. 列出要改的文件和这个版本的更新说明，**确认后**提交 `Release v1.1.0`、打附注标签并推送；之后由发布工作流构建并自动发布，脚本最后给出进度和发布页面的链接。

选项：`--dry-run` 只显示会改什么；`--no-push` 只在本地提交和打标签；`--no-checks` 跳过本地检查；`--yes` 不再询问。版本号可以带预发布后缀（`1.1.0-beta.1`），发布为预发布版本，不推送给自动更新。

### 手动

1. 统一版本号。以下三处必须一致，工作流会检查（`node scripts/check-version.mjs X.Y.Z`）：
   - `crates/app/tauri.conf.json` 的 `version`
   - `Cargo.toml` 的 `[workspace.package] version`（以及各 crate 之间路径依赖的 `version`）
   - `package.json` 的 `version`
2. 在 `CHANGELOG.md` 中写好 `## [X.Y.Z] - 日期` 一节。它就是 GitHub release 的说明，也是用户在应用内看到的「更新说明」；没有这一节时工作流在构建前就会停下（`node scripts/changelog-section.mjs X.Y.Z` 可以预览）。
3. 提交并推送，确认 CI 通过。
4. 打标签并推送：

   ```bash
   git tag -a vX.Y.Z -m "GlideGit X.Y.Z"
   ```

   ```bash
   git push origin vX.Y.Z
   ```

5. 等 Release 工作流完成。最后的 `publish` 任务确认草稿上的文件齐全后发布；任何一步失败都会留下草稿、不发布，修好后删除标签和草稿重新来过。

发布后，已安装的 GlideGit 就会收到更新提示（更新程序只读取最新的**已发布** release）。带连字符的标签（如 `v1.1.0-beta.1`）发布为预发布版本，不会推送给自动更新。

release 中的文件：

| 文件 | 用途 |
|---|---|
| `GlideGit_X.Y.Z_universal.dmg` | macOS 安装 |
| `GlideGit_X.Y.Z_x64-setup.exe` | Windows 安装（NSIS），也是 Windows 的更新包 |
| `GlideGit_X.Y.Z_x64_zh-CN.msi` | Windows 安装（MSI，批量部署） |
| `GlideGit_X.Y.Z_universal.app.tar.gz` | macOS 更新包 |
| `latest.json` | 更新清单 |

没有配置更新密钥时 `publish` 任务不会发布：没有更新功能的版本发出去之后，用户就再也收不到后续版本的自动更新。

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

## 自动更新

GlideGit 启动约 5 秒后（之后每 6 小时）检查一次 GitHub 上最新已发布 release 中的 `latest.json`。有新版本时，右下角提示版本号和更新说明，可以「安装并重启」「稍后」或「跳过此版本」；命令面板中的「Check for Updates…」可以随时手动检查。下载进度显示在状态栏。macOS 上替换应用后重启；Windows 上以 passive 模式运行新版 NSIS 安装程序（显示进度、无需操作）。

更新包用单独的**更新签名密钥**签名（与 Apple / Windows 的代码签名无关，免费、本地生成），应用内置公钥，只安装签名有效的包。签名中还记录了版本号，应用会拒绝「版本号与签名不符」的包，防止有人把旧版本的包冒充成新版本推给用户（`requireSignedVersion`）。

### 一次性配置

1. 生成密钥（在你自己的电脑上，设置一个密码）：

   ```bash
   pnpm tauri signer generate -w ~/.tauri/glidegit-updater.key
   ```

   会生成私钥 `glidegit-updater.key` 和公钥 `glidegit-updater.key.pub`。
2. 公钥放进仓库：复制到 `crates/app/updater.pub` 并提交（公钥不是秘密；当前的公钥 ID 为 `C73653DF0AD389FE`）。
3. 在 GitHub 仓库 **Settings → Secrets and variables → Actions → Secrets** 中添加 `TAURI_SIGNING_PRIVATE_KEY`：私钥文件的内容；`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：生成时设置的密码。
4. **妥善备份私钥和密码**（例如放进密码管理器）。私钥一旦丢失，已经安装的版本就再也无法收到更新，只能让用户手动下载新版本。私钥不要提交到仓库，也不要发到聊天或 issue 里。

公钥和私钥要么都配置，要么都不配置：只配置一半时工作流会给出警告，并按未启用更新来构建（这样构建出的版本不会被发布）。

### 工作流做了什么

1. 构建时由 `scripts/release-config.mjs` 把公钥写入配置（`plugins.updater.pubkey`，应用从这里读取），并用私钥为更新包签名：macOS 是 `GlideGit.app.tar.gz`（通用二进制，两种芯片共用），Windows 直接使用 NSIS 安装程序。
2. 用 `scripts/verify-update-sig.mjs` 按更新程序的算法，用仓库中的公钥校验刚签好的更新包。GitHub 上的私钥与公钥不配对时在这里失败，不会发布一个之后再也无法更新的版本。
3. 两个平台都完成后，`updater-manifest` 任务用 `scripts/updater-manifest.mjs` 生成 `latest.json`（版本号、更新说明、各平台下载地址和签名），连同 macOS 的更新包一起上传到草稿 release。

没有配置密钥时构建出的应用不带更新功能（命令面板中的「Check for Updates…」为灰色），开发版本和 fork 也是如此。

### 发布后验证

用和应用相同的方式检查已发布的 `latest.json`：下载本平台的更新包，校验签名和签名中的版本号：

```bash
cargo run -p glidegit --example updater_check -- https://github.com/ming2502/GlideGit/releases/latest/download/latest.json "$(cat ~/.tauri/glidegit-updater.key.pub)" 0.0.0
```

输出 `signature OK` 即表示用户可以正常更新。

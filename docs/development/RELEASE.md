# Windows x64 发版操作

安装器采用 Inno Setup 6.7.3。`tools/install_innosetup.ps1` 从官方 GitHub Release 获取编译器，并检查固定的 SHA-256。项目 DLL 直接来自仓库已登记运行库，打包时逐文件核对内容哈希，不依赖开发机上的外部目录。

## 自动验证

`.github/workflows/windows.yml` 在 PR 和 main 上执行快速检查：格式检查、Clippy、task-engine/renderer-host 单元测试和 CLI 审计回归。普通提交不再运行全量测试、Release 构建或安装器验收。仅保留 PR 与 main 两类触发，避免同一 PR 分支提交重复执行。

发版 workflow 复用该文件，并选择验证模式。版本标签执行 `full`：格式检查、Clippy、工作区单元/集成测试、串行 doctest、Release 构建和安装器验收。手动运行默认选择 `package`：仅做格式检查、Release 构建和包内转换、安装/卸载验收；需要完整复查时再选择 `full`。完整测试已经包含 CLI 审计回归，不再单独重复运行。

运行库测试启用真实 libass，缺失时失败。解压后的便携包和实际安装后的目录必须通过全部文件 SHA-256、宿主握手、ASS→BDN/PNG、ASS→SUP 和 SUP Standard 校验。安装器测试在 GitHub 临时 Windows runner 中验证静默安装、关联默认关闭、卸载保留/删除数据。all-users 仍只有静态声明检查，不代表真实 UAC 验收。

PR 分支需要单独检查安装包时，可手动运行 `Windows Rust checks`，选择该分支和 `package` 模式。它与该分支的快速检查互不取消。

`package` 或 `full` 验收完成后，Actions 的 `windows-x64-release` 资产包含：

- `SubtitleToolkit-<版本>-windows-x64-portable.zip`
- `SubtitleToolkit-<版本>-setup.exe`
- `release-manifest.json`：Cargo 版本、标签、提交、平台、是否签名
- `SHA256SUMS.txt`：上面各项资产的 SHA-256

## 标签与 Release 草稿

先合并 PR，并确认主分支检查通过。Cargo workspace 的版本号是唯一版本来源。标签允许 `v<版本>` 或 `v<版本>-preview.<序号>`；不能用标签覆盖另一 Cargo 版本。

推送标签后，`.github/workflows/release.yml` 重新执行完整验证，检查标签提交已经合入 main，核对下载资产哈希和提交身份，然后创建 Release **草稿**。预览标签自动标记 prerelease。最终在 GitHub 检查草稿中的版本、说明和附件，再发布。脚本不覆盖已经存在的 Release，也不移动旧标签。

```powershell
git tag -a v0.1.0-preview.2 -m '发布：0.1.0 预览版 2'
git push origin v0.1.0-preview.2
```

这是使用示例，不表示该标签已经发布。手动运行 `Windows 发布` workflow 只构建和验证，不创建标签或 Release。

## 本地打包

```powershell
. .\tools\set_build_cache.ps1
$iscc = .\tools\install_innosetup.ps1
.\tools\make_release.ps1 -IsccPath $iscc
```

默认输出到 Cargo 目标目录下的新目录；指定 `-OutDir` 时必须是尚不存在的目录。`-NoBuild` 使用已有 Release 文件。脚本先编译 Inno Setup 安装器，再解压便携 ZIP 执行真实转换核验，最后生成 `assets/`。安装/卸载行为测试禁止在日常开发账户运行。

## 签名

当前没有可用的受信任代码签名证书。未配置证书时，清单中 `signed=false`，Release 草稿说明未签名状态。

需要签名时，在仓库 Actions Secrets 配置 `STK_SIGNING_PFX_BASE64` 和 `STK_SIGNING_PFX_PASSWORD`。只有 main 分支或版本标签构建导入证书；PR 不导入。脚本使用当前用户证书库签署三个项目程序和最终安装器，附 SHA-256 RFC 3161 时间戳，再用 SignTool `/pa` 验证。私钥不写入软件包、日志或 Actions 资产。临时 runner 随任务结束销毁。

签名开关不表示已有受信任证书；证书必须支持 Code Signing、含私钥且受 Windows 信任。硬件令牌或云托管证书需另接对应签名服务。独立卸载器当前不单独签名。

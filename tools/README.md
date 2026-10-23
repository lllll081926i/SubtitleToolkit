# 工程工具

从仓库根目录运行命令。构建产物使用 Cargo 的目标目录；门阶日志固定保存到仓库的 `target/gate/`。

PowerShell 中先点源 `. .\tools\set_build_cache.ps1`，确保 Cargo、vcpkg、sccache/ccache、pip 缓存、构建输出和临时目录不在 C 盘。`check_all.sh` 会在执行前检查并纠正这些路径。

Cargo 配置将编译子进程的临时目录固定为仓库 `target/tmp/`。初始化脚本和 Git Bash 入口都先创建该目录，避免自定义 `CARGO_TARGET_DIR` 时链接器找不到临时目录。其他缓存跟随 Cargo 目标目录；初始化脚本保留已有的非 C 盘设置。

| 工具 | 用途 |
|---|---|
| `set_build_cache.ps1` | 初始化开发会话的非 C 盘缓存与临时目录 |
| `make_release.ps1` | 统一生成便携包、Inno Setup 安装器和发布校验和；验证解压包的真实转换 |
| `install_innosetup.ps1` | 下载固定版本 Inno Setup，检查 SHA-256 后安装编译器 |
| `test_distribution.ps1` | 核对软件包文件并用包内程序执行 BDN/SUP 转换 |
| `make_portable.ps1` | 把 release 产物打成 portable zip；可用 `-Sign` 签署包内项目可执行文件 |
| `make_installer.ps1` | 用 Inno Setup 编出安装器；可用 `-Sign` 签署项目可执行文件和安装器 |
| `sign_authenticode.ps1` | 用 Windows SDK SignTool 签署并验证 Authenticode 文件签名 |
| `verify_installer.ps1` | 把 28 §3 的五条要求真跑一遍（静默装卸载、查注册表与用户数据），逐项报 PASS/FAIL |
| `check_all.ps1` / `check_all.sh` | Windows 入口与完整验证；严格模式不允许缺依赖跳过 |
| `doc_claims_check.py` | 历史可追溯矩阵中的符号是否仍存在；只作归档材料核对，不证明行为正确 |
| `audit_evidence.py` | 历史门阶日志与历史声明对照；不替代行为测试 |
| `issue_status_audit.py` | 历史问题清单与历史状态的一致性检查 |
| `rule_coverage.py` | 校验规则点名覆盖盘点，不等于语义覆盖率 |
| `notices.py` | 依赖声明和第三方来源核对 |
| `png_pillow_check.py` / `subs_pysubs2_check.py` | PNG 与字幕的外部实现交叉检查 |
| `make_probe_fixtures.sh` / `make_embedded_fixture.py` | 测试素材生成；内嵌字体生成器当前存在编码问题，见审查报告 |
| `check_injection_report.py` | 字体注入报告结构检查 |
| `vcpkg_mirror_retry.sh` | 历史本机 native 依赖构建辅助；使用前检查工具路径 |

## 完整验证

```powershell
$env:RELEASE = '1'
$env:STK_LIBASS_DLL = 'D:/dependencies/libass/ass-9.dll'
$env:STK_FFPROBE = 'D:/dependencies/ffmpeg/ffprobe.exe'
$env:STK_MKVTOOLNIX_DIR = 'D:/dependencies/mkvtoolnix'
& ./tools/check_all.ps1 --strict
```

上面的依赖目录是示例，替换为实际安装路径。Git for Windows 的 Bash、可用的 Python、Pillow、pysubs2、cargo-deny 和 cargo-nextest 也必须可用。一次只运行一套完整验证。

日常核验优先直接使用 Cargo：`cargo test --workspace --lib --tests --no-fail-fast` 运行 Rust 行为测试，`cargo test --workspace --doc --no-fail-fast -j 1` 串行检查 doctest。并行 workspace doctest 在本机曾报 `E0463`。历史门阶声明保留在 `docs/history/`，不代表当前通过；运行脚本本身也不能证明格式互操作。

## portable 打包

```powershell
. .\tools\set_build_cache.ps1
.\tools\make_portable.ps1            # 构建后打包
.\tools\make_portable.ps1 -NoBuild   # 只打包现有产物
$env:STK_SIGNING_CERT_THUMBPRINT = '证书 SHA-1 指纹'
.\tools\make_portable.ps1 -Sign
```

先将代码签名证书导入当前用户 `Cert:\CurrentUser\My`。签名脚本只使用证书指纹，不读取或输出私钥。证书必须有私钥、未过期并包含 Code Signing EKU；脚本用 SHA-256 和 RFC 3161 时间戳签名，再用 SignTool `/pa` 策略验证。缺少证书时脚本失败，不会创建自签名证书冒充受信任签名。

产出 zip 位于 Cargo 目标目录的 `portable/`。包内为 CLI、renderer-host 与 GUI 三个可执行文件、20 个 libass 运行库 DLL、fontconfig 配置、逐包许可证材料，加上 `LICENSE-MIT`/`LICENSE-APACHE`/`THIRD_PARTY_NOTICES.md`/`sbom.cdx.json`/`README.md`、`portable.flag` 与 `SHA256SUMS.txt`。

## 安装器

```powershell
. .\tools\set_build_cache.ps1
.\tools\make_installer.ps1             # 构建后打包
.\tools\make_installer.ps1 -NoBuild    # 只用现有产物
.\tools\make_installer.ps1 -NoBuild -Sign
pwsh -NoProfile -File .\tools\verify_installer.ps1   # 仅 GitHub Actions 临时 runner
```

公开开源项目可以申请 [SignPath Foundation](https://signpath.org/) 的免费代码签名。申请获批后仍需在 GitHub/SignPath 配置项目、策略和凭据；本地签名脚本不代替这些外部配置。

产出 `SubtitleToolkit-<版本>-setup.exe` 位于 Cargo 目标目录的 `installer/`。
五条要求（per-user 默认、可选 all-users、关联 opt-in、legacy 可选、卸载保留数据由用户选）
的落法与验证见 [28 §3](../docs/development/28_WINDOWS_BUILD_RELEASE.md) 与
[安装器裁决](../docs/reviews/2026-10-17-windows-installer.md)。

`verify_installer.ps1` 只允许在 GitHub Actions 临时 Windows runner 中运行。它会改写测试账户的文件关联和用户数据，禁止在日常开发账户运行。

边界（不要把它们当成已完成）：

- 安装包包含通过 `third_party/LIBASS_RUNTIME_LOCK.json` 核验的 20 个 libass DLL、fontconfig 配置、各 MSYS2 包元数据和许可证正文。需要更新 DLL 时，先更新来源锁与许可材料，再运行打包脚本；脚本拒绝哈希不符或文件缺失的运行库。
- **签名需证书**。`-Sign` 已接入便携包和安装器构建；当前开发机没有可用的代码签名证书，因此尚未生成受信任签名产物。
- **安装器用 Inno Setup 6.7.3 编译**，不是 28 §3 推荐的 7（用到的特性 6 均有，已编译且实跑通过）。
- **all-users 模式只核对了静态声明**，未做实机 UAC 验证（静默环境应答不了 UAC，且提权安装会真写 `HKLM` 与 `%ProgramFiles%`）。
- **x86 renderer host 未构建**，安装器把它当可选组件并告警。
- 打包前不做验收声明，只重新计算内容哈希；是否可发布以 `docs/STATUS.md` 为准。

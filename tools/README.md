# 工程工具

从仓库根目录运行命令。构建产物使用 Cargo 的目标目录；门阶日志固定保存到仓库的 `target/gate/`。

PowerShell 中先点源 `. .\tools\set_build_cache.ps1`，确保 Cargo、vcpkg、sccache/ccache、pip 缓存、构建输出和临时目录不在 C 盘。`check_all.sh` 会在执行前检查并纠正这些路径。

Cargo 配置将编译子进程的临时目录固定为仓库 `target/tmp/`。初始化脚本和 Git Bash 入口都先创建该目录，避免自定义 `CARGO_TARGET_DIR` 时链接器找不到临时目录。其他缓存跟随 Cargo 目标目录；初始化脚本保留已有的非 C 盘设置。

| 工具 | 用途 |
|---|---|
| `set_build_cache.ps1` | 初始化开发会话的非 C 盘缓存与临时目录 |
| `make_portable.ps1` | 把已验收的 release 产物打成 portable zip（含 `SHA256SUMS.txt`、许可正文、`sbom.cdx.json` 与 `portable.flag`）；不含第三方 DLL、不签名 |
| `make_installer.ps1` | 用 Inno Setup 编出 `SubtitleToolkit-<版本>-setup.exe`（28 §3）；找不到 ISCC 或编译失败均退非零 |
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
```

产出 zip 位于 Cargo 目标目录的 `portable/`。包内为 CLI、renderer-host 与 GUI 三个可执行文件，加上 `LICENSE-MIT`/`LICENSE-APACHE`/`THIRD_PARTY_NOTICES.md`/`sbom.cdx.json`/`README.md`、`portable.flag` 与 `SHA256SUMS.txt`。

## 安装器

```powershell
. .\tools\set_build_cache.ps1
.\tools\make_installer.ps1             # 构建后打包
.\tools\make_installer.ps1 -NoBuild    # 只用现有产物
pwsh -NoProfile -File .\tools\verify_installer.ps1   # 把 28 §3 跑一遍
```

产出 `SubtitleToolkit-<版本>-setup.exe` 位于 Cargo 目标目录的 `installer/`。
五条要求（per-user 默认、可选 all-users、关联 opt-in、legacy 可选、卸载保留数据由用户选）
的落法与验证见 [28 §3](../docs/development/28_WINDOWS_BUILD_RELEASE.md) 与
[安装器裁决](../docs/reviews/2026-10-17-windows-installer.md)。

`verify_installer.ps1` 只写 `target\tmp\installer-verify-<随机>`、`HKCU` 下相关键，
并会**备份并恢复** `%LOCALAPPDATA%\subtitletool` —— 开发机上那份可能是你真正在用的数据。

边界（不要把它们当成已完成）：

- **不包含第三方运行库 DLL**。libass 那组 MSYS2 DLL 的分发义务仍在审计中（21 §3、`third_party/README.md`），未完成前不进发行物；渲染依赖由用户在目标机上设 `STK_LIBASS_DLL`。
- **未签名**。Authenticode（28 §5）尚未实现；未签名的 `setup.exe` 在 SmartScreen 上会警告。
- **安装器用 Inno Setup 6.7.3 编译**，不是 28 §3 推荐的 7（用到的特性 6 均有，已编译且实跑通过）。
- **all-users 模式只核对了静态声明**，未做实机 UAC 验证（静默环境应答不了 UAC，且提权安装会真写 `HKLM` 与 `%ProgramFiles%`）。
- **x86 renderer host 未构建**，安装器把它当可选组件并告警。
- 打包前不做验收声明，只重新计算内容哈希；是否可发布以 `docs/STATUS.md` 为准。

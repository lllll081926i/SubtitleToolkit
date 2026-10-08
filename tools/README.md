# 工程工具

从仓库根目录运行命令。构建产物使用 Cargo 的目标目录；门阶日志固定保存到仓库的 `target/gate/`。

PowerShell 中先点源 `. .\tools\set_build_cache.ps1`，确保 Cargo、vcpkg、sccache/ccache、pip 缓存、构建输出和临时目录不在 C 盘。`check_all.sh` 会在执行前检查并纠正这些路径。

Cargo 配置将编译子进程的临时目录固定为仓库 `target/tmp/`。初始化脚本和 Git Bash 入口都先创建该目录，避免自定义 `CARGO_TARGET_DIR` 时链接器找不到临时目录。其他缓存跟随 Cargo 目标目录；初始化脚本保留已有的非 C 盘设置。

| 工具 | 用途 |
|---|---|
| `set_build_cache.ps1` | 初始化开发会话的非 C 盘缓存与临时目录 |
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
& ./tools/check_all.ps1 --strict
```

上面的依赖目录是示例，替换为实际安装路径。Git for Windows 的 Bash、可用的 Python、Pillow、pysubs2、cargo-deny 和 cargo-nextest 也必须可用。一次只运行一套完整验证。

日常核验优先直接使用 Cargo：`cargo test --workspace --lib --tests --no-fail-fast` 运行 Rust 行为测试，`cargo test --workspace --doc --no-fail-fast -j 1` 串行检查 doctest。并行 workspace doctest 在本机曾报 `E0463`。历史门阶声明保留在 `docs/history/`，不代表当前通过；运行脚本本身也不能证明格式互操作。

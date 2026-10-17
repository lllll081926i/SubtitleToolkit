# SubtitleToolkit

面向 Windows 的 Rust 字幕转换与适配工具箱。输入现有字幕，通过 CLI 完成文本转换、时间调整、颜色调整、字体检查和批量处理。

**当前处于开发阶段，不是正式发布版本。** GUI、CSRI/VSFilterMod、字体子集和安装包尚未完成。现有实现与验证范围见[当前状态](docs/STATUS.md)和[2026-10-08 代码审查](docs/reviews/2026-10-08.md)。新增规划与文档审查建议的处理结果见[文档审查采纳记录](docs/planning/AUDIT_ADOPTION_20261008.md)。

## 快速开始

准备 Rust stable、Visual Studio Build Tools 的 C++ 工具链和 Windows SDK。正式目标为 `x86_64-pc-windows-msvc`。

PowerShell 开发会话先点源 `. .\tools\set_build_cache.ps1`，让 Cargo、vcpkg、sccache/ccache、pip 缓存、构建输出和临时目录使用非 C 盘路径。

```powershell
. .\tools\set_build_cache.ps1
cargo build --workspace --release
cargo run -p cli -- doctor --json
cargo run -p cli -- --help
```

未设置 `CARGO_TARGET_DIR` 时，CLI 位于 `target/release/subtitletool-cli.exe`，渲染宿主位于 `target/release/subtitletool-render-host.exe`。自定义构建目录以 `cargo metadata --no-deps --format-version 1` 的 `target_directory` 为准。

可通过 `STK_LIBASS_DLL`、`STK_FFPROBE`、`STK_FONTS_DIR`、`STK_MKVTOOLNIX_DIR` 指定依赖。构建产物和私有字体不随源码提交；`third_party/VSFilterMod.dll` 是尚未接入产品的候选文件，来源和分发许可仍需核实。当前不能将“编译成功”解释为“BDN/PGS 已兼容外部播放器”。

项目已登记用户提供的 libass 0.17.5 开发期运行库与依赖。启用真实渲染时设置 `$env:STK_LIBASS_DLL = (Resolve-Path '.\third_party\libass-9.dll').Path`；版本、哈希、测试命令与分发边界见[第三方说明](third_party/README.md#libass-开发期运行库)。

`mkv` 命令需要外部 MKVToolNix，用 `--tools-dir DIR` 或环境变量 `STK_MKVTOOLNIX_DIR` 指定安装目录。

## 目录

| 路径 | 职责 |
|---|---|
| `crates/` | 20 个 Rust workspace 成员；各模块的集成测试和素材随模块保存 |
| `docs/` | 产品规格、计划、开发指南、当前状态和历史审查 |
| `tools/` | 验证、夹具生成和依赖审计脚本 |
| `third_party/` | 第三方来源锁、研究源码归档与未集成的 DLL 候选；GPL 来源与限制见第三方声明 |
| `target/` | 本地构建及验证证据，Git 忽略 |
| `audit/` | 接手前已有的本地审查材料，保留原状，不作为当前验收证据 |

## 开发与验证

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --tests --no-fail-fast
cargo test --workspace --doc --no-fail-fast -j 1
cargo build --workspace --release
```

本机并行 workspace doctest 会报 `E0463`，串行 `-j 1` 可通过；因此将 Rust 行为测试与 doctest 分开运行。PNG、字幕库和真实 native 渲染交叉验证需要对应外部依赖；缺依赖时应单独报告，不要把跳过记为通过。`tools/check_all.ps1 --strict` 是可选的综合门阶，不是日常验证入口。

- [开发说明](CONTRIBUTING.md)
- [文档导航](docs/README.md)
- [目录约定与迁移记录](docs/development/REPOSITORY_LAYOUT.md)
- [验证脚本说明](tools/README.md)
- [第三方声明](THIRD_PARTY_NOTICES.md)

本项目采用 `MIT OR Apache-2.0` 双许可，随你选择其一。正文见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。第三方源码与运行库按各自许可证使用，见 [third_party/README.md](third_party/README.md)。

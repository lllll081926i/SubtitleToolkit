# Repository Guidelines

## 项目结构

本项目采用 Rust 2021 workspace，正式目标为 `x86_64-pc-windows-msvc`。`crates/core-model` 定义共享模型；`codec-*` 处理格式；`transform-*` 处理时间、颜色和分辨率；`crates/cli` 提供命令行；`crates/renderer-host` 隔离 libass 渲染进程。单元测试位于模块内部，集成测试位于 `crates/*/tests/`，素材位于相应 crate 的 `tests/fixtures/`。`docs/` 保存规格与验收依据，`tools/` 保存可选验证工具，`third_party/` 保存来源锁与研究材料。

## 构建与开发命令

安装 Rust stable、Visual Studio Build Tools、Windows SDK。在仓库根目录执行：

PowerShell 开发会话先点源 `. .\tools\set_build_cache.ps1`，将空缺或指向 C 盘的 Cargo 缓存、构建输出和临时目录改到仓库所在磁盘；已有的非 C 盘路径会保留。

脚本也会将 vcpkg 下载与二进制缓存、sccache、ccache 和 pip 缓存重定向到 Cargo 目标目录下。运行任何会构建或下载依赖的命令前，先初始化该会话；直接运行 Cargo 命令不会自动加载 PowerShell 脚本。

- `cargo build --workspace --release`：构建 CLI 和渲染宿主，产物位于 Cargo 目标目录的 `release/`。
- `cargo run -p cli -- doctor --json`：检查依赖和已实现能力。
- `cargo test --workspace --lib --tests --no-fail-fast`：运行工作区单元与集成测试。
- `cargo test --workspace --doc --no-fail-fast -j 1`：串行检查工作区 doctest；本机并行 doctest 会报 `E0463`。
- `cargo test -p codec-ass --test roundtrip`：运行 ASS 往返测试。
- `cargo fmt --all --check`：检查格式。
- `cargo clippy --workspace --all-targets -- -D warnings`：检查代码并拒绝警告。
- 在 Git Bash 执行 `RELEASE=1 bash tools/check_all.sh --strict`：运行完整验证，缺依赖的跳过也判失败。脚本会把 C 盘缓存、构建目录和临时目录改到非 C 盘。日志位于仓库 `target/gate/`。

先运行受影响 crate 的 Rust 测试，再运行工作区单元/集成测试、串行 doctest 和 workspace 构建。外部交叉验证依赖本机工具与素材；逐项报告通过、失败和跳过。历史门阶见 `docs/history/2026-10-06/REVIEW_GATES.md`，它不代表当前执行结果。

## 代码风格与命名

Rust 使用四空格缩进，运行 `cargo fmt --all` 格式化。函数与模块使用 `snake_case`，类型使用 `PascalCase`，常量使用 `SCREAMING_SNAKE_CASE`。注释、文档与提交使用中文，标识符保持英文。改动保持集中；API 或架构变更同步更新对应文档。

## 测试要求

使用内置 `#[test]`。测试名称描述行为，例如 `doctor_json_is_one_line_and_names_missing_capabilities`。缺陷修复增加针对行为的回归用例，覆盖边界输入和失败路径。当前没有配置数值覆盖率阈值。需要外部交叉验证时运行对应真实实现；报告区分通过、失败和跳过。

## 提交与 Pull Request

本目录接手时没有 Git 历史，当前仅建立审查基准，尚无长期提交约定。建议使用中文动作摘要，例如 `修复：保留 ASS 未知标签`；每次提交只处理一个主题。

PR 说明问题、行为变化、验证命令与结果。关联适用 issue 或本地问题编号。列出缺失依赖和跳过项；渲染变化附前后图像与复现素材。不得将规划项或历史测试结果表述为本次已验证。

## 配置与文件安全

通过 `STK_LIBASS_DLL`、`STK_FFPROBE`、`STK_FONTS_DIR` 指定依赖，优先使用绝对路径。保留 `Cargo.lock`；不提交构建产物、未登记的外部 DLL 或私有字体。已明确纳入的第三方二进制必须登记来源、许可证状态和 SHA-256；未完成来源与许可证核验的文件不得进入安装包。依赖变更同步核对 `deny.toml`、第三方声明和审计锁定记录。清理文件移入 `.trash/`，不得清理其他会话资产。

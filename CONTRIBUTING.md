# 开发说明

## 环境

在 Windows x64 使用 Rust stable、MSVC C++ 工具链及 Windows SDK。仓库的 `.cargo/config.toml` 配置了 rsproxy 镜像；网络环境不同的机器应在明确了解 Cargo 源替换规则后调整。保留 `Cargo.lock`。

PowerShell 开发会话先点源 `. .\tools\set_build_cache.ps1`。脚本会将空缺或指向 C 盘的 Cargo、vcpkg、sccache/ccache、pip 缓存、构建输出和临时目录放到仓库所在磁盘，并保留已有的非 C 盘设置。Git Bash 验证入口也会执行相同检查。直接运行 Cargo 命令前也要初始化会话；Cargo 配置只能重定向编译子进程环境，不能替代 `CARGO_HOME` 和 `CARGO_TARGET_DIR` 的会话设置。

原生依赖通过环境变量指定绝对路径，不依赖仓库旁边恰好存在某个工具目录。Python 必须是可实际运行的解释器，WindowsApps 的占位启动器不算已安装。

## 工作步骤

1. 阅读[当前状态](docs/STATUS.md)和[文档导航](docs/README.md)。
2. 保持单次改动集中。接口和架构变更同步修改对应规格或决策。
3. 为缺陷修复添加行为回归用例，覆盖应拒绝与应接受的输入。
4. 执行格式、Clippy、适用测试和 release 构建。
5. 先运行受影响 crate 的行为测试，再运行工作区单元/集成测试和串行 doctest。
6. 在交付说明中分别报告通过、失败和缺依赖跳过的检查。

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --tests --no-fail-fast
cargo test --workspace --doc --no-fail-fast -j 1
cargo build --workspace --release
```

本机并行 workspace doctest 曾报 `E0463`，串行 `-j 1` 通过。分开执行可保留完整测试覆盖，并避免把并发 rustdoc 失败误判为源码错误。

外部实现交叉验证需要本机 native DLL、ffprobe、Pillow 或 pysubs2。没有这些依赖时只报告实际运行的 Rust 测试结果，不引用历史测试数量。

## 文件约定

模块代码位于 `crates/<crate>/src/`。模块集成测试位于 `crates/<crate>/tests/`，字节敏感素材位于该模块的 `tests/fixtures/`。没有跨模块素材需求时，不创建空的根目录 `tests/`。

从 `docs/README.md` 进入当前文档。历史快照放在 `docs/history/`，新增审查放在 `docs/reviews/`。第三方归档放在 `third_party/sources/`，来源与哈希登记在 `third_party/SOURCE_AUDIT_LOCK.json`。

不提交 `target/`、未登记 DLL、EXE、私有字体或本地日志。已纳入的第三方 DLL 必须在 `third_party/SOURCE_AUDIT_LOCK.json` 和第三方声明中登记；来源或许可待核的 DLL 不得进入安装包。不覆盖其他会话的资产。清理文件时移入 `.trash/` 或回收站。

提交使用中文摘要。代码标识符保留英文。PR 说明问题、行为变化、验证结果和剩余限制。

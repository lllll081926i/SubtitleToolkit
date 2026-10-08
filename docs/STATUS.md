# 当前项目状态

核对日期：2026-10-08。本文描述当前工作区源码和实际执行的验证，不引用历史门阶结果充当本轮通过证据。

## 项目边界

- 项目面向 Windows x64，正式 Rust target 为 `x86_64-pc-windows-msvc`。
- 当前交付形态是 Rust CLI；GUI、安装包、portable 包、签名和更新器未实现。
- Cargo 工作区包含 `subtitletool-cli.exe` 与 `subtitletool-render-host.exe`。renderer host 用于隔离 libass DLL。
- `P0` 等级表示产品目标优先级，不表示该功能已交付。

## 已有实现

- ASS/SSA、SRT 解析与转换，时间、颜色和分辨率变换。
- CLI 批处理、输出事务、失败恢复、JSON 报告和 `doctor`。
- ffprobe adapter；视频探测只给建议，不自动改变字幕时间轴。
- libass 渲染、renderer-host 进程隔离、BDN XML/PNG 输出和 Basic PGS 编码链。
- PGS 段解析、Fast Path 变换、分层校验及 ODS 内容摘要。
- 字体扫描、缺字报告、整字体复制与 manifest、渲染字体身份和字体注入。

实现边界和验收要求见[功能矩阵](specs/03_FEATURE_MATRIX.md)及[验收用例](specs/25_ACCEPTANCE_TESTS.md)。

## 未交付或未裁决

- `font subset`、MKV helper、CSRI/VSFilterMod provider、x86 renderer host、桌面 GUI 和发行安装链未实现。
- 字体服务未使用 DirectWrite 作为权威 resolver；字体子集正确性尚无复杂文字渲染语料。
- PGS Basic 编码不等于 Strict Blu-ray 合规。PCS 帧率字段、palette range/matrix、ODS 图末标记尚无真盘/设备结论；当前 ffprobe 证据只覆盖 `.sup` 容器段布局与时钟。
- Strict validator 需要尚未实现的缓冲模型和设备语料，因此不会给出 Strict 通过结论。
- 仓库未补根目录 MIT/Apache-2.0 license 正文；公开发布前需要处理。

## 本轮验证

本轮新增 renderer-host EOF/读取错误退出、有界接收队列、IPC 字节拒绝检查，以及覆盖整个 ffprobe 生命周期的超时控制。修复 Cargo 临时目录未创建导致的 `LNK1104`。完整证据与范围见[2026-10-08 修复记录](reviews/2026-10-08.md)。

- `cargo fmt --all --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo build --workspace --release`：通过。
- `cargo test --workspace --lib --tests --no-fail-fast`：55 个单元与集成测试目标退出 0，包含 7 项真实系统字体测试。该完整运行在本轮新增宿主/超时修复前启动；最终代码由以下受影响 crate 测试重跑核验。
- `cargo test -p renderer-host --lib --tests --no-fail-fast`：13 项单元测试通过；实际执行 2 项集成场景，3 项缺 libass 的集成场景提前返回。
- `cargo test -p media-probe --lib -- --show-output`：23 项单元测试通过，包含 3 项真实进程生命周期超时回归。
- 指定本机 ffprobe，设置 `STK_REQUIRE_FFPROBE=1` 后运行 `cargo test -p media-probe --test real_probe -- --show-output`：6 项真实工具测试通过。
- `cargo test --workspace --doc --no-fail-fast -j 1`：20 个 crate 的 doctest 目标退出 0；各目标实际用例数为 0。
- 发布版 `subtitletool-cli.exe doctor --json`：返回 `ok=true`；确认 ffprobe 可用，renderer-host 因没有 libass DLL 报为不可用。

本轮没有可用 libass DLL。依赖真渲染的 CLI 集成用例按约定跳过；不得把跳过视作真渲染通过。设置 `STK_REQUIRE_LIBASS=1` 可要求缺依赖时失败。

# 当前项目状态

核对日期：2026-10-09。本文描述当前工作区源码和实际执行的验证，不引用历史门阶结果充当本轮通过证据。

同日后续更新：
- 维护文档已按[38 项建议采纳记录](planning/AUDIT_ADOPTION_20261008.md)优化。新增用户提供的 libass 运行库与真实验证见[追加记录](reviews/2026-10-08-docs.md)。
- 2026-10-09 完成了全项目深度审查、测试过度防御修复与纯 Rust 前端规格重构，详见[2026-10-09 审查报告](reviews/2026-10-09-code-audit-defense.md)。
- 2026-10-16 落地了该审查的四项代码建议（IPC 自残后门、字体缓存坏就用坏即弃、Strict 不再被未实现层阻断、XML 加固幅述校正），证据与未做项见[审查报告第五节](reviews/2026-10-09-code-audit-defense.md)。新增 GUI crate（`crates/gui`，Iced 工作台骨架）可编译，但任务执行链未接通真实引擎、无测试，不计为已交付。
- 2026-10-16 核验了 Phase 0–2 的退出条件（均满足），并补齐 AT-BATCH-001 的 100×3 容量用例；仍未覆盖项与证据见[实施检查表](planning/39_IMPLEMENTATION_CHECKLIST.md)。
- 2026-10-17 推进 Phase 6 发布起步：补齐根目录双许可正文，新增 `tools/make_portable.ps1` 并实测（解包后 CLI 与 renderer-host 可运行、`SHA256SUMS.txt` 校验通过）。安装器、签名、便携数据目录与第三方 DLL 分发审计仍未完成，见 [28 节](development/28_WINDOWS_BUILD_RELEASE.md)。
- 2026-10-17 修正依赖许可门禁：`tools/notices.py` 原先只读已解压源码（平台专属依赖从不下载，误报 37 项"读不到 license"），且把 SPDX 的 `A OR B` 拆开逐个要求白名单（把 `Apache-2.0 OR GPL-2.0-only` 误报成 GPL 污染）。已改为回退读 `.crate` 归档、按 SPDX 语义判定，并新增 `--fetch`。真实结论：无 GPL/LGPL 强制义务；`gui` 关闭 iced 的 `svg` 特性后去掉了强制 MPL-2.0 的 `resvg`/`usvg`（依赖 487→470 包）。`THIRD_PARTY_NOTICES.md` 已重新生成。

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
- 全局环境测试加速：配置 `[profile.dev.package."*"] opt-level = 3`，纯算法与字体计算在 dev 模式下提速 10~50 倍。
- 优化了测试门禁：真实系统字体测试默认跳过（显式指定 `STK_REQUIRE_FONTS=1` 或 `STK_FONTS_DIR` 触发），`fontenv` 单测使用隔离单字体测试目录，彻底消除全量扫描 `C:\Windows\Fonts` 导致的单测超时卡死。
- 前端设计重构：将原本物理割裂的 10 个独立页面重塑为“三栏一体化主工作台 + 单帧实时预览 Canvas + 多 Profile 矩阵”，见 [05 UI 页面规格](specs/05_UI_SCREEN_SPEC.md) 与 [35 UI 交互规范](specs/35_UI_STATE_INTERACTION.md)。

实现边界和验收要求见[功能矩阵](specs/03_FEATURE_MATRIX.md)及[验收用例](specs/25_ACCEPTANCE_TESTS.md)。

## 未交付或未裁决

- `font subset`、MKV helper、CSRI/VSFilterMod provider、x86 renderer host、桌面 GUI 和发行安装链未实现。
- 字体服务未使用 DirectWrite 作为权威 resolver；字体子集正确性尚无复杂文字渲染语料。
- PGS Basic 编码不等于 Strict Blu-ray 合规。PCS 帧率字段、palette range/matrix、ODS 图末标记尚无真盘/设备结论；当前 ffprobe 证据只覆盖 `.sup` 容器段布局与时钟。
- Strict validator 需要尚未实现的缓冲模型和设备语料，因此不会给出 Strict 通过结论。
- 仓库已补根目录 MIT/Apache-2.0 license 正文（`LICENSE-MIT`、`LICENSE-APACHE`，2026-10-17）；portable 包可由 [tools/make_portable.ps1](../tools/README.md) 生成。安装器、Authenticode 签名、GUI 与第三方 DLL 分发审计仍未完成。

## 2026-10-08 早前代码审查验证

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

该次运行没有可用 libass DLL。依赖真渲染的 CLI 集成用例按约定跳过；这些历史结果仍不能作为真渲染通过。设置 `STK_REQUIRE_LIBASS=1` 可要求缺依赖时失败。

## 2026-10-08 追加真实 libass 验证

用户提供的 `third_party/libass-9.dll` 已对应 MSYS2 UCRT64 0.17.5-1 官方包。按原包构建版本补齐并登记 19 个依赖 DLL。修正 `ass_read_memory` 的 codepage 参数为 UTF-8；来源、许可和分发边界见[第三方说明](../third_party/README.md#libass-开发期运行库)。

- `cargo test -p renderer-host --test host_isolation`：5 项全部实际执行并通过，覆盖像素相等、超时、宿主故障重试和位数/身份拒绝。
- `cargo test -p render-libass --lib --tests`：16 项单元、5 项真实渲染通过，覆盖文字、输出画布与字体注入。
- `cargo test -p cli --test render`：7 项通过，覆盖真实 BDN/PNG 输出与失败路径。
- 工作区单元/集成测试完整执行 55 个目标；首次唯一失败为 CLI doctor 的旧协议版本断言，其他 54 个目标退出 0。修正该断言后运行 `cargo test -p cli --release --test cli`，16 项全部通过。首次失败与最终重跑日志分别保留。
- 最终 fmt、workspace Clippy、release 构建通过；串行 doctest 20 个目标退出 0，实际 doctest 用例均为 0。发布版 doctor 的 renderer-host 握手成功。

这些结果只升级本次 libass/host/CLI 渲染范围的证据，不改变 VSFilterMod、Strict PGS、GUI、设备矩阵与产品分发许可的未完成状态。

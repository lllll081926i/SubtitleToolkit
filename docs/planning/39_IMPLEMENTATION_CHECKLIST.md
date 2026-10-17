# 实施与验收检查表

本表列正式验收条件，不是实时进度表。空框只表示本文件没有签署验收，不表示代码一定不存在；当前实现和本次验证以 [`../STATUS.md`](../STATUS.md) 为准。历史门阶日志不作为当前证据。

适用阶段见[26](26_DEVELOPMENT_PHASES.md)，任务 ID 见[27](27_WBS_DEPENDENCIES.md)，可执行验收步骤见[25](../specs/25_ACCEPTANCE_TESTS.md)。CSRI/x86、MKV、字体子集、GUI 与 Strict 不是 CLI 基线的共同前置条件。

| 验收主题 | WBS 责任节点 | 验收 ID 与证据要求 |
|---|---|---|
| ASS 保真与字段变换 | F021, T020, T030 | AT-ASS-001/002、AT-CANVAS-001/002；字节白名单与真实视觉语料分开 |
| 时间与探测 | T010, A001 | AT-TIME-001/002、AT-PROBE-001；精确分数、量化边界与真实 ffprobe |
| libass 与宿主 | R001, R010, R011 | AT-RENDER-001；真实 DLL/像素，缺依赖记未覆盖 |
| 可选特效 provider | R012, R020, R030, R031 | AT-RENDER-002/003；实际 provider/x86 产物，不用 Poison 代替 |
| BDN/PGS | B002, P002, P003, P004, V002 | AT-BDN-001、AT-PGS-001–004、AT-BD-001；独立解码与合法 Profile |
| 多输出与缓存 | R040, J003 | AT-MULTI-001/002、AT-FONT-002；分组隔离、计数与身份失效 |
| 输出事务与批处理 | J002, J004, J005 | AT-OUT-001、AT-BATCH-001、AT-PATH-001；真实文件失败路径与集合完整性 |
| 后续界面与容器 | QA002, A002 | AT-UI-001、AT-MKV-001；AT-MKV-001 已用真实 MKVToolNix 执行通过（23/23）；AT-UI-001 的取消链已通并断言，60 秒响应实测仍缺 |

每次签署记录执行人、代码提交、日期、日志、依赖与未覆盖范围。新增文档审查建议的处理状态见[采纳记录](AUDIT_ADOPTION_20261008.md)；“规范已补”不自动勾选下面的行为验收。

## Phase 0–2 退出条件核验（2026-10-16）

按[26 阶段](26_DEVELOPMENT_PHASES.md)逐条核对，证据为本次实际执行，不是历史结果。

| 阶段退出条件 | 状态 | 证据 / 未覆盖项 |
|---|---|---|
| Phase 0：相关 crate 回归通过、有本次日志、渲染依赖不可用记未覆盖 | 满足 | 57 个测试目标 0 失败（`target/gate/final.log`）；无 libass 时按约定 SKIP 并单列 |
| Phase 1：每种用户可见变换有正向与拒绝用例 | 满足 | `cli/tests/cli.rs` 16 项含 `usage_errors_exit_two_and_run_errors_exit_one`；`codec-ass/tests/roundtrip.rs` 字节保真 |
| Phase 1：输入/输出错误不覆盖原文件 | 满足 | `cli/tests/cli.rs` `preserve_is_a_byte_for_byte_no_op_through_the_binary`；`task-engine` 事务测试 |
| Phase 1：批处理部分失败可定位到输入与 profile | 满足 | 本轮新增 `at_batch_001_…`（100 输入 × 3 profile，混入 2 个坏输入，逐 profile 定位） |
| Phase 2：可用 native DLL 时运行真渲染集成测试 | 满足 | 设 `STK_LIBASS_DLL` 后 `renderer-host`、`render-libass`、`cli --test render`、`--test batch` 均真跑 |
| Phase 2：无 DLL 时报告跳过，不冒充真渲染证据 | 满足 | 各测试缺依赖时打印 SKIP 并 return，`STK_REQUIRE_LIBASS=1` 可把跳过变失败 |

### 本轮新增用例

- `crates/cli/tests/batch.rs::at_batch_001_hundred_inputs_by_three_profiles_partial_failure_is_contained`：补齐 AT-BATCH-001 的容量验收（此前只有 2×2 的小样本）。同时锁定两条真实契约：非 UTF-8 输入报 `failed` + `STK-INP-0112`；默认 rename 不覆盖既有输出。

### 仍未覆盖（依赖外部素材或未实现产物）

下列项**不因本页已核验 Phase 0–2 而变为通过**：

- **AT-PGS-001**：需可合法使用的第三方真盘 SUP。仓库内无此类素材（仅本项目自产流），保持未覆盖。
- **AT-RENDER-002**：需真实 VSFilterMod provider 二进制。当前仅能用宿主故障/超时证明隔离，不能代替 provider 崩溃结论。
- **AT-RENDER-003**：x86 renderer host 产物不存在，不验收自动路由。
- **AT-MKV-001**：已实现并执行通过（2026-10-17，真实 MKVToolNix v102.0，23/23 断言；见[裁决](../reviews/2026-10-17-mkv-container-helper.md)）。未覆盖：第 7 层设备语料、逐包内容 diff。
- **AT-FONT-003（子集渲染等价）**：已执行并通过（2026-10-17）。同一 ASS/canvas/fps 下，整份 Arial 与子集各经 `--inject-fonts` 注入真 libass 渲染，**2 帧 PNG 逐像素一致**（1,045,720 → 29,240 字节）。入口 `crates/cli/tests/subset_render.rs`。未覆盖：非拉丁文字、合字序列、TTC/OTC、variable font（当前一律回退整份）。见[子集裁决](../reviews/2026-10-17-font-subset.md)。
- **AT-UI-001**：**部分完成**（2026-10-17）。已接通：GUI 把输入×Profile 编译成真 batch spec、交 `cli::batch::run_batch` 执行、逐行回收状态；阻塞 IO 与渲染都在 `spawn_blocking` 里；取消令牌与引擎共享同一份 `Arc`（已断言）。**未完成**：60 秒持续响应的**实测**需真人操作与录屏，当前只有结构证据，不声称通过。见[GUI 接通裁决](../reviews/2026-10-17-gui-backend-wiring.md)。
- **AT-BD-001 的 GUI 部分**：UI 禁用解释留到 GUI 阶段。

## 开工前
- [ ] Rust/VS Build Tools/Windows SDK 固定版本
- [ ] CI Windows runner 可重现
- [ ] third-party license table 建立
- [ ] test corpus 仓库/大文件存储策略建立

## ASS MVP
- [ ] Format 动态列
- [ ] Script Info unknown keys
- [ ] attachments
- [ ] override tokenizer
- [ ] byte/semantic roundtrip corpus
- [ ] 16:9 resolution transform golden
- [ ] HDR transform golden
- [ ] Rational FPS transform

## GUI MVP
- [ ] Iced FileDropped 多文件
- [ ] native open/save dialog
- [ ] Manual-first pages
- [ ] immutable JobSnapshot
- [ ] queue cancel
- [ ] no UI blocking E2E

## Renderer
- [ ] libass version probe
- [ ] font environment
- [ ] dynamic event sampling
- [ ] CSRI x64 proof
- [ ] CSRI x86 proof
- [ ] provider crash isolation
- [ ] pixel diff report

## BDN/PGS
- [ ] PNG alpha
- [ ] bbox preserves full-canvas position
- [ ] BDN TC correct
- [ ] third-party PGS parse
- [ ] Fast Path ODS invariant
- [ ] Basic encoder roundtrip
- [ ] 2160p strict profile blocked

## 多输出
- [ ] profile duplicate/edit
- [ ] shared render cache
- [ ] partial success
- [ ] deterministic filename template

## 发布
- [ ] portable smoke
- [ ] installer smoke
- [ ] uninstall smoke
- [ ] Chinese/emoji/long paths
- [ ] Authenticode signing pipeline
- [ ] SBOM/THIRD_PARTY_NOTICES
- [ ] diagnostic zip redaction


## V3.1 源码审计新增 Gate
- [ ] libass renderer 初始化同时设置 storage_size + frame_size + fonts。
- [ ] ASS 文件通过 Rust 读 bytes 后 `ass_read_memory`，不依赖窄字符串文件路径。
- [ ] OutputCanvas / PlayRes / LayoutRes / StorageSize 数据模型分开。
- [ ] 多输出先按 RenderGroupKey 分组，再在同组内合并采样与渲染；跨组不复用 RGBA。
- [ ] `detect_change` 仅作为 hint，事件合并基于 visible RGBA hash/diff。
- [ ] CSRI provider 与 legacy VSFilterMod adapter 分开 capability probe。
- [ ] PGS Fast Path 有 ODS payload hash invariant。
- [ ] PGS time transform 完成 DTS/decode schedule validator。
- [ ] Basic Encoder 通过大 ODS fragmentation golden test。
- [ ] Strict bitrate validator 与 stream shaping 功能名称分离。
- [ ] font subset 通过 GSUB/GPOS/复杂脚本/variable/TTC corpus，否则自动 full-font fallback。
- [ ] 所有 GPL/AGPL 组件发布边界再次审计并写 THIRD_PARTY_NOTICES。

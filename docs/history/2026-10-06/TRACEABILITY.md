# 规格 → 实现 可追溯矩阵（审查/审批基线）

> 用途：把 `SubtitleToolkit-RS-v3/` 的 40 份规格逐条落到具体 crate/WBS 编号与可执行证据上。
> 状态口径：**未开始 / 进行中 / 已实现待审 / 已通过审查**。"已通过"必须有 `cargo test` 真实输出与审查记录支撑，不接受自述完成。
> 开发顺序硬约束（00 §4）：`格式探测/时间类型 → ASS AST round-trip → SRT → HDR/FPS transforms → CLI golden → Iced shell → libass → multi-output cache → BDN → PGS parser/fast-path → PGS basic encoder → CSRI/VSFilterMod host → strict Blu-ray author/validator`。

## 1. Crate 与 WBS 对照

| WBS | Crate | 覆盖规格 | 状态 | 证据 |
|---|---|---|---|---|
| C001 | core-model（错误/result 模型） | 22 §3-§4 | 已实现待审 | `cargo test -p core-model`；编号身份的机器把关见 `no_two_crates_share_an_error_code_with_different_meanings`（跨 crate 撞码，ISSUES H8），把关器自己的负基线 `the_checker_still_detects_a_deliberate_collision`、`a_trampoline_helper_feeds_its_call_sites_into_the_table`、`comments_and_string_contents_are_not_error_codes`（ISSUES AK12） |
| C002 | core-model（rational/time） | 08 §2、10 §2、ADR-007 | 已实现待审 | time.rs 15 个单测，含 `long_run_has_no_accumulated_drift`（AT-TIME-001 前置） |
| C003 | core-model（canvas/profile） | 09 §1、§4 | 已实现待审 | `sixteen_nine_is_exact`、`strict_pgs_blocks_2160_canvas`（AT-BD-001 前置） |
| C004 | core-model（Plan/JobSnapshot/preset schema） | 07 §4、36 | 部分（Plan JSON round-trip 已有；TOML preset 未做） | `plan_roundtrips_json` |
| F001 | format-detect | 08 §3 | 已通过审查 | 门阶 `tests` + `cli-smoke`；`convert` 的编码/BOM/换行判定被 `--to` 全链使用（§24） |
| F010 | codec-srt | 08 §1、14 | 已通过审查 | 门阶 `tests`（含 fuzz：任意字节不 panic）与 `subs-cross-check`（pysubs2 读写互认，§28） |
| F020/F021 | codec-ass（parser + 字节保真 writer） | 08 §1、24 §2、AT-ASS-001 | 已通过审查 | 门阶 `tests` 的逐字节 golden + `subs-cross-check`；AT-ASS-001 见 §28 |
| F030 | codec-bdn | 13 §7、AT-BDN-001 | 已通过审查 | 门阶 `tests` + `render-real`；AT-BDN-001 由测试自己解析 PNG IHDR 判定（§26） |
| F040/F041 | codec-pgs | 08 §1 PGS IR、13 §2-§3、AT-PGS-001..003 | 已通过审查 | 门阶 `tests`（与 `oxideav-sub-image` 双向差分）+ `strict-badge` + `security-limits`；`pgs encode` 全链见 §25 |
| T010 | transform-time | 10 全文、AT-TIME-001/002 | 已通过审查 | 门阶 `tests`；`time` 命令的三种模式（偏移/换算/两点校准）走 `cli-smoke` |
| T020 | transform-hdr | 11 全文、AT-ASS-002 | 已实现待审 | `cargo test -p transform-hdr` 11 passed；`only_color_lines_change` = AT-ASS-002 |
| T030 | transform-resolution | 09 §2-§4、§9、AT-CANVAS-001/002 | 已实现待审 | 36 单元 + 6 集成；`preserve_is_byte_identical_on_a_dirty_script` = AT-CANVAS-002 前置；缩放判据见 ISSUES §H |
| T040 | transform-text（安全规范化/合并） | 14 Multi-ASS merge | 未开始 | 合并语义以 Assmodhelper 反例为负基线（固定字符偏移禁止） |
| U001-U005 | app（Iced 0.14） | 04、05、35 | 未开始 | 依赖 PoC 已完成（C11/C12 能编译），但整条 GUI 线未动工（ISSUES C7/C8/C9） |
| R001 | render-libass | 12 §1A、09 §3 | 已通过审查 | 门阶 `render-real`：`ass-9.dll`（libass 0.17.5）真渲染出像素并 author 成 PGS/BDN 读回（§25/§26） |
| R010-R031 | render-host-protocol / x64 / x86 / CSRI / VS / AVS | 12 §2-§7、AT-RENDER-001..003 | 部分 | host 协议、x64 host、位数路由已交付（§27）；CSRI / VapourSynth / AviSynth+ provider 与 x86 host 仍未落实（ISSUES C9、AE6） |
| I001/I002/B001 | image-core（bbox/量化/PNG） | 09 §2、21 §5 | 已通过审查 | 门阶 `png-cross-check`（Pillow 双向逐像素）；量化仍由 `codec-pgs` 承担，M10 登记未合层 |
| B002 | codec-bdn writer | 13 §7 | 已通过审查 | `render`（14 §ASS→BDN）端到端交 XML+PNG 目录并按盘上 IHDR 复核（§26、门阶 `render-real`） |
| P001-P011 | pgs-author / validator | 13 §1-§6、31 §7 | author 规划层已落地（Level 3 路径），Advanced Author 未做 | 见 §14：重复视觉事件消除、erase 表达、画布坐标不变、自检；13 §5 的 reuse/buffer/acquisition 明确未实现 |
| J001-J005 | task-engine | 15、37、38 | 进行中：输出事务 + 任务模型已落地，输入发现/并发限制器未做 | 见 §15；`batch` CLI 仍返回 `STK-INP-0060`，没有假成功 |
| A001 | media-probe | 16、AT-PROBE-001 | 已通过审查 | 门阶 `probe-real`：真 ffprobe + 真视频跑尺寸/帧率/色彩/建议四条链（§29）；真发行素材仍缺（AF5、待决策 E3） |
| A002 | container-adapter（`mkv`） | 18、AT-MKV-001 | 未开始 | 需要外部 MKVToolNix；`mkv` 命令现在返回 `STK-INP-0060`，不假成功 |
| A003 | font-service | 17、AT-FONT-001/002 | 已通过审查 | 门阶 `font-scan`：真字体库读表建库、六档评级、缺字清单、注册字体、整份交付与缓存对照（§30-§33）；subset/embed/注入未做（AG2/AG8） |
| — | cli（subtitletool-cli） | 20、36 §5、24 §Golden | 已实现待审 | 36 单元 + 8 端到端（真二进制、退出码、`--json` 单行 schema、golden 字节）；未实现命令返回 `STK-INP-0060` 而非假成功 |

## 2. 验收用例覆盖计划

| 用例 | 归属 | 计划证据形式 |
|---|---|---|
| AT-ASS-001 | F020/F021 | 每个 fixture `parse→to_string` 逐字节相等 + 未知 tag 保留断言 |
| AT-ASS-002 | T020 | 颜色字段变、时间/文本/几何 hash 不变 |
| AT-CANVAS-001/002 | T030 | 1080→2160 关键坐标 ×2；非 16:9 建议被拒 |
| AT-MULTI-001/002 | J003、14 Multi-output | 单输入 4 profile 全 verify；cache metrics 证明只渲染一次 |
| AT-TIME-001/002 | T010 | 3 小时 cue 的 rational 精确缩放；25→50 snap 不改总时长 |
| AT-PGS-001..004 | F040/P002/P003 | 第三方 SUP +1000ms 后 ODS payload hash 不变；encoder 结构自检 |
| AT-BD-001 | C003 + UI | `strict_pgs_blocks_2160_canvas` 已绿，UI 侧待做 |
| AT-BDN-001 | B002/F030 | XML 声明尺寸 == PNG IHDR；FF 字段合法 |
| AT-UI-001 | U001/task-engine | 重任务在 spawn 线程，UI 循环不被阻塞（旧工程 P0-1 的反例测试） |
| AT-PATH-001 | task-engine/output | 中文/emoji/260+ 长路径；Windows UTF-16 API |
| AT-SEC-001 | codec-pgs | 恶意 length/RLE fuzz 不崩溃、不超额分配 |
| AT-OUT-001 | 输出事务 | verify 失败不替换 final |

## 3. 审批（决策）记录入口

架构/许可类决策以规格 32_ADR_DECISIONS.md 为准（ADR-001..012 全部 Accepted）。实施期新增决策记录在 `ISSUES.md` C 段（例如 BDN timecode 单一舍入源的具体选法 C1、PGS 字段结论 C3）。**任何偏离规格的实现必须在 ISSUES.md 留条目并标状态**，否则审查不通过。

## 4. 审查（review）门禁清单

每个 crate 通过标准：
1. `cargo test -p <crate>` 全绿，且测试真的调用生产 API（旧工程反例：测试用桩、主链路 0 覆盖）。
2. UI/选项类功能必须有"接通证据"：参数从入口到产物生效的断言（规格 00 §2.8；旧工程 20 个选项仅 7 个生效）。
3. `cargo clippy --all-targets -D warnings` 与 `cargo fmt --check` 干净。
4. 格式产物（PGS/BDN/PNG）有字节级 validator 自检 + 与参考实现的差分。
5. 错误路径返回 `StkError`（带 `STK-XXX-####`），不 panic、不静默降级。

## 5. V3.1 源码审计门禁（对应 39 §"V3.1 源码审计新增 Gate"）

| Gate | 归属 crate | 状态 | 证据（测试名 / 记录） |
|---|---|---|---|
| libass 初始化同时设置 storage_size + frame_size + fonts | render-api / render-libass | 数据模型已就绪，绑定未做 | `geometry::tests::storage_is_mandatory_before_render`（无 storage 直接拒绝渲染） |
| ASS 走 bytes + `ass_read_memory`，不依赖窄字符串路径 | render-api | 设计层已满足 | `RenderRequest.script: String`（内存内容，非路径）；见 ISSUES F7 |
| OutputCanvas / PlayRes / LayoutRes / StorageSize 分层 | core-model + transform-resolution | 已实现 | `geometry.rs` 9 个测试（含 `layout_keepasis_avoids_double_scale_warning`）+ transform-resolution 的 `layout_res_default_kept_and_only_warned` / `drop_policy_removes_only_layout_lines_and_scales_style_space` |
| 多输出先生成 Render Timeline Planner | render-api | 未开始 | ISSUES F5 |
| `detect_change` 仅作 hint，事件合并基于 visible RGBA hash | render-api / image-core | 未开始 | ISSUES F5（`FrameFingerprint`） |
| CSRI provider 与 legacy adapter 分开 capability probe | render-api | 未开始 | ISSUES F6 |
| PGS Fast Path 有 ODS payload hash invariant | codec-pgs | 通过：`ods_digests` + `ods_invariant`，报告交出 `per_object` 与 `aggregate_sha256`（F4） | `sha2` 早已在树里（D11/AK4），「未开始」是过期快照 |
| PGS time transform 有 DTS/decode schedule validator | codec-pgs / validator | 未开始 | ISSUES F3 |
| Basic Encoder 通过大 ODS fragmentation golden | codec-pgs::encoder | 已实现待审 | `encoder::tests::fragmentation_matches_reference_constants_and_reassembles`（首片 65508 / 续片 65515 由 `MAX_ODS_SEGMENT_PAYLOAD=65519` 推导）+ 差分 `encoder_output_is_readable_by_reference_decoder` |
| Strict bitrate validator 与 shaping 命名分离 | validator | 未开始 | ISSUES F9 |
| font subset 复杂 shaping 等价否则整字体回退 | font-service | 未开始 | ISSUES F10 |
| GPL/AGPL 边界再次审计 + THIRD_PARTY_NOTICES | 发布期 | 未开始 | SupMover=AGPL、ALASS=GPL、VSFilterMod=GPL-3、ass2bdnxml=GPL |

## 6. 当前真实测试计数（本次会话实测，非引用）

`cargo test` 逐 crate：core-model 38、format-detect 36、codec-srt 3+45、transform-time 52、render-api 5、task-engine 11、media-probe 17 = **207 个测试通过，0 失败**；`cargo clippy` 对 core-model/media-probe/task-engine 为 0 warning。codec-ass 由并行子代理实现中，未计入。

## 7. 本轮（V3.1 + PGS 取证）之后的真实状态

`cargo test`（全 workspace）= **352 passed / 0 failed**；`cargo clippy --workspace --all-targets` = 0 warning；`cargo fmt --all --check` = 干净。

| Crate | 测试 | 对应 WBS | 备注 |
|---|---|---|---|
| core-model | 38 | C001-C004 | 含 geometry（V3.1 四层坐标） |
| format-detect | 36 | F001 | PGS 段头已按取证修正，含真布局回归 |
| codec-ass | 75 | F020/F021 | 逐字节 round-trip + override tokenizer + views/loss/colour_map |
| codec-srt | 48 | F010 | 含 4000 例确定性 fuzz |
| transform-time | 52 | T010 | AT-TIME-001/002 对应项 |
| codec-pgs | 56（50 单元 + 6 差分） | F040 的一部分 + P002 | 与 `oxideav-sub-image 0.0.7` 双向差分；G1-G5 全部由此暴露 |
| render-api | 19 | R010 前置 + 09 §6 | Timeline planner、capability probe、PE arch |
| task-engine | 11 | J004/J005 前置 | 输出事务、命名模板、长路径 |
| media-probe | 17 | A001 | ffprobe adapter + Job Object |

仍未开始的只剩：app(Iced GUI)、container-adapter（18 的 `mkv`，要外部 MKVToolNix）、transform-text 合并（T040）。
本行以前列的九项里，codec-bdn / validator / pgs-author / render-libass / renderer-host / cli / font-service 都已交付并被门阶跑过（§24-§33），这是 Phase-0 快照一直没回头改造成的（ISSUES §AJ6）。
阻塞：render-libass（A6 libass crate 需 pkg-config → 倾向手写 C ABI + 预编译 DLL）、CSRI provider（C9 需要 csri.h 与 x64 provider DLL）、ffprobe/mkvtoolnix 二进制（A5 无包管理器，需你允许下载）。

## 8. T030 完成后的实测状态（2026-10-05）

`cargo test`（全 workspace，**不含尚未登记的 `crates/codec-bdn`**）= **417 passed / 0 failed**（31 个测试二进制）；
`cargo clippy --all-targets -- -D warnings` 对 transform-resolution / codec-pgs = 0；`cargo fmt` 已格式化。

| 新增 | 测试 | 关键断言 |
|---|---|---|
| transform-resolution 单元 | 36 | 整数 half-up 取整、`.5` 精确小数、`0→0` 不计改动、Preserve 优先于一切策略 |
| transform-resolution 集成 `tests/rewrite.rs` | 6 | 脏脚本（BOM+混合换行+未知 key+未知 tag+附件+drawing+karaoke）Preserve 逐字节不变；Rewrite 后仍可 round-trip 且同尺寸再写为零改动 |
| codec-pgs `encoder`（Level 3 Basic Encoder） | 11 | PCS/WDS/PDS/ODS/END 结构自解析回读、erase display set、DTS<PTS、>64KB RLE 分片重组、>256 可分辨颜色硬失败 |
| codec-pgs 差分新增 | 1 | 我们 encoder 的每个段都能被 `oxideav` 的 `read_segment` 逐字节读回，其 `decode_rle` 与我们的索引图一致 |

真实来源锁：`third_party/SOURCE_AUDIT_LOCK.json`（libass master `f61db56…`，0.17.5，tarball SHA-256 已记）→ ISSUES F11 由"待解决"转为"部分已解决"。

## 9. CLI（20）接入后的实测状态（2026-10-05）

`cargo test`（全 workspace，仍不含未登记的 `crates/codec-bdn`）= **461 passed / 0 failed**（35 个测试二进制）；
`cargo clippy --workspace --all-targets -- -D warnings` = **0**；`cargo fmt --all --check` = 干净。

新增可交付物：`crates/cli` → `subtitletool-cli.exe`，命令 `probe / convert / time / hdr / pgs adjust / pgs validate / doctor`。

| 证据 | 说明 |
|---|---|
| `tests/cli.rs` 8 个端到端 | 真二进制调用：doctor 的 JSON 是一行且 `schema=1`；ASS→SRT 字节 golden；Rewrite 后 `corpus_required=true`；Preserve 经二进制仍是逐字节相同；dry-run 不落盘；未实现命令 exit 1 + `STK-INP-0060`；usage 错误 exit 2 |
| `args.rs` 15 个单测 | 全局开关三种位置等价、`--flag=value`、无值开关、two-point 用 ASS 时间而非毫秒、`--preset` 明确拒绝（E4） |
| `pipeline.rs` 11 个单测 | BOM 归属、UTF-16 判定与解码（判不动时明确失败）、ASS↔SRT 往返、时间改写只动 Start/End、几何取自文件 PlayRes |
| `run.rs` 剩余断言 | 写出前一律回读校验（ASS 重解析字节相同 / SRT 重写稳定 / SUP 段数一致） |

已知缺口（不要误认为已完成）：`--preset` 的 TOML（E4）、非 UTF-8 输入解码（J10）、GUI"复制 CLI 命令"所需的 `Command` serde（J4）、`render/pgs encode/batch/font/mkv` 依赖后续 Phase。

## 11. codec-bdn 接管 + BDN 时间码定案（2026-10-05）

`cargo test`（全 workspace，含 `crates/codec-bdn`）= **537 passed / 0 failed**（44 个测试二进制）；
`cargo clippy --workspace --all-targets -- -D warnings` = **0**；`cargo fmt --all --check` = 干净。

| 交付 | 内容 | 证据 |
|---|---|---|
| codec-bdn（F030/B002） | BDN XML 流式解析（quick-xml，不解外部实体/不联网）、写出、PNG IHDR 尺寸一致性、路径与文件名安全、排序守卫、尺寸/画布守卫、LossReport | `tests/xml_safety.rs` 21、`png_consistency.rs` 12、`roundtrip.rs` 12、`ordering.rs` 7、`canvas.rs` 10、`paths.rs` 4、`timecode.rs` 10 |
| core-model 修正 | `Time::to_bdn_timecode` / `BdnTimecode::to_time` 改成帧号语义（单一量化点），新增 `from_frame_index/to_frame_index` 与 99:59:59:99 拒绝 | `bdn_timecode_is_a_frame_index_not_wall_clock`、`bdn_timecode_roundtrips_exactly_on_the_frame_grid`（两小时帧网格逐帧往返） |
| 解析器良构性 | 结束标签名字匹配检查；诊断路径保留原样大小写并指向元素自身 | `unclosed_tag_is_error_with_position`、`missing_required_fields_report_which_field_and_where` |
| 来源锁 | `ass2bdnxml` master `6faa51e…` + tarball SHA-256 入库 | `third_party/SOURCE_AUDIT_LOCK.json` |

仍未做（不要当成已完成）：BDN→PGS 的 ActualTime/RepeatCount 语义细化、Style 字体字段与 font-service 对接、`validator` 的 13 §6 分层严格校验、`render-libass`/`renderer-host`、Iced `app`、`pgs encode` 的 RGBA 输入链路。

## 12. validator（13 §6）与 CLI 接线（2026-10-05）

`cargo test` = **563 passed / 0 failed**（46 个测试二进制）；`cargo clippy --workspace --all-targets -- -D warnings` = **0**；fmt 干净。

| 层（13 §6） | 实现 | 证据（测试名） |
|---|---|---|
| 1 结构 | `validator::structure` + 自建永不失败的 `spans()` 分组 | `missing_end_marker_is_reported_with_display_set`、`corrupt_ods_data_length_is_caught` |
| 2 时间线 | `validator::timeline`（DTS/PTS、集内一致、单调、lead 混用、帧网格、负时钟） | `non_monotonic_pts_and_dts_after_pts_are_caught`、`frame_grid_alignment_is_a_warning_not_an_error` |
| 3 生命周期 | `validator::lifecycle`（object/window/palette 定义与引用、redefinition、palette-only set、epoch） | `encoder_output_passes_structure_timeline_and_lifecycle`、`undefined_object_reference_is_fatal_for_strict` |
| 4 Profile | `validator::profile`（画布 profile、多画布、frame-rate 字节、compliant 声明） | `oversized_canvas_is_rejected_for_compliant_claims` |
| 6 码率 | `peak_bitrate` **只测量**；`PGS.BTR.01` 的文案明写不是编码器承诺 | `peak_bitrate_measures_and_only_measures` |
| 5 / 7 未覆盖 | Strict 等级强制输出 `PGS.DEC.01` / `PGS.DEV.01` warning，不假通过 | 同上 oversized 测试 |
| ASS/SRT | `validator::ass_document`（画布/PlayRes/LayoutRes/样式引用/时间/未知 tag/附件/字段表来源）、`time_pairs` | `ass.rs` 11 个测试 |
| CLI | `pgs validate --level ... --peak-bitrate-bps N` 输出 findings + `strict_badge_allowed`；有 error/fatal 退出码 1 | `cli::run::tests::pgs_validate_and_adjust_round_trip`、`tests/cli.rs::pgs_validate_reports_structure_for_streams_we_encoded` |

## 13. image-core（09 §2/§3、13 §7、AT-BDN-001，2026-10-06）

`cargo test` = **582 passed / 0 failed**（50 个测试二进制）；`cargo clippy --workspace --all-targets -- -D warnings` = **0**；fmt 干净。

| 要求 | 实现 | 证据 |
|---|---|---|
| 09 §3 裁剪不改变画布位置 | `Patch{x,y}` 携带完整画布坐标；`place_back/blend_into` 覆盖语义 | `bbox_uses_full_canvas_coordinates`、`crop_then_place_back_is_pixel_identical`（09 §9 逐像素等价）、`alpha_zero_pixels_are_not_copied_as_content`（M3 负基线） |
| 全透明帧不写全透明图 | `Frame::bbox()` 返回 `None` → 作者层必须转 erase | `transparent_frame_has_no_bbox_and_must_become_erase`、`blank_frame_has_no_patch_and_must_not_write_a_full_transparent_png` |
| PNG 必须真压缩（旧工程 stored 块缺陷） | flate2 deflate + 逐行 filter 代价选择（平局取小编号，输出可复现） | `idat_is_really_compressed_not_stored`（<1% / <10% 门槛）、`filter_choice_is_per_row_and_deterministic` |
| AT-BDN-001 声明尺寸 == PNG 尺寸 | `Patch::to_png` 的 IHDR 即实测尺寸；`header()` 只读 IHDR 供 XML 对齐 | `tests/bdn_png.rs::png_sizes_drive_the_xml_and_the_directory_check`（含 `min_axis=4000` 丢弃而非放大、`STK-BDN-0073` 反例、`validate_dir` 真实目录复核） |
| 外部实现参与验证（不接受自证） | `tools/png_pillow_check.py` 双向：我们写→Pillow 读，Pillow 写→我们读（0/2/3/4/6 + 1 位子字节） | `tests/pillow_cross.rs` 实跑输出 `Pillow 12.3.0 / VERIFY OK 4`，反向 7 个参考文件逐像素相等；缺失时打印 SKIP，`STK_REQUIRE_PILLOW=1` 时视为失败 |
| CRC 等格式细节的正确性 | 反射 CRC-32 步进修正（`>>1`），并对**外部**常量负责 | `crc32_matches_official_check_values`（`0xCBF43926` / `IEND` / IHDR 三个值由 zlib 现算核对）。发现过程见 ISSUES M2：这个 bug 只在 Pillow 下暴露，自家 reader 全绿 |
| 23 §健壮性 | 签名/chunk/CRC/IEND/尾部数据检查；解压容量按 IHDR 精确限定并封顶 2^27 | `rejects_corrupt_crc_and_truncation`、`rejects_decompression_bomb`、`rejects_interlace_16bit_and_unknown_color_type` |

## 14. pgs-author（13 §4/§5）与审查口径修正（2026-10-06）

`cargo test` = **589 passed / 0 failed**（53 个测试二进制）；`cargo clippy --workspace --all-targets -- -D warnings` = **0**；fmt 干净。

| 要求 | 实现 | 证据 |
|---|---|---|
| 13 §5 duplicate visual event elimination | `plan()`：SHA-256(bbox 位置 + 局部位图) 指纹，连续同指纹并成一个事件；alpha=0 残影不算内容（09 §3） | `identical_samples_collapse_into_one_event`、`invisible_rgb_changes_do_not_split_events`、`moving_the_same_bitmap_is_a_new_event` |
| 13 §4 全透明帧 → erase display set | 中间/结尾空白成一个 erase；连续空白只留一个；**开头**空白丢弃并记账 | `leading_blank_is_dropped_and_interior_blank_becomes_erase`、`consecutive_blanks_produce_only_one_erase_set`、端到端 `sets[1].is_erase` |
| 13 §4 PCS placement 用完整画布坐标 | `Patch{x,y}` 原样进 `FrameObject.x/y`，author 不改坐标 | `authored_stream_is_valid_and_keeps_canvas_positions` 直接读回 PCS 比对坐标 |
| 输入不可信时的稳定性 | 采样时间必须严格递增、`stream_end` 必填且晚于最后采样（`STK-PGS-0110/0111/0112`） | `plan_rejects_bad_input` |
| 交付前自检 | `author()` 一次给出 `plan/segments/bytes/encode_report/validation/bitrate`，`ensure_valid()` 拦 error/fatal | 同上端到端测试 |
| **审查口径**：Strict 徽章不能被"没跑过的层"骗过去 | `no_blocking_findings()` 与 `strict_badge_allowed(level)` 分离；DEC/DEV 升为 error；`pgs validate` 失败也保留完整 findings（ISSUES N1/N2/N3） | `strict_badge_requires_strict_level_and_no_error_or_fatal`、`cli/tests/cli.rs::pgs_validate_reports_structure_for_streams_we_encoded`（`--level strict` 断言 PGS.DEC.01/DEV.01 出现在 findings 且退出码非零） |

仍未做（不要当成已完成）：13 §5 的 object reuse / palette animation / epoch planning / windows layout / ahead buffering / coded object buffer / acquisition point；真渲染出图（libass 仍在构建）；`pgs encode` CLI 入口仍报 `STK-INP-0060`；Iced `app`；font-service；container-adapter。

## 15. task-engine 任务模型（15、38）与依赖审计门阶（21、28，2026-10-06）

`cargo deny check` 全绿：**bans ok, licenses ok, sources ok, advisories ok**；
`RELEASE=1 bash tools/check_all.sh --strict` → **8 个门阶全部 PASS，fail=0 skip=0**，
nextest `Starting 606 tests across 37 binaries` 全过，release 构建产出 `target/release/subtitletool-cli.exe`。

| 要求 | 实现 | 证据 |
|---|---|---|
| 38 §2 PartialSuccess | `JobOutcome::classify`：取消优先于统计、全成功才是 Success、零成功是 Failed、其余 PartialSuccess；`JobReport` 逐 profile 列 status/detail | `partial_success_keeps_verified_outputs_and_lists_failures`（就是文档里"ASS 成功 / SUP 失败"的字面例子）、`cancel_beats_the_statistics`、`all_verified_is_success_and_none_verified_is_failed` |
| 15 §4 缓存 key 分量齐全 + 无绑定歧义 | 每字段带名字与长度前缀后进 SHA-256；renderer 身份要求真实二进制哈希 | `key_changes_when_any_component_changes`（输入/版本/二进制/画布/fps 各自改变都换 key）、`compute_is_reproducible`、`rejects_missing_or_uppercase_binary_hash` |
| 38 §6 外部版本变化失效 | `JobState::reuse_for` 先比工具链版本快照 | `toolchain_change_invalidates_even_if_hashes_match` |
| 15 §7 只复用验证过的产物 | 恢复判定要求调用方真读文件算哈希；缓存 key 或哈希不一致即拒绝；`.jobstate.json` 读写走 38 §1 原子事务并回读再解析 | `reuse_requires_matching_hash_and_cache_key`、`write_and_read_use_the_atomic_transaction_path`、`re_record_replaces_the_previous_attempt`、`json_roundtrip_rejects_unknown_fields_and_bad_hashes` |
| 38 §3 renderer 崩溃 | `CrashBudget` per-profile，最多一次；有风险时一次都不重启 | `crash_budget_restarts_once_then_stops_only_that_profile` |
| 38 §7 协作取消 | `CancelToken`（Arc<AtomicBool>），writer 在 atomic unit 边界 `check()` | `cancel_token_is_cooperative_and_shared` |
| 01 §任务凭据 | 任务必须有主输入 sha256；profile 不能重名；报告长度与 profile 一致 | `job_validation_catches_ambiguous_profiles`、`report_refuses_length_mismatch` |
| 21 §1 许可政策落地 | `deny.toml`（schema 由本机 `cargo deny init` 取证）：宽松许可白名单 + GPL 组件按 crate 点名 + 只允许 rsproxy/crates-io 源 + `wildcards = "deny"` | `cargo deny check` 四项全 ok；path 依赖补 `version = "0.1.0"` 让 wildcard 检查只对真实外部依赖生效（ISSUES P2） |

仍未做（不要当成已完成）：15 §2 输入发现（递归/glob/IDX+SUB 配对，等 E6 决策）、15 §3 并发分池与
外部进程 limiter、15 §8 的 HTML 报告、37 的容量估算接线、`batch` CLI 入口。

## 16. 模糊冒烟门阶（28 §7、23 §健壮性，2026-10-06）

`RELEASE=1 bash tools/check_all.sh --strict` = **9/9 PASS，fail=0 skip=0**；
`cargo test --workspace` = **610 passed / 0 failed**；clippy `-D warnings` = 0；fmt 干净；
`cargo deny check` 四项 ok。

| 契约 | 实现 | 证据 |
|---|---|---|
| 任意字节都不许 panic | 确定性 LCG 变异器（bit 翻 / 截断 / NUL / 行尾 / 半截 UTF-8 / 垃圾块 / 复制段 / 空文件 / 换行），4 个解析入口各 600 次 | `parsers_never_panic_on_mutated_bytes` |
| probe 必须能自证判定 | 断言 `format`/`confidence`/`reasons` 齐全，`unknown` 只能是 Low 置信 | `cli_probe_entry_never_panics_and_always_explains_itself` |
| convert 看不懂必须失败且不留半成品 | 200 个变异输入：`failed=95 succeeded=105`，两个计数都必须 >0；失败分支断言输出文件不存在（38 §1） | `cli_convert_entry_fails_cleanly_on_garbage_and_never_writes_a_bad_file` |
| 输出可复现（golden / 命令重放的前提） | ASS render 幂等、SRT 往返字节稳定、已解析 SUP 重写与原字节全等 | `clean_inputs_are_idempotent_through_their_own_render` |
| 门阶要能抓自己的错 | 一次"只跑测试没跑 lint"的编辑被门阶抓到（3 门 FAIL），修完重新全绿 | `target/gate-run-final.log`（FAIL 那次）与 `target/gate-run-final2.log`（全 PASS） |

## 17. render-libass 第一层（12 §1A、09 §3，2026-10-06）

`RELEASE=1 bash tools/check_all.sh --strict` = **9/9 PASS**；`cargo test --workspace` = **623 passed / 0 failed（56 个测试二进制）**。

| 契约 | 取证 | 证据 |
|---|---|---|
| ASS_Image 语义（顺序 / 零尺寸 / stride / 保证长度 / RGBA color） | `libass/ass.h:53-86` 原文（libass 0.17.5；`ASS_Image` 在 `ass.h` 不在 `ass_types.h`），来自已入库的 libass-master.tar.gz | `order_matters_shadow_under_glyph_vs_above`、`zero_sized_bitmap_is_skipped_not_error`、`last_row_may_be_short_but_stride_smaller_than_width_is_error`、`color_unpack_is_rgba_with_inverted_alpha` |
| 结构镜像与 C 布局一致 | x64 SysV/MSVC 对齐规则 | `mirrored_layout_matches_the_c_header`（逐字段偏移 + size=56） |
| 损坏输入不失控 | 自环链表、越界 dst | `chain_walk_is_ordered_terminates_and_bounds_cycles`、`negative_dest_and_overrun_are_clipped_not_wrapped` |
| 量化必须可见 | 09 §9 / 08 §4 LossReport | `quantization_to_zero_is_reported_not_silent`、`color_alpha_multiplies_coverage` |
| 不按文件名认能力 | 12 §探测 | `capability_is_decided_by_symbols_not_by_the_file_name`、`missing_library_is_a_reported_error_not_a_panic` |

本轮之后（2026-10-06 晚）：native 依赖已落地（libass 0.17.5，见 §21），`ass_render_frame` 封装、
字体提供（`ass_set_fonts` + AUTODETECT）、`ASS_Library` 生命周期都已完成并由真实渲染测试覆盖；
仍未做的是像素 diff 语料与 12 §2-§7 的 renderer-host 进程隔离。

## 18. 25 §验收用例逐条状态（2026-10-06）

| AT | 状态 | 落点 / 阻塞原因 |
|---|---|---|
| AT-ASS-001 | 通过 | `codec-ass`：parse→render 语义等价、未知 tag 保留（`unknown_tags` 相关测试） |
| AT-ASS-002 | 通过 | `transform-hdr`：`only_color_lines_change` |
| AT-CANVAS-001 | 通过 | `transform-resolution` Rewrite：关键定位 ×2、Preserve 逐字节不变 |
| AT-CANVAS-002 | 通过 | `preserve_is_byte_identical_on_a_dirty_script` + 校验器 `ASS.GEO.*` |
| AT-TIME-001 | 通过 | `transform-time` 有理数速度换算 + 长片无浮点漂移测试 |
| AT-TIME-002 | 通过 | frame snap 保持总时长 |
| AT-PGS-001 | 通过 | Fast Path：`offset_shifts_both_clocks_and_keeps_ods_identical`（+时间后 ODS 字节不变），`ods_digests`/`ods_invariant` 可复核；逐对象哈希与折叠摘要由 `ods_digests_are_sha256_of_the_payload_bytes`（外部锚点）与 `timing_rewrites_keep_the_ods_digests_identical` 钉住，CLI 报告侧由 `pgs_validate_and_adjust_round_trip` 钉住（F4） |
| AT-PGS-002 | 通过 | `forced_flag_only_touches_pcs`：只改 PCS 标志位，不动任何 ODS/PDS |
| AT-PGS-003 | 通过 | `palette_luma_leaves_ods_and_alpha_alone`：调亮度只重写 PDS，alpha 与 ODS 保持 |
| AT-PGS-004 | 通过 | Basic Encoder 段长/RLE/object id/palette 不变量 |
| AT-BDN-001 | 通过 | `image-core/tests/bdn_png.rs` + `pillow_cross.rs`（PNG 尺寸与真实目录复核） |
| AT-OUT-001 | 通过 | `task-engine`：`verify_failure_keeps_old_file_intact`（先 tmp → fsync → 重开校验 → rename；失败时 final 不动、**并点名留在盘上的那个诊断文件**，D13） |
| AT-PATH-001 | 通过 | `long_path_chinese_and_emoji`（中文/emoji/260+ 路径）；CLI 读文件也走同一批路径 API |
| AT-SEC-001 | 通过 | `codec-pgs/tests/security.rs`：`huge_declared_payload_does_not_allocate_it`、`tiny_payload_claiming_a_giant_canvas_is_rejected`、`rle_run_that_never_terminates_is_stopped`、`random_bytes_never_panic_and_stay_bounded` —— 用计数分配器实测分配字节，新增 `security-limits` 门阶 |
| AT-BD-001 | 部分 | 校验器侧已拦（`oversized_canvas_is_rejected_for_compliant_claims`）；UI 解释等 `app` |
| AT-MULTI-001 | 通过 | 25 §AT 的矩阵实测：单 ASS 一次出 1080 ASS + 2160 ASS + 1080 SUP + SRT（外加一份 1080 BDN），五个 profile 全部 `verified` —— `batch_shares_one_render_between_sup_and_bdn_profiles` |
| AT-MULTI-002 | 通过 | "BDN+SUP 相同 render profile 只渲染一次（cache metrics 证明）"：报告里的 `render_cache` 给 `executions=2 / renders=1 / keys=1`；反向控制是 `fps 25` 与 `fps 30` 两个 bdn 必须 `renders=2 keys=2` 且两份 XML 的 FrameRate 不同（`batch_does_not_share_renders_across_different_frame_rates`，ISSUES AC6 已关闭） |
| AT-PROBE-001 | 通过（合成语料）| 真 ffprobe + 真视频实测尺寸/帧率/色彩/建议四条链，见 §29；真发行素材（VFR/telecine）仍未覆盖，ISSUES AF5 |
| AT-RENDER-001/002/003 | 通过（host 隔离本身），残余见 §27 | 这三条考的是 **renderer-host 进程隔离**：崩溃只终止 host、可以重试、位数不符自动路由。`crates/renderer-host` 已交付并被门阶 `renderer-host` 跑（证据见 §27）。残余：大帧走 file mapping（AE4）、minidump（AE5）、`renderer-host-x86.exe` 端到端（AE6）、真 VSFilterMod 崩溃（AE7）；GUI 侧的重试入口要等 04/05 的界面 |
| AT-FONT-001/002 | 通过（门阶 `font-scan`，真字体库 + 真注册表 + 真交付） | 缺字报告与判定见 §30-§33；缓存 key 已含 font environment 分量。剩下的边界：注入未做（AG8/AI4），DirectWrite 权威未接（AG1） |
| AT-MKV-001 | 未开始 | 需要外部 mkvtoolnix（18 §、21 §9） |
| AT-BATCH-001 | 通过 | `task-engine::discover`（15 §2 全套，无 globset 依赖）+ `runner`（15 §3 分阶段额度、38 §3 崩溃预算）+ `cli batch`（§1/§5/§6/§7/§8）。证据：`batch_produces_every_profile_and_a_report_that_reopens`、`batch_partial_success_keeps_the_good_outputs_and_names_the_bad_one`、`two_inputs_with_the_same_profile_do_not_overwrite_each_other` |
| AT-UI-001 | 未开始 | Iced `app` 未开始（04/05/35 §） |

## 19. pysubs2 语义交叉验证（AT-ASS-001，2026-10-06）

`RELEASE=1 bash tools/check_all.sh --strict` = **14 条门阶全 PASS，fail=0 skip=0**；
`cargo test --workspace` = **633 passed / 0 failed（59 个测试二进制）**；clippy 0；fmt 干净。

| 契约 | 实现 | 证据 |
|---|---|---|
| 第三方实现能读懂我们渲染的 ASS，且语义一致 | `tools/subs_pysubs2_check.py verify`：pysubs2 1.8.1 读 `ours_a.ass`，比较 playres/sbs/title/样式字段/每条事件的时间、layer、style、name、三边距、effect、text | `pysubs2_reads_our_ass_and_we_read_pysubs2_ass` |
| 我们读回参考实现写的 ASS/SRT | `emit` 由 pysubs2 产出 `ref_rich.ass`/`ref_plain.ass`/`ref.srt` + 语义 sidecar | 同上测试的第二半 + `srt_semantics_match_pysubs2_too` |
| 表示差异 ≠ 语义差异 | `num()` 规范化 float（pysubs2 把 Fontsize 60 读成 60.0）；SRT 期望取自文件文本（pysubs2 会把 `<i>` 转成 `{\i1}`） | ISSUES U2/U3 |
| 缺依赖不假装通过 | `STK_REQUIRE_PYSUBS2=1` + 门阶 `subs-cross-check`；`--strict` 下 SKIP 即 FAIL | `docs/history/2026-10-06/REVIEW_GATES.md` |

## 20. 渲染位图 → PGS 的整链路（12 §1A、09 §3、13 §4，2026-10-06）

`RELEASE=1 bash tools/check_all.sh --strict` = 全部门阶 PASS（fail=0 skip=0）；
`cargo test --workspace` = **633 passed / 0 failed（59 个测试二进制）**；clippy 0；fmt 干净。

| 契约 | 实现 | 证据 |
|---|---|---|
| 合成 → 裁剪 → author → 段流 → 读回，整链一致 | `pgs-author/tests/pipeline.rs`（合成 provider 产出 `ImageDesc`，不走 libass） | `render_output_flows_into_a_valid_pgs_stream`（3 个显示集、erase 在末尾、PTS=90k/270k、PCS 画布 1280x720、对象坐标 300/600、ODS 尺寸 121x13） |
| 重复渲染不膨胀输出 | 同内容 50 帧塌成 2 个显示集 | `repeated_render_output_does_not_bloat_the_stream` |
| 半透明像素是内容（21 §5 保 alpha） | bbox 与 alpha 分布精确断言 | `semi_transparent_pixels_survive_crop_and_bbox` |

## 21. 第一次真实 libass 渲染与它的三处暴露（12 §1A、13 §4/§5、23 §外部进程，2026-10-06）

`RELEASE=1 bash tools/check_all.sh --strict` = **16 门阶 PASS（fail=0 skip=0，退出码 0）**；
`cargo nextest run --workspace` = **649 tests run: 649 passed（1 leaky，见 ISSUES Z6）**；clippy `-D warnings` = 0；fmt 干净。（§22 的 CLI 接线之后是 654，以最后一条记录为准。）
门阶 `render-real` 现在真的调 `ass-9.dll`（libass 0.17.5，ABI 与产物哈希锁在 `third_party/SOURCE_AUDIT_LOCK.json` 的 `native_build`）。

| 契约 | 取证 / 现象 | 证据 |
|---|---|---|
| `ASS_Image.color` 最低字节是透明度（0=不透明，0xFF=全透明） | `libass/ass.h:71` + `ass_parse.h:26-29` + `ass_render.c:3411-3417`（"removing fully transparent bitmaps"）+ 官方消费者 `compare/compare.c:48-53`（`mul = 129 * (255 - a)`）。真渲染一开始画出 0 个非透明像素才暴露 | `color_unpack_is_rgba_with_inverted_alpha`、`fully_transparent_color_alpha_draws_nothing`、`libass_renders_text_into_rgba` |
| 打包方与拆包方不能各自自洽地错 | pipeline 夹具与 `render-libass` 都由我写，两边都按直通 alpha 错过一次 | `color_packing_agrees_with_the_renderer_that_reads_libass`（用 `unpack_color` 反查 `ass_color`） |
| 真实渲染的像素必须能一路走到可读回的 PGS 段流 | 抗锯齿一帧的可分辨颜色 > 255，`STK-PGS-0095` 拦下（ISSUES Z3） | `real_libass_output_flows_into_a_valid_pgs_stream`（MedianCut 档 + 断言失真进了 LossReport） |
| 量化在 `(Y,Cr,Cb,T)` 域做，且 `T` 参与切分（保 alpha 阶梯） | 13 §5 image quantization；13 §4 透明槽位约定 | `median_cut_stays_inside_budget_and_is_deterministic`、`alpha_ramp_survives_as_a_ramp_not_a_binary_choice`、`exact_policy_fails_loudly_and_median_cut_fits_the_same_frame`、`median_cut_is_the_identity_when_the_source_already_fits`；**真渲染像素上的 corpus 版**由 `real_pixels_keep_their_alpha_ramp_through_quantization` 钉住（M10） |
| 我们写的 .sup 必须能被**别人**的解析器读出来（13 §2 容器层；自家 writer + 自家 reader 只证明自洽） | 真 ffprobe 的 `sup` demuxer 读我们的产物：段数逐段相等、每段 PTS 相等、90000 ticks 读成 `pts_time=1.000…`；截断一半的文件不许再报出同一串（反向对照，证明这条比较能变红）。真 CLI 产物 49 558 字节：我们数 40 段、ffprobe 报 40 个 packet，`ffmpeg -f sup … -c copy` 退出码 0（ISSUES G4） | `ffprobe_reads_our_pgs_container_with_the_same_clocks`
| Basic 档不许静默丢色；Advanced 档的失真必须可见 | `Exact` 超预算报错；`MedianCut` 度量 `distinct/used/merged/worst/mean_sq` 并冒泡到 `LossReport.approximated` | `exact_policy_refuses_the_256th_distinguishable_color`、`zero_max_colors_is_a_value_error_not_a_panic` |
| DLL 路径既不能靠正斜杠猜，也不能靠相对路径猜 | `LoadLibraryExW` 对 `C:/...` 失败、对 `../...`（测试二进制 cwd 是 crate 目录）失败 | `forward_slash_paths_are_normalized_for_the_loader`、门阶 `render-real` 用 `readlink -f` 传绝对路径 |
| 加载失败必须能分诊 | 原先固定写"native 依赖未落地"，把三种病因混成一句 | `missing_library_is_a_reported_error_not_a_panic`（断言细节里带 Win32 错误码） |
| SKIP 不能冒充通过，且只允许一份实现 | 第二个渲染测试的 SKIP 分支漏了 `STK_REQUIRE_LIBASS`，门阶因此"没跑也 PASS" | `render_real.rs` 的 `need_libass!` 宏（两个测试共用） |
| 外部进程约束失败要报告，不要静默退化 | nextest 把测试二进制放进外层 Job → `AssignProcessToJobObject` 失败 → 孙进程不受约束（LEAK） | `timeout_kills_child` 现在区分 `STK-DEP-0021`（已约束）与 `STK-DEP-0023`（未约束，带错误码）；剩余根治项见 ISSUES Z6 |
| 证据文件必须属于本次运行 | `summary.txt` 追加了十几轮历史；`tests.log` 在 nextest 分支根本不写 | 脚本启动清空 summary；nextest 输出重定向进 `tests.log` |

## 22. CLI 打通 ASS→SUP（20 §Commands、14 §ASS→SUP，2026-10-06）

`cargo nextest run --workspace` = **654 tests run: 654 passed（1 leaky，ISSUES Z6）**；clippy `-D warnings` = 0；fmt 干净；
`RELEASE=1 bash tools/check_all.sh --strict` = 16 门阶 PASS（fail=0 skip=0）。

| 契约 | 实现 | 证据 |
|---|---|---|
| `pgs encode` 不再是占位命令 | `cli::run::pgs_encode`：ASS → 整数毫秒采样网格 → libass 逐帧渲染 → `pgs_author::author` → 回读 → 原子写出 | `pgs_encode_writes_a_sup_that_the_same_cli_validates_clean`（写出 7210 字节、2 显示集、1 erase、validate 回读 `no_blocking_findings=true`） |
| 缺渲染器要能自证缺什么，且不落半成品 | `libass_dll()` 只认显式 `--libass-dll` 与 `STK_LIBASS_DLL`，不做磁盘搜索（23 §安全） | `pgs_encode_without_a_renderer_reports_the_missing_input_and_writes_nothing`（`STK-INP-0141` + 两个补法 + 目录里没有 `.sup`） |
| `--dry-run` 不碰盘 | 复用同一套 `pipeline::emit` 事务 | `pgs_encode_dry_run_writes_nothing_even_with_a_renderer`（`state="would-write"`，文件不存在） |
| 失真必须一路到信封 | 渲染层 loss + author 层 loss（含量化合并）合并后才交付 | `merge_cannot_turn_a_lossy_report_into_a_lossless_one`、e2e 里断言 `quantized_frames>0` 时 `loss.approximated` 必须含"量化" |
| 采样网格的量化是事实，要写出来 | 分数帧率下帧号→ms 半进位；空白帧与 `stream_end` 是两个时刻 | `pgs_encode_defaults_are_the_bluray_profile_and_quantized_colors` + ISSUES AA3/AA5 |
| composition canvas 必须是完整 16:9 profile | 参数阶段拒绝非 16:9（13 §4） | 同上测试：`--canvas 1000x800` 直接报错 |
| 帮助文本与实现一致（可反向验证） | usage 按"已实现/尚未实现"分节，测试分段断言 | `usage_errors_exit_two_and_run_errors_exit_one`（已实现命令不得出现在"尚未实现"里） |

## 23. 批量执行器：输入发现、并发与崩溃预算（15 §2/§3/§4、38 §3，2026-10-06）

`cargo nextest run --workspace` = **670 tests run: 670 passed（1 leaky，ISSUES Z6）**，
`concurrency-soak` 门阶把 task-engine 重复跑 12 轮仍全绿；clippy `-D warnings` = 0；fmt 干净。

| 契约 | 实现 | 证据 |
|---|---|---|
| 目录/递归/include-exclude/配对/同名视频发现（15 §2） | `task-engine/src/discover.rs`，**不依赖未批准的 `globset`**（E6）：手写 `*`/`?` 匹配，迭代回溯 | `wildcard_matcher_is_exact_about_star_question_and_case`、`discovery_is_recursive_sorted_and_deduplicated`、`non_recursive_skips_nested_directories`、`include_and_exclude_have_the_documented_precedence` |
| 发现结果必须确定（golden 与缓存 key 的前提） | `BTreeSet` 收集 + 排序，两次发现逐字节相等 | `discovery_is_recursive_sorted_and_deduplicated` 的第二段断言 |
| 读不动 / 配不上对 / 空模式 都要能说清 | 不静默跳过：进 `skipped` 并带原因；空模式与不存在的根直接报错 | `idx_without_a_partner_is_recorded_not_dropped`、`missing_root_is_an_error_not_a_silently_empty_result`、`empty_pattern_is_refused_because_it_would_swallow_the_tree` |
| 每阶段独立并发额度（15 §3） | `Pools` + 六个类别各自的计数信号量；`quant`/`author` 复用 cpu 额度而不是各算一份 | `zero_permits_are_refused_because_they_hang_instead_of_queueing`、`render_semaphore_holds_the_concurrency_down`（峰值 == 1 且 确实排过队） |
| 崩溃必须进预算，预算耗尽要停手（38 §3） | `run_profiles` 里 `catch_unwind` → `CrashBudget::on_crash()`（`Mutex` 传入，崩溃当场消费）→ `StopProfile` 立取消标志 | `a_panicking_profile_becomes_a_crash_not_a_silent_hang_or_fake_success`、`exhausted_crash_budget_stops_the_rest_of_the_job`、`cancelled_job_does_not_start_more_work` |
| 报告顺序与调度无关（15 §8） | 结果按 profile 下标落位，输出按名字排序；未开工的落成 `not-started` 取消 | `results_are_ordered_by_profile_name_not_by_completion_order` |
| 多输出共享渲染（15 §4，AT-MULTI-002 的结构化版本） | 只有一个机制：CLI 侧 `front::RenderMemo` 用同一批分量（输入哈希 / 画布 / 帧率 / 字体 / renderer 哈希）在运行时只渲一次，并交出 executions/renders/keys 三个可数的数 | `batch_shares_one_render_between_sup_and_bdn_profiles`（正向：两个渲染型 profile 只渲一次）与 `batch_does_not_share_renders_across_different_frame_rates`（反向：分量不同就必须各渲一次），门阶 `render-real` 再跑一遍 share 过滤 |

仍未做：`batch` 命令的 CLI 接线（15 §1 的 spec 解析 + 每 profile 的输出路径参数化），见 ISSUES AB4。

## 24. `batch`：一个输入集合 × 多个输出 profile（15 §1/§5/§6/§7/§8、14 §Multi-output，2026-10-06）

`cargo nextest run --workspace` = **679 tests run: 679 passed（1 leaky，ISSUES Z6）**；
`RELEASE=1 bash tools/check_all.sh --strict` = **17 门阶 PASS（fail=0 skip=0）**。

| 契约 | 实现 | 证据 |
|---|---|---|
| 每个 profile 走单命令同一条路（20 §Repro 可重放） | `batch::command_for` 造 `Command::Convert`/`Command::PgsEncode` 后调 `run::execute` | `batch_produces_every_profile_and_a_report_that_reopens`（2 输入 × 2 profile 全落盘 + 报告 + `.jobstate.json`） |
| 命名模板与覆盖策略（15 §5/§6） | `NameTemplate` + `Invocation::overwrite` 一路传到 `pipeline::emit` | `batch_skip_policy_never_touches_an_existing_output_and_says_so`（原文件字节不变 + 报告解释原因）、`batch_reuses_only_artifacts_that_still_hash_right` |
| 恢复只认 `(input, profile)` + 哈希 + 分量（15 §7、38 §6） | `StageRecord.input` 新字段，`record`/`reuse_for` 双键匹配 | `two_inputs_with_the_same_profile_do_not_overwrite_each_other`、`batch_reuses_only_artifacts_that_still_hash_right`（篡改文件后必须重做） |
| 一个 output 失败不回滚别的（14 §Multi-output、38 §2） | `judge()` 定级 + `ok=false` 退 1，报告逐个说明 | `batch_partial_success_keeps_the_good_outputs_and_names_the_bad_one` |
| dry-run 什么都不写、也不写恢复状态 | `WouldWrite` 变体 + 跳过 `.jobstate.json` | `batch_dry_run_writes_nothing_and_no_jobstate` |
| 没有渲染器也能跑文本批量；有渲染就必须交代 renderer 身份 | `renderer` 只在存在 pgs profile 时取（DLL 的 sha256 + `ass_library_version`） | `batch_text_only_job_does_not_need_libass`、`batch_with_render_profile_reports_the_renderer_identity`（哈希与真 DLL 逐一比对，版本 `0x01705000`） |
| spec 写错不许静默忽略 | `deny_unknown_fields` + 一串 STK-INP-015x 码 | `batch_rejects_bad_specs_before_touching_anything`（含"位置参数=用法错误退 2"） |
| doctor 的功能面陈述必须可反查 | `args::not_built_commands()` 从解析器推导，不手抄 | `doctor_json_is_one_line_and_names_missing_capabilities` 的双向断言 + 门阶 `cli-smoke` |

仍未做：同输入多渲染 profile 的"只渲染一次"接入（ISSUES AC6）。

## 25. `render`：ASS→BDN 交付与批量里的目录型产物（14 §ASS→BDN、14 §Multi-output、13 §7，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 14 §ASS→BDN 全链：Renderer → 变化检测 → tight bbox → PNG → BDN writer → XML+PNG validator | `cli::render::build`：真 libass 采样 → `pgs_author::plan`（与 SUP 共用同一份变化检测）→ `patch.to_png` → `build_document_on_canvas` → `write_bdn_xml` → `validate_dir` | `render_writes_a_bdn_dir_whose_declared_sizes_are_the_png_sizes`（2 个 Dialogue → 恰好 2 个事件，InTC/OutTC 落在 25fps 帧网格上：`00:00:01:00`/`00:00:02:00`/`00:00:03:00`/`00:00:04:00`） |
| AT-BDN-001：XML 声明尺寸 == PNG 实际尺寸 | 写前自校 IHDR（`png_size`），写后 `validate_dir` 读盘复核；**测试不依赖 codec-bdn**，自己按字节解析 PNG IHDR 与 `<Width>` | 同上（`ihdr_size` 是测试里的独立实现，避免"自家 writer + 自家 reader 自洽地错"） |
| 13 §7：`min_axis` 只过滤小对象，绝不钳制尺寸 | `codec_bdn::BdnOptions.min_axis` 传给构造器；`assemble` 两遍构造保证"存活事件连续编号、无孤儿 PNG"，规则仍只有一份 | `min_axis_filtering_leaves_no_orphan_png_and_no_numbering_gaps`、`without_filtering_the_first_pass_is_the_answer`、`min_axis_that_filters_everything_reports_an_empty_bdn_instead_of_fake_success`（全过滤时 `loss.user_attention` 明说 BDN 是空的） |
| 输出事务：失败不留半成品、XML 晚于 PNG | PNG 先写、XML 后写（XML 引用 PNG 文件名，反序会留下指向不存在文件的 XML）；无渲染器时 `STK-INP-0141` 且不建目录 | `render_without_a_renderer_fails_with_the_missing_input_and_writes_nothing`、门阶 `cli-smoke` 的"失败的 render 不许留下半成品目录" |
| 20 §契约：全局开关与退出码 | `--dry-run` 全部 `would-write` 且不建目录；`--overwrite skip` 保住原文件；非 16:9 画布是用法错误退 2（stderr 文本、无信封） | `render_dry_run_writes_nothing_at_all`、`render_skips_an_existing_target_when_told_to`、`render_rejects_a_non_widescreen_canvas_as_a_usage_error` |
| 14 §Multi-output：bdn 作为批量 profile 是**一组文件** | 每输入一层目录 `{out}/{profile}/{stem}/{文件名}`（两个输入的 PNG 都从 `0000.png` 开始，平铺必然互相覆盖）；复用判据换成 `dir_fingerprint`（目录内"文件名 + 内容哈希"，排除 `.tmp`）；`min_axis` 进 `job_cache_key` | `batch_bdn_profile_is_a_directory_artifact_whole_directory_must_match_to_reuse`：第二次 `from-cache` → 删一个 PNG 后必须 `verified` 且 PNG 回来 → 换 `min_axis` 的 spec 不得 `from-cache` |
| 26 §Phase / 03 §功能矩阵：`render` 不再是占位 | `Command::Render` + `USAGE` + `not_built_commands()` 推导（font/mkv 仍在清单内） | `unimplemented_commands_are_not_silently_ok`（双向：清单来自解析器，且 `render`/`pgs encode`/`batch` 不得出现在里面）、门阶 `cli-smoke` 的 doctor 检查 |

仍在 ISSUES 里没关的：`pgs encode` / `render` 目前仍在主进程内直连 libass —— 搬进 host 要先有 file mapping 传大帧（AE4，P1），
`doctor` 已经能如实报告 host 可用性与孙进程约束状态（§27）；
AD9 那套重叠的第二机制已删除（一个语义只留一个调用点），不再挂在"没关"清单里。

### 25.1 前半链只跑一次（14 §ASS→SUP「若同时输出 BDN+SUP，共享 render/cache」，本轮关闭 AC6）

| 契约 | 实现 | 证据 |
|---|---|---|
| 两条流水线共用同一份"采样 + 变化检测" | `crates/cli/src/front.rs`：`render_to_plan` 产出 `pgs_author::Plan`；`pgs encode` 走 `author_from_plan`，`render` 直接消费 `plan.events` 的补丁 | `batch_shares_one_render_between_sup_and_bdn_profiles`（`executions=2 renders=1 keys=1`） |
| 共享 key 只覆盖影响渲染的分量 | `render_cache_key` = 输入内容哈希 + 画布 + 帧率 + 默认字体 + renderer 二进制哈希；`min_axis`/`exact_colors`/`level` 故意不进（它们是渲染之后的事） | `batch_does_not_share_renders_across_different_frame_rates`（不同 fps 必须 `renders=2`，且两份 XML 的 `<FrameRate>` 不同） |
| 共享不等于少做输出 | 每个 profile 仍各自编码、各自 verify | 同一测试里 5 个 profile 全 `verified`（AT-MULTI-001 的矩阵） |
| 报告要能自证共享 | `data.report.render_cache = {executions, renders, keys}` 三个数一起给，谁都能自己算 | 上面两条测试都是读这三个数断言的 |

## 27. renderer-host 进程隔离（12 §2-§3、23 §外部进程、28 §2，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| AT-RENDER-001：libass 在 host 里跑，且**崩-free**——出的像素必须与进程内一致 | `crates/renderer-host`：`client::Host::start` 起子进程 → 握手 → `LoadScript` → `Render`；host 侧才 `LoadLibrary`（`render_libass::Provider::open`） | `at_render_001_the_host_hands_back_the_same_pixels_as_an_in_process_render`（同一时刻、同一画布，host 交回的 RGBA 与进程内渲染 **逐字节相等**；握手逐项核 `provider/api_version/pixel_format/arch/sha256`） |
| AT-RENDER-002：provider 崩溃只终止 host，并且**可以重试** | 协议里的 `Poison{abort\|exit\|panic\|hang}` 自伤开关（本机没有真 VSFilterMod 二进制，用注入代替，见 ISSUES AE7）；崩溃取证 = 退出码 + host 的 stderr 尾部 + `contained`/`job_error` | `at_render_002_a_crashing_provider_only_kills_the_host_and_a_retry_works`（崩前能出帧 → `Crashed{exit_code}` → 新 host 重试出帧成功；测试继续跑本身就是"主进程没死"的证据） |
| 显示集时刻要能被判定为帧边界（09 §6、13 §2；严格兼容声明的前提） | `FpsTicks::frame_ticks` 用 `90000 / fps` 且只在整数 tick 时判定；author 侧 `Plan::snap_starts_to_grid` 提供吸附（CLI `--snap-pts`，默认关）；23.976 这类非整数 tick 的网格**不判**而不是罚人（AA5/AK16） | `an_on_grid_stream_raises_no_frame_boundary_warning`、`a_fractional_frame_period_is_not_judged_at_all`、`aa5_snapping_puts_every_display_set_on_the_frame_grid`、`aa5_snapping_refuses_a_grid_that_is_not_whole_ticks`、`aa5_snapping_never_collapses_two_display_sets_onto_one_tick`、`aa5_snapping_is_opt_in_and_the_default_path_stays_off_grid`、`aa5_a_snapped_origin_still_hits_the_decode_lead_floor`、`pgs_encode_on_a_frame_grid_raises_no_boundary_warning_and_snapping_is_inert` |
| 12 §2 的 timeout：挂住不等于永远等 | 读线程 + `mpsc::recv_timeout`，超时返回 `STK-RND-0203` 并交代约束状态；`abandon()` 关 Job 句柄带走整棵树 | `at_render_002_a_hanging_host_times_out_and_can_be_abandoned`（700ms 超时确实生效；放手后 `killed` 为真，且"没约束住"必须带非零错误码） |
| AT-RENDER-003：x86 DLL 自动路由 x86 host；**不得跨位数 `LoadLibrary`** | `pe::arch_of_file` 只读 PE 头（不加载、不执行），位数不符在 **spawn 之前**拒绝；host 侧加载之前再自查一次（两侧都查，AE2） | `at_render_003_an_x86_provider_is_never_handed_to_the_x64_host`（拿真的 `SysWOW64\kernel32.dll`：`Arch{provider:"x86", host:"x64"}` + `STK-RND-0201`；非 PE 文件给可读诊断）、`the_host_refuses_a_mismatched_provider_before_loading_it`（直接喂 host：回 `Error` 并非零退出）、`machine_values_map_to_the_documented_arches`（0x8664 才是 x64；Itanium 0x0200 归 Other 并被拒绝） |
| 12 §2 的 Job Object 限制 child tree | 新 crate `win-job`（`media-probe` 与 host 共用一份实现）；附着失败**返回并显示 Win32 错误码** | `attach_either_succeeds_or_reports_an_explained_win32_code`。修结构体之前这条从未成功过——那是 Z6 误诊的真因（AE3），修完 `media-probe` 在 nextest 下的 LEAK 消失 |
| 12 §2 的控制面握手 | `protocol::Handshake`：provider 类型/名字/版本/API 版本/host 位数/provider 位数/像素格式/provider 二进制 SHA-256 | `text_and_blob_frames_round_trip_exactly`、`every_host_failure_has_its_own_code`（六类失败各有独立错误码，12 §5） |
| 25 AT-SEC-001 的纪律延续到协议层 | 分帧的长度字段先与 `MAX_MESSAGE_BYTES` 比再分配；截断的头部是错误而不是死等 | `absurd_declared_length_is_refused_before_allocating_it`、`truncated_header_is_an_error_not_a_hang_or_a_panic`、`unknown_tag_is_rejected` |
| 20 §契约：能力要能被用户自己查 | `doctor` 新增 `render_host` 段：host 路径、握手身份、`tree_contained`/`job_error`，不可用时给原因 | `doctor_json_is_one_line_and_names_missing_capabilities` 的 render_host 断言 + 门阶 `renderer-host` 里对 `doctor --json` 的 grep |
| 28 §2：`renderer-host` 是发布产物之一 | `cargo build --workspace --release` 现在必须同时产出 `subtitletool-cli.exe` 与 `subtitletool-render-host.exe` | 门阶 `release-build`；`renderer-host` 门阶自己构建并跑 host 的验收测试 |

还没做（不要当成已完成）：大帧走 shared memory / file mapping（AE4，P1 —— 没做之前不把 `pgs encode` 的采样循环搬进 host）；minidump（AE5，P2）；
`renderer-host-x86.exe` 与真 x86 provider 的端到端（AE6，P2）；真实第三方 provider 的崩溃验证（AE7 的残余）。

## 29. 真 ffprobe 与合成视频语料（16 §2-§5、AT-PROBE-001，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| AT-PROBE-001：探测真实视频得到帧率/尺寸/色彩候选，且**永不**自动改时间轴 | `SystemProbe`（固定 argv、Job Object 约束、超时）→ `parse_video_streams` → `suggest_for`；CLI `probe` 输出 `video_streams` + `suggestions` | `real_ffprobe_reads_exact_25fps_sdr_without_hdr_advice`（25/1 必须读成精确 `25/1`，不是 24.999…）、`real_ffprobe_reads_uhd_hdr_and_suggests_the_hdr_preset`（PQ+BT.2020 → `HdrPreset{203}` High，且每条候选都带 `evidence`）、`real_ffprobe_does_not_suggest_a_canvas_for_a_4_3_source` |
| 16 §2 的色彩字段 | 读**扁平**的 `color_transfer`/`color_primaries`/`color_space`(矩阵)/`color_range`；嵌套形式仅兜底 | `parses_the_captured_real_ffprobe_payload` 直接吃 `tests/fixtures/*.ffprobe.json`（真工具抓下来的原始输出）。这一条是补 BUG 的：此前按构想的嵌套 schema 解析，真二进制读出 `transfer=None`，HDR 判定全盲（ISSUES AF1） |
| 16 §5 的二进制身份 | `FfprobeIdentity`（路径 + 大小 + 内容哈希），CLI 与 doctor 都按内容报 | `ffprobe_identity_is_reported_by_content_not_by_name`、`identity_digest_is_stable_and_detects_change` |
| 依赖不可用要可报告 | 找不到二进制 → `STK-DEP-*`；`doctor` 报 `ffprobe_search_paths`（找过哪几处） | `probing_a_missing_file_reports_instead_of_hanging`、`missing_binary_yields_dependency_error_not_panic` |
| 语料可复现 | `tools/make_probe_fixtures.sh`：三条 ffmpeg 命令 + 两份 ffprobe 原始 JSON + 自检 | 本机重跑得到同尺寸文件（828 KB 三个 mp4 + 两份 JSON）；二进制来源与 SHA-256 记在 `third_party/SOURCE_AUDIT_LOCK.json` |
| 子进程的 stdout 与 stderr **都要一边跑一边排空**，否则话多的进程会把我们锁死在 `wait`（D10） | `runner.rs::spawn_and_read`：两条读线程，`wait` 之后从各自通道取文本；超时分支只在 Job Object 约束住时才 join 线程，否则留下线程并如实报 `TimeoutUncontained`（Z6） | `chatty_stderr_does_not_deadlock_the_wait`（假 probe 吐 200 KB stderr，判据是 25 s 的时间上界；反向验证过：顺序改回"先 wait 后读"时该测试以 25.01 s 失败）、`timeout_kills_child`、`nonzero_exit_is_reported_with_stderr` |
| 28 §7 CI 等价 | 新门阶 `probe-real`：无 ffprobe 时 SKIP，`--strict` 下 SKIP 即 FAIL | 梯子的第 15 条（当时的总数是 19；加进 `font-scan` 之后是 20） |

没做：真发行素材（可变帧率、容器/流时长打架、telecine/interlaced）——合成 testsrc2 覆盖不到，见 ISSUES AF5 与待决策 E3。

## 30. 字体服务与 `font scan`（17 §2-§4、25 AT-FONT-001/002，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 17 §2 数据库要记的字段：localized family、typographic/Win32 family、full/PostScript 名、weight/style/stretch、TTC 集合索引、glyph coverage、SHA-256 与 mtime | 新 crate `font-service`（20 个 crate 之一）。`db::FaceRecord` 用 `skrifa`/`read-fonts` 读 `name`/`OS2`/`head`/`maxp`/`cmap`/`fvar`，**不靠文件名**；TTC/OTC 由 `FileRef::fonts()` 逐成员给出并回填 `collection_len`；`axes: Vec<AxisInfo>` 记 fvar 的标签与 min/default/max（AG9）；读不了或格式不支持的文件必须留下 `ScanNote`，静默少扫等于报告在撒谎 | `a_real_font_library_is_read_from_its_tables_not_its_filenames`：本机 `C:\Windows\Fonts` 实测 154 个文件 → 196 条 face、192 条 note；集合成员的索引互不相同（`msyhbd.ttc#1` 这类标签就是证据）；必须有 face 带非 ASCII 的本地化家族名 |
| 21 §1 依赖政策 + 23 §安全：解析不可信字节的依赖必须还在收修复 | 换掉 `ttf-parser`（RUSTSEC-2026-0192 unmaintained，无可安全升级版本）→ `skrifa` 0.48 + `read-fonts` 0.45 + `font-types` 0.12（fontations，MIT OR Apache-2.0）。**没有**动 `deny.toml` 的 `[advisories] ignore`，它仍然是空的 | `advisory-audit` 门阶：`cargo deny check advisories` = `advisories ok`；迁移等价性由 `cargo test -p font-service`（17 项，含 6 项真字体库）+ 同一台机器上扫描结果逐项相同（196 face / 154 file / 192 note、`{exact:3, missing_glyph:1}`）证明，见 ISSUES AG15 |
| 17 §2 "不能只扫描 `C:\Windows\Fonts` 的文件名" | 默认目录是 `%WINDIR%\Fonts` **加上** `%LOCALAPPDATA%\Microsoft\Windows\Fonts`（Win10 1703 起用户自装字体只落在这里）；`--fonts-dir` 可重复、去重、排序；指错的目录是错误，不是"库里没有" | `default_dirs_are_deduped_and_sorted`、`a_nonexistent_fonts_dir_is_an_error_not_a_missing_font`、e2e 断言 `fonts_dirs` 真的列出被扫目录 |
| 17 §3 用量扫描：动态 `Format:` 字段（不依赖固定列号）、Style 的 Fontname/Bold/Italic、inline `\fn`、`\b`、`\r`/`\rStyle`、dialogue 实际字符集 | `usage.rs` 复用 `codec-ass` 的 override token（含 `\t` 内层递归），维护"当前 face"状态；每个 (家族, 粗体, 斜体) 一条 `FaceRequest`，字集互不污染；来源精确到"第几行的哪个 tag" | `style_fields_follow_the_dynamic_format_line`、`inline_tags_move_the_current_face_forward`、`drawing_and_escapes_are_not_characters`、`weight_and_slant_are_separate_requests`、`scanning_is_deterministic_and_ordered`、`missing_events_section_is_an_error_not_an_empty_report` |
| 17 §4 候选评分八项与六档结论 | `report.rs` 的排序键：名字层级（family → win32 → postscript → full → localized）→ 字重距离 → 斜体罚分 → 宽度。第 7/8 项（variable axes、renderer 实际解析结果）**没有**用到，边界写在报告的 `scope` 里（ISSUES AG1/AG9） | `tier_order_follows_the_spec_not_convenience`、`tokens_split_on_punctuation_and_drop_single_letters`、`verdicts_separate_exact_alias_substituted_and_missing`、`postscript_and_localized_names_are_matchable_not_just_family` |
| 17 §4 的六档：Exact / Alias / Substituted / Missing / Missing Glyph / Ambiguous | `Verdict` 六档齐了。`MissingGlyph` 是**改判**出来的：名字对上（Exact/Alias）或走近似替代（Substituted）之后，赢面 face 覆盖不足就改判，原匹配方式保留在 `face.match_by`；`Ambiguous` 不改判。序列化名单由 `Verdict::SPEC_STATES` 一处定义，CLI 的计数键从序列化取，没有第二份手抄清单 | `the_six_spec_states_have_stable_serialization_names`、`a_named_face_that_cannot_draw_the_text_is_relabelled_missing_glyph`、`why_truncates_a_long_missing_list_but_points_at_the_full_one` |
| AT-FONT-001：中文字体缺字要被报告 | `db::missing_glyphs` 按 cmap 码位判，`FontMatch.missing_glyphs` 交清单，`why` 交人读的一句 | `at_font_001_missing_cjk_glyphs_are_reported_and_a_covering_font_is_clean`（两个方向都要验）、`font_scan_reports_against_the_real_font_library`（本机 Arial 真缺"你好"，评级与缺字必须自洽） |
| AT-FONT-002：字体环境变了必须给新答案，不能继续用旧库的结论 | `missing_glyphs` 判定前先重读文件并核对扫描时记下的 SHA-256，内容变了直接 `STK-INP-0222` 拒绝下结论；重扫即恢复可用 | `at_font_002_changed_font_content_invalidates_the_stale_database`（复制一份 ttf → 改最后一个字节 → 必须报错，且重扫后的哈希不同） |
| 20 §Commands：`font scan\|subset` | `font scan` 是实命令（report only，可选 `--out` 写报告）；`font subset` 保持占位并回 `STK-INP-0060`，帮助文本的"尚未实现"段落列它——**不因为 scan 落地就顺带宣称子集可用**（ISSUES AG2） | `font_scan_collects_repeated_font_dirs_and_rejects_stray_flags`、`scan_reports_the_database_and_writes_the_json`（含 dry-run 不碰盘、落盘内容与信封一致）、`unimplemented_commands_are_not_silently_ok`、e2e 里的 subset 占位断言 |
| 报告必须自证边界，不能只在源码注释里说 | `data.scope` 四条：report only、未使用 DirectWrite 权威、未询问 libass/VSFilterMod 的 fallback、缺字按 cmap 判定不含 shaping | e2e 断言 `scope` 含 `DirectWrite`；`font-cli.log` 里 `writes[].state == written` |
| 28 §7 CI 等价 | 新门阶 `font-scan`：没有可读字体目录时 SKIP、`--strict` 下 SKIP 即 FAIL；跑 `cargo test -p font-service`（含真字体库）+ 真 CLI 扫 `full22.ass` 并检查报告内容 | `bash tools/check_all.sh --strict` 的第 16 条（梯子总数由 19 变 20） |

没做（不要当成已完成）：`font subset` 与 17 §5 的子集等价 corpus（AG2，P1），以及 40 §5.2 要求 subset 必过的 GSUB/GPOS/ligature/Arabic/Indic/combining/vertical/variable/CFF2/TTC 等价测试（F10，P1）；17 §6 的 copy **subset** fonts、embed ASS `[Fonts]`、MKV attachments（§31 交付的是 report only + copy full fonts + manifest 这三件）；17 §7 的字体注入已在 §35 做完（默认关闭，剩下的见附件解析与默认开：AK5）；17 §6 的 embed `[Fonts]` / MKV 附件仍未做（AG8）；fvar 轴现在记下来了（`AxisInfo`）但 §4 的第 7 项评分仍未使用它（AG9，P2）；17 §2 的 DirectWrite 权威里"禁用/隐藏状态与 fallback 链"那半边还没接，注册表两张键是本轮接上的那半边（AG1/AH6）。

## 31. 注册字体与整份交付（17 §2 的"字体注册"、17 §6 第二/五档、17 §8，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 17 §2：不能只看 `%WINDIR%\Fonts`，"字体注册"必须参与 | 新模块 `font_service::registry`：枚举 HKLM/HKCU 的 `...\CurrentVersion\Fonts` 两张键（`RegOpenKeyExW` + `RegEnumValueW` 循环，advapi32 手写 FFI，没有新依赖），把登记值解析成真实文件（`%VAR%` 展开、绝对路径/相对写法/光文件名三种形态、**先用户目录再系统目录**的覆盖次序），再与目录扫描合并去重 | `enumeration_distinguishes_unsupported_from_failure`（本机实测 164 条登记、164 条解析到文件、0 条读取失败，与 `reg query` 的行数一致；两次枚举顺序必须相同）、`resolution_covers_the_shapes_the_registry_actually_uses`、`percent_vars_are_expanded_or_reported`、`unresolvable_values_get_a_reason_not_silence` |
| 两个来源指向同一文件不能重复计（否则 `files_read` 与 face 都在虚报） | `scan_registered`：按 `canonicalize` 的键去重，规范化失败才退回原路径；`registry_only` 只统计"目录里没有、注册表带来"的文件 | `registry_supplied_files_enter_the_database_without_double_counting`：空目录 + 外部文件（真进库）、同一路径两来源（`files_read == 1`、`registry_only == 0`）、两来源不重叠（`files_read == 2`、`registry_only == 1`）。本机所有登记项都落在系统目录里，所以"注册在别处"这一分支是用构造输入测的，不是用真注册表（AH2） |
| 17 §6 第二档 copy full fonts | `crates/cli/src/deliver.rs`：按 `--copy-fonts DIR` 把解析到的 face 文件复制过去，一个文件只复制一次（TTC 成员共用同一份，`face_index` 记在 manifest 里）；复制走 `pipeline::emit`，因此原子写、`--overwrite` 策略、`--dry-run` 不落盘与别的产物同一套规矩 | `copied_bytes_match_the_source_and_are_still_parseable`、`one_file_shared_by_two_members_is_copied_once`、`relative_destination_is_refused_before_any_write`、e2e `font_copy_delivers_the_file_and_a_manifest_that_names_its_source`（含 `--dry-run` 一个字节都不落、相对路径退 2） |
| 17 §6 第五档 manifest（原 font hash → subset hash → ASS request） | `manifest.json` 每条记 `source_file`/`source_sha256`/`face_index`/`collection_len`/`delivered_as`/`chars`/`requested_by`；没做 subset 就明确写 `subset_sha256: null`，并留三条 `limits` 说明本档边界 | 同上 e2e 断言 `subset_sha256` 为 null、来源路径与哈希齐全；门阶 `font-scan` grep `subset_sha256` 与 `"mode": "copy full fonts"`，并 `cmp` 复制件与原件逐字节相同（AH3） |
| 17 §8：永远保留原字体来源信息；subset 是可回退优化而不是必需 | 交付前先用解析器验一次原字节（`is_parseable_font`），复制后再验"字节哈希相同 + 仍能读回来"；没做 subset 就不假装有 | `nothing_is_delivered_when_no_request_resolved_a_face`（没有可交付 face 时必须失败并给出下一步，而不是交出一个空目录） |
| 20 §Commands：`font scan` 的新开关 | `--no-registry` 关掉注册表来源（默认开）；`--copy-fonts` 要求绝对路径，相对路径在解析层就是用法错误(2)，而不是写盘时才失败 | `font_scan_collects_repeated_font_dirs_and_rejects_stray_flags` 覆盖三种形态；`font_scan_reports_against_the_real_font_library` 断言报告里 `registry` 的 read/entries/resolved 自相一致 |
| 28 §7 CI 等价 | `font-scan` 阶往下多跑三步：注册表解析结果、临时目录里的整份交付（逐字节 `cmp` + manifest 内容）、交付的 `--dry-run` 不落盘 | 仍是梯子第 16 条；缺字体目录或本机没有可请求到的字体时 SKIP，`--strict` 下 SKIP 即 FAIL |

## 32. 渲染的字体环境身份（17 §7 最后一行、AT-FONT-002 的复用侧，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 17 §7：`render cache key` 必须含**实际用到的字体字节哈希** + resolver policy（renderer 版本由 provider 二进制哈希承担） | `font_service::env::identify`：每条请求取"命中 face 的 (路径, face_index, SHA-256)"或"缺席"，加 roots 与算法版本 `POLICY_VERSION`，出一个摘要；`cli::fontenv::Scanner` 一进程建库一次，逐输入出身份；`job_cache_key` 与 `render_cache_key` 都加了 `("fonts", digest)` | `a_changed_font_file_changes_the_identity`（换字体内容必换身份）、`a_request_that_later_resolves_changes_the_identity`（今天缺席、明天装上也要换）、`roots_are_part_of_the_identity_even_with_the_same_faces`、`identity_is_order_independent`、`a_different_requested_family_gives_a_different_identity`、`the_library_is_built_once_per_scanner` |
| 报告与渲染说的必须是同一个环境 | `font scan` 的报告带 `render_identity`（digest/policy/faces_used/faces_missing），与两个 cache key 用的是同一个函数 | e2e `font_scan_reports_against_the_real_font_library` 断言 `faces_used + faces_missing == requests`（每条请求都得在身份里占一格，否则换字体不会失效） |
| 交付要验"还是扫描时那份字节"（17 §8 的来源可追溯） | `FaceSummary.source_sha256` + `copy_fonts` 复制前核对，不一致即 `STK-OUT-0200` 并给出"重跑 font scan"的 action | `a_font_replaced_after_the_scan_is_refused`（改一个字节就必须拒绝，且不能留下半份交付） |
| 环境读不到时不能停摆，也不能假装完整 | 无字体来源时给 `complete:false` 的独立身份（渲染继续），`font scan` 仍报错（它的工作就是回答"这里有什么字体"） | `an_incomplete_environment_is_labeled_as_such`（不完整身份与"空库"必须是两个值） |
| 批量报告的可审计性 | `data.font_env`（逐输入的身份 JSON）+ `cache_keys` 用同一份身份重算，不再写 "none" | `cli --test batch` 的复用断言仍成立（key 变了会重做而不是 `from-cache`） |

没做（不要当成已完成）：这一节只钉住"我们按名字判断会用的 face"的哈希。要把身份变成"实际喂给渲染器的字节"，靠 17 §7 的注入 —— 机制已在 §35 落地（`--inject-fonts`），但**默认仍关闭**，且 `[Fonts]` 内嵌块未解析，剩下的那一半登记在 ISSUES AK5（P2）。

## 33. 字体库磁盘缓存与它的复核点（17 §2 的"数据库"、AI5 的性能债，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 建库不能每次从零开始（AI5：每个 CLI 进程 debug 档 ~45 s，整梯 5 分钟 → 20 分钟） | `font_service::cache`（`CACHE_VERSION` + `fingerprint` + `load`/`store`，`STK_FONTCACHE` 可换位置）+ `FontDb::scan_cached(dirs, registered, mode)`；`CacheMode` 三档：`Use` / `Refresh` / `Off` | 门阶 `font-scan` 阶实测：同一命令第二次 `"cache": "hit"` 且 `build_ms` 从 45 s 级掉到 57 ms 级；`a_hit_returns_the_same_database_as_a_full_scan`（命中给出的库与全量扫描逐条相同） |
| 缓存只能改变代价，不能改变答案 | 复核放在**使用点**而不是命中前：`FontDb::missing_glyphs` 读那个 face 的字节时顺手比内容哈希，不符即报 `FONT_CHANGED_AFTER_SCAN`；`evaluate_or_rescan` 看到这个码就用 `CacheMode::Refresh` 重建再解析，并把 `cache` 标成 `Refreshed` | `a_poisoned_cache_is_caught_where_the_bytes_are_used`（假哈希：缓存层照旧命中，但用它判覆盖必须炸，且自愈后的结论与全量扫描一致）、`a_poisoned_font_cache_still_gives_the_same_identity`（CLI 层：撒谎的缓存不能改变身份，也不能让命令失败）、`every_cache_path_gives_the_same_render_identity` |
| "这次信的是什么"必须看得见 | `CacheState`（`scanned`/`hit`/`stale-refreshed`/`off`）带 `label` 与 `meaning`，进 `font scan` 报告的 `database.cache` 与 `render_identity.cache`，也进渲染侧的 `env_json` | 门阶对这三格各有 grep 断言；`a_cache_hit_and_a_full_rescan_give_the_same_identity`（来历是成本事实，**不进**摘要，否则同一环境会产生两个缓存 key） |
| 怀疑缓存时要有一条独立可复现的路 | 全局开关 `--no-font-cache` → `CacheMode::Off`（不读也不写），`cli::fontenv::cache_mode` 给渲染链路与 `font scan` 共用同一判据 | `no_font_cache_is_a_global_switch`、`every_cache_path_gives_the_same_render_identity`、门阶"不读也不写 + 身份仍是同一个"那段断言 |
| 缓存不可用不能变成"命令跑不了" | `cache::load` 对版本不符/JSON 坏/空库一律返回 `None` 退回全量；`store` 失败静默（性能路径不值得让交付失败） | `an_unreadable_cache_falls_back_instead_of_failing`、`a_changed_font_file_invalidates_the_fingerprint` |

没做（不要当成已完成）：没被用到的 face 的元数据仍可能滞后一次（AI7，P3；把指纹换成"全部文件内容哈希"就能关掉，代价正是 AI5 实测不划算的那份）；17 §2 的 DirectWrite 权威视角仍缺禁用/隐藏状态与 fallback 链（AG1/AH6）。

## 35. 把字体字节交给渲染器（17 §7、AI4 的收口，2026-10-06）

| 契约 | 实现 | 证据 |
|---|---|---|
| 17 §7：Rust/Win32 读 font bytes → `ass_add_font()` 注入 renderer library | `render_libass::Provider::add_font`（运行时符号，`LIBASS_FONT_SYMBOLS` 只列 `ass_add_font`）；CLI 侧 `crate::front::InjectFace` + `render_to_plan` 在读文件之后、`load_script` 之前逐条交给 libass | `injected_font_bytes_are_what_the_renderer_draws_with`（不注入 `(StkProb,400,0) -> ArialMT`，注入后 `-> StkProb`，像素必须变）、门阶 `render-real` 用 `tools/check_injection_report.py` 核对真 CLI 报告 |
| 注入用的字节必须就是缓存 key 里钉住的那份 | 注入前重读文件算 SHA-256，与扫描记下的 `FaceSummary.source_sha256` 比对，不符即 `STK-RND-0088` 拒绝渲染（不静默按旧身份画新像素） | `a_font_replaced_after_the_scan_refuses_to_be_injected`、`render_reports_the_font_bytes_handed_to_the_renderer`（台账哈希必须落在 `font scan` 记过的集合里 —— 两条独立实现路径互相核对） |
| 符号缺失/能力不符不能变成"看起来成功了" | `can_inject_fonts()` 来自符号探测而不是文件名；缺 `ass_add_font` 时 `add_font` 报 `STK-RND-0085` 并给出两条可执行的出路；`ass_clear_fonts` 明确**不**做成能力（头文件要求先释放 renderer/track） | `capability_is_decided_by_symbols_not_by_the_file_name`、`add_font_refuses_bytes_that_cannot_be_handed_over`、`ffi::LIBASS_FONT_SYMBOLS` 的注释 |
| 报告要能区分"没开"与"开了但没字体"，也不能把顺序写成优先级 | `data.font_injection = {enabled, requested, embedded, from_library, injected[], meaning}`；`requested = embedded + from_library` 且开注入时 `injected.len() == requested`，没开时台账必须为空。这些等式由 `tools/check_injection_report.py` 在门阶上核对（**不按 JSON 字段顺序匹配**，AK7），测试另外钉相等 | `render_reports_the_font_bytes_handed_to_the_renderer`、门阶的 `inject-check`/`noinject-check`/`embed-check`/`plain-check` 四段 |
| 17 §7 第二句：`[Fonts]` 由我们自己的 attachment parser 处理 | `font_service::attachments`（`P:` + base64、单份与合计大小上限、可解析性、认不出的形状点名报错）→ 与库字体走同一条 `ass_add_font` 通路，且每一份都进 `render_identity` 摘要 | `parses_a_real_embedded_font_with_its_internal_family`、`a_payload_that_is_not_a_font_is_reported`、`old_style_or_unrecognised_entries_are_named_not_ignored`、`base64_matches_known_vectors`、`base64_refuses_garbage_instead_of_skipping_it`、`embedded_fonts_change_the_identity`、`embedded_fonts_change_the_identity_and_are_injected`、门阶的 `tools/make_embedded_fixture.py` 段（内嵌字节哈希进台账 + 去掉 `[Fonts]` 后身份必须变） |
| 同一批字节不交两次；同名多份时**谁赢由 libass 决定，不是注册顺序** | `inject_plan` 按内容哈希跨来源去重（`[Fonts]` 里嵌的就是库里那个文件时只留一条）。曾经写过的"先注册的内存字体胜出"已被像素否定：verdana 与 marlett 同改名成一个家族时，两种注册顺序画出同一份像素（AK8），所以这条只承诺实测成立的两件事——顺序不改变结果、赢家必是交出去的某批字节 | `registration_order_is_not_a_tiebreaker_for_the_same_family` |
| 新参数要么进缓存 key，要么改变某次系统调用（AD5 的纪律） | `inject` 进 `job_cache_key` 与 `render_cache_key`；子命令继承父 `Invocation` 的开关 | `the_injection_switch_changes_both_cache_keys`、`inject_fonts_is_accepted_by_every_rendering_command` |

没做（不要当成已完成）：**libass 自己的 extract-fonts 通路没接** —— 17 §7 那句"`[Fonts]` 可由 libass extraction 或我们自己的 attachment parser 处理"给的是二选一，我们走的是后者，因此没有绑定头文件 325 行那个开关（ass_set_extract_fonts），也不把字体解到临时目录。17 §6 的**输出侧**三档未做：copy **subset** fonts、把字体 embed 进产物 ASS、prepare MKV 附件（AG2/AG8，后两档还要等容器助手与外部 MKVToolNix）。`--inject-fonts` 默认关闭 —— 默认开等于改变所有历史产物的像素来路，要等 12 §9 的像素 diff 语料与真发行素材（AF5、待决策 E3）才敢合上这个开关。内嵌字体与系统同名字体冲突时不做"排除式覆盖"，那是留给用户的产品决定（AK9）。

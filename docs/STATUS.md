# 当前项目状态

核对日期：2026-10-17。本文描述当前工作区源码和实际执行的验证，不引用历史门阶结果充当本轮通过证据。

同日后续更新：
- 维护文档已按[38 项建议采纳记录](planning/AUDIT_ADOPTION_20261008.md)优化。新增用户提供的 libass 运行库与真实验证见[追加记录](reviews/2026-10-08-docs.md)。
- 2026-10-09 完成了全项目深度审查、测试过度防御修复与纯 Rust 前端规格重构，详见[2026-10-09 审查报告](reviews/2026-10-09-code-audit-defense.md)。
- 2026-10-16 落地了该审查的四项代码建议（IPC 自残后门、字体缓存坏就用坏即弃、Strict 不再被未实现层阻断、XML 加固幅述校正），证据与未做项见[审查报告第五节](reviews/2026-10-09-code-audit-defense.md)。新增 GUI crate（`crates/gui`，Iced 工作台）。
  **（2026-10-17 更新）**：这一条当时的结论“任务执行链未接通真实引擎、无测试”已**失效**——GUI 现已编译真实 batch spec 并交给 `cli::batch::run_batch` 执行，有 `gui_cli_replay.rs`（4 条）与 `task_e2e.rs`（4 条）两类端到端用例，且已进 portable 包。当前 GUI 的未完成项是 AT-UI-001 的 60 秒响应实测、A/B 对比渲染与设置持久化，见[GUI 接通裁决](reviews/2026-10-17-gui-backend-wiring.md)。
- 2026-10-16 核验了 Phase 0–2 的退出条件（均满足），并补齐 AT-BATCH-001 的 100×3 容量用例；仍未覆盖项与证据见[实施检查表](planning/39_IMPLEMENTATION_CHECKLIST.md)。
- 2026-10-17 **x86 renderer host 可构建，并抳出一个渲染缓存跨宿主复用的静默错误**：此前 AT-RENDER-003 记为“无可执行证据（需 x86 host 产物）”，基于一个**未经验证的假设**——“x86 host 需要 x86 libass DLL 才能构建”。实际上 `render-libass` 从第一行就写明**不在编译期链接 libass**（`LoadLibraryExW` + `GetProcAddress`），所以构建与是否存在 x86 DLL **完全无关**。实测 `cargo build -p renderer-host --target i686-pc-windows-msvc --release` 通过，产物 310.5 KiB、PE 机器类型 `0x014C`（真 32 位）；用真 libass 各渲一次，x64 与 x86 产物**逐字节相同**（516 字节 / `FF6AB07D3676F386`）。
  **抳出的真缺陷**：做上述对比时发现渲染缓存**跨宿主复用**——x64 先渲一次 `hit=false`，再把 `STK_RENDERER_HOST` 指向 x86 host 渲同一个东西得到 `hit=true` 且退 0，而那只 host **本来会拒绝**这个 x64 DLL（关掉缓存重跑即报 `STK-RND-0091`）。即 **host 压根没跑，它的位数校验也就没机会报错**；用户以为在用 x86 host，拿到的却是另一只宿主的像素。根因：`key_for` 含 provider DLL 哈希但**不含 host 身份**。已把 host 可执行文件的 SHA-256 加入 key，并把 key 版本从 `v1` 升到 `v2`（不升版本等于没修：旧记录会被 v2 算法当作命中）。修复后端到端复现脚本由“退 0 / hit=true”变为“退 1 / `STK-RND-0091`”；**负基线**：同一 host 连渲两次仍 `hit=false → true`，即修的是键而非把缓存关掉。新增 4 个单元用例。裁决：[x86-host-and-cache-key](reviews/2026-10-17-x86-host-and-cache-key.md)。
  **未完成（如实记录）**：AT-RENDER-003 要求两件事，本次只完成“x86 产物”；**“x86 DLL 路由”仍无证据**——需要真的 x86 libass DLL（本机只有 x64 那份），我验到的是“x86 host 正确**拒绝** x64 DLL”，那是反面证据（证明校验在位），**不是**路由可用的证据，所以 `at_coverage.py` 仍记为无证据；x86 host **未随包分发**（前提是 x86 DLL 的许可审计 21 §6），且与安装器里的文件名约定（`subtitletool-render-host-x86.exe`）尚未对齐；其余 crate 未验证 x86 可编译，**不声称“x86 支持已就绪”**。
- 2026-10-17 **交付 AT-BD-001 的 GUI 侧解释**（25 §AT-BD-001）：`gui/src/state/profile.rs` 新增 `problem()`/`is_runnable()`；`profiles_view.rs` 把一句**无条件说的假话**（“Basic PGS: 具备”无例外）换成按真实判定的“符合 09 §6 的硬规则 / 不合规：看下方说明”，并在展开卡片里给出原因 + 两条可选改法。
  **本条抳出一个真缺陷**：出厂默认的第二个 profile 是 `2160p 4K HDR SUP`，而 `pgs encode` 默认 `--level standard`、标准级 PGS 不允许 3840x2160 —— 实测报 `STK-PGS-0091`。即**用户装好、什么都不改、直接排队，第一个 profile 就失败**；而且失败发生在批量执行中途。已改成 `1080p HDR SUP`（HDR 映射 `ReferenceWhite(203)` 保留），回归用例 `the_default_sup_profile_is_not_4k` 钉住；实测新默认打通真 libass 并产出合法 SUP（`PG` 魔数、`no_blocking_findings: true`）。
  GUI **不重写判定**：直接调 `core_model::Canvas::within_bluray`（组件级单一事实源），有专门用例 `gui_agrees_with_the_core_rule_it_reuses` 锁住“GUI 不另立规则”。新增 9 个用例；`cargo test -p gui --lib` 35 passed。AT 覆盖 23/26 → **24/26**。裁决：[at-bd-001-gui](reviews/2026-10-17-at-bd-001-gui.md)。
- 2026-10-17 推进 Phase 6 发布起步：补齐根目录双许可正文，新增 `tools/make_portable.ps1` 并实测（解包后 CLI 与 renderer-host 可运行、`SHA256SUMS.txt` 校验通过）。**便携数据目录与安装器已于同日完成（见下两条）**；签名与第三方 DLL 分发审计仍未完成，见 [28 节](development/28_WINDOWS_BUILD_RELEASE.md)。
- 2026-10-17 **交付 Windows 安装器（28 §3）**：新增 `tools/subtitletool.iss`（Inno Setup）+ `tools/make_installer.ps1`（驱动，找不到 ISCC 或编译失败均退非零）+ `tools/verify_installer.ps1`（把五条要求真跑一遍）。产物 `SubtitleToolkit-0.1.0-setup.exe`（6.15 MiB）。五条要求的验证结果：**11 项检查 11 通过**——per-user 静默安装不弹 UAC、三 exe 到位；关联默认不写（opt-in，只勾 `.ass` 时其余四类一个不写）；x86 宿主为可选且默认不装；卸载选择（保留/删除）两条分支都验过。
  **本条抳出三个真缺陷**：① 卸载选择存在 Pascal 变量里，而卸载器是**另一个进程**（`InitializeWizard` 没在那边跑过），于是“默认保留”实际是“总是删数据”——改成写 `HKCU\Software\{AppId}` 并由卸载器读取，**读不到就当保留**；② `{#AppId}` 外层双花括号是 `[Setup]` 的转义约定，而 **Pascal 字符串里不转义**，拼出来的是 `Software\{{7E3A…}}` 这个无关键——该错法**自洽**（写读同位置，自检会通过），只有拿安装器写的键去比对才看得出；③ progid 键漏了 `uninsdeletekey`，卸载后“打开方式”列表里会留一个指向已删程序的空壳条目。另：第一版为“键空才删”自己写了个 `RegDeleteKey`，Inno 里根本没这个名字——`RegDeleteKeyIfEmpty` 是自带的。裁决与完整记录：[windows-installer](reviews/2026-10-17-windows-installer.md)。**未完成**：Authenticode 签名（28 §5）；x86 宿主未构建；all-users 模式**只核对了静态声明**，未做实机 UAC 验证（如实记录）。
- 2026-10-17 修正依赖许可门禁：`tools/notices.py` 原先只读已解压源码（平台专属依赖从不下载，误报 37 项"读不到 license"），且把 SPDX 的 `A OR B` 拆开逐个要求白名单（把 `Apache-2.0 OR GPL-2.0-only` 误报成 GPL 污染）。已改为回退读 `.crate` 归档、按 SPDX 语义判定，并新增 `--fetch`。真实结论：无 GPL/LGPL 强制义务；`gui` 关闭 iced 的 `svg` 特性后去掉了强制 MPL-2.0 的 `resvg`/`usvg`（依赖 487→470 包）。`THIRD_PARTY_NOTICES.md` 已重新生成。
- 2026-10-17 **用真实第三方商业 SUP（98.97 MB）驱动 PGS 校验与 Fast Path，修正四个真实现缺陷**：ODS `data_length` 语义算错（应含宽高 4 字节，编码器与校验器都错）、`PGS.TML.02` 基于发明出来的不变量（段间时钟本可不同）、Fast Path 要求逐段 PTS 单调、对象定义被判为不跨 display set 持续（真实文件有 1641 个复用集合）。修复后 `standard` 校验 `error=0`；**AT-PGS-001/002/003 均已执行并通过**（时间平移后 11286 个 ODS payload 逐字节一致、PTS/DTS 精确 +90000 tick；Forced 只改 PCS、调色板只改 PDS，ODS 摘要均不变）。详见[真实 SUP 互操作裁决](reviews/2026-10-17-pgs-real-corpus.md)。
- 2026-10-17 依赖许可门禁已全绿（`NOTICES OK … 0 个问题`）：补齐了 `notices.py` 的 SPDX OR 语义与 `.crate` 归档回退，`gui` 关闭 iced 的 `svg` 特性去掉强制 MPL-2.0 依赖，并恢复了两份被工作区删除的第三方源码归档。MKVToolNix v102.0 已按 21 §9 登记可用性与哈希。
- 2026-10-17 **实现 MKV 容器助手（18_CONTAINER_HELPER，AUDIT-A002），AT-MKV-001 用真实 MKVToolNix 执行通过（23/23 断言）**：新增 `container-adapter` crate（只调用 mkvmerge/mkvextract，不含任何 EBML 实现）与 `mkv identify|extract|remux` 三个子命令。`remux` 写临时文件后原子改名，写完重新 identify 并逐轨核对元数据（类型/编码/语言/名称/default/forced、附件数、章节数），对不上就删半成品并失败——源文件全程只读（内容哈希前后一致）。过程中修正四个真实现缺陷（mkvmerge v102.0 的 `chapters` 是对象列表而非计数；选项名与取值必须是两个 argv 元素，拼成一个带空格的串会 exit=2 且 stderr 为空；列表选项是 `--subtitle-tracks` 而"全丢"是 `--no-subtitles`；把"按要求改元数据"误判成"元数据漂移"），另发现并绕开一处既存撞码（`Input/200` 已被 `codec-srt` 占用，本命令改用 250–260 段）。详见[MKV 助手裁决](reviews/2026-10-17-mkv-container-helper.md)。
- 2026-10-17 **实现字体子集（17 §5/§6，ISSUES AG2/E4），AT-FONT-003 用真 libass 执行通过**：新增 `font subset` 与 `font-service::subset`。选 `allsorts` 0.17.0（Apache-2.0，符合 21 §1）；复杂脚本按码位区间一律回退整份；其余做 cmap 级等价检查（字符解出的字形存在性与 advance 一致）。**渲染级等价用真 libass 验证**：同一 ASS/canvas/fps 下整份 Arial 与子集各经 `--inject-fonts` 注入渲染，2 帧 PNG 逐像素一致（1,045,720 → 29,240 字节）。过程中发现并绕开 `allsorts` 的真实缺陷：请求列表不含 glyph 0 时子集会把 `U+0020` 的 cmap 映射写成 0（空格渲染成 .notdef）。详见[子集裁决](reviews/2026-10-17-font-subset.md)。
- 2026-10-17 **补齐 17 §6 的输出侧后两档（ISSUES AG8）**：新增 `font subset --embed-fonts --out X.ass`（把字体写进产物 ASS 的 `[Fonts]` 段）与 `font subset --mkv-attachments`（生成可直接嗂给 `mkv remux --attach-font` 的附件目录）。为此在 `font_service::attachments` 补了 base64 / `[Fonts]` 段的**编码器**，与既有解码器严格互逆（整份真字体字节往返用例守护）。过程中修正两个真实缺陷：**BOM 在文件头时 `\u{feff}[Fonts]` 不是一个段头，会导致整段隐形**；以及不去重会写出两份 `[Fonts]`（解析器只认第一份，第二份字节静默留下）。至此 17 §6 六档全部交付。见[子集裁决第七节](reviews/2026-10-17-font-subset.md)。
- 2026-10-17 **全部已规划命令落地**：`doctor` 的 `not_built` 已为空（`font subset` 与 `mkv *` 是最后两条）。占位机制（`Command::NotBuilt` + `not_built_commands`）保留，供后续新增命令使用。
- 2026-10-17 **GUI 与后端接通（20 §Repro）**：删掉 `task_worker` 里 `sleep+Ok(())` 的模拟执行，改把输入集与 Profile 矩阵编译成**真实 batch spec**（`cli::batch::Spec`）再交给 `cli::batch::run_batch` 跑——于是“GUI 复制的命令能直接重放”是结构上成立的，而不是靠事后对齐。同时修掉四个“看起来能用”的洞：`StartQueue` 从不调执行器、取消令牌跟引擎不是同一份（取消按钮无效）、`show_doctor` 设了但从不渲染、视频建议是写死字符串。预览占位色块新增 `FrameSource` 标记（不再冒充真实渲染），失败时清掉旧帧。仍**未**交付：AT-UI-001 的 60 秒持续响应实测、A/B 对比渲染、设置持久化。详见[GUI 接通裁决](reviews/2026-10-17-gui-backend-wiring.md)。
- 2026-10-17 **实现 PGS 校验第 5 层（解码器模型，13 §6）**：新增 `crates/validator/src/decoder_model.rs`。关键决定是**先用真实语料反推，而不是照抄理论公式**：理论模型（US7620297B2 / FFmpeg `supenc.c`）是编码器的前向模型，拿它量真实商业流命中率 0%（`PDS`/`END` 实际恒为 DTS==PTS，`ODS` 的 `PTS-DTS` 与 `w*h*FREQ/Rd` 无相关）。改为实现**真实流满足的 5 条不变式**，在本机 98.97 MB 真实 SUP 上全部 100% 成立（9951 个显示集 / 49659 段）。该层立刻抳出**我们自己编码器的两个缺陷**：`PDS` 与 `END` 都被写成了"有解码耗时"（真实流两者均恒为 DTS==PTS）。另更正覆盖缺口账目：`PGS.DEC.01` 已回收为真不变式，不再列为缺口，CLI 新增 `coverage_gap` 字段说明徽章为何被拒。详见[第 5 层裁决](reviews/2026-10-17-decoder-model-layer5.md)。：新增 `core_model::DataDir`；`portable.flag` 存在时运行期状态落在 exe 同级 `data/`，否则退回 `%LOCALAPPDATA%\subtitletool`。优先级 `STK_DATA_DIR` > `portable.flag` > 用户目录；`doctor --json` 新增 `data_dir` 与 `font_cache` 两项，报出路径、来源与理由。`make_portable.ps1` 默认在包里生成 `portable.flag`，且校验和改为覆盖包内**所有文件**。实测：解包后 `data_dir.kind=portable`，`font scan` 的缓存确实落在 `<包>/data/fontdb-v1.json`；回归用例 `crates/cli/tests/portable.rs`。仍未实现：安装器与 Authenticode 签名。
- 2026-10-17 **补齐 28 §2 的 SBOM，并把便携包补成 checksums + SBOM + notices 三件套**：`tools/notices.py` 现同时产出 `sbom.cdx.json`（CycloneDX 1.5，529 个组件，数据全取自它本来就在读的 `Cargo.lock`/registry/`SOURCE_AUDIT_LOCK.json`）。选 CycloneDX 而不是 SPDX：它原生表达“组件 + 哈希 + 许可证”这三件本仓库真正持有的事实（21 §11 要求的三样），字段少到能用一份自己的代码写对，不引第三方依赖。`scope` 把 `required` 与 `excluded` 分开（开发期工具与研究用源码归档不进安装包，列成 runtime 会让下游误判分发义务）；native/二进制产物带 `stk:hash-status`，`match`（本机实测相等）与 `absent-here`（这台机器没那个文件）**分开写**。门禁会查它过期：把 SBOM 改成 5 个组件后，`notices.py --check` 报 `SBOM 过期：有 5 个组件，当前依赖树应该是 529 个` 并退 1。便携包已实测：解包后 9 个文件的 SHA-256 全与 `SHA256SUMS.txt` 一致。
- 2026-10-17 **实现 37 §2 的磁盘渲染缓存**（`crates/cli/src/rendercache.rs`）：内存那级（批量内同 key 共享）此前已有，磁盘这级（配额、清理、跨运行复用）一直缺。入口挂在 `front::resolve_cached`——SUP 的 `pgs_encode` 与 BDN 的 `render::build` 唯一都经过的地方。CLI 新增 `cache stat|clear` 与 `--no-render-cache`；环境变量 `STK_RENDERCACHE`/`_DIR`/`_MAX_BYTES`（默认 512 MiB）。缓存 key 与内存共享**同一口径**（后半链参数不进 key，这正是跨运行复用成立的原因）。**读回时逐字段校验**（魔数、key、长度、patch 的 RGBA 与宽高自洽），不符就当作未命中并删掉。实测（真 libass）：第 1 次 `hit=false`、第 2 次 `hit=true`，两次 XML 与**每个 PNG 逐字节相同**；`--no-render-cache` 给出同一份产物；改文案必重渲染、改回重新命中；截断/改魔数/换 key 三种篡改都计 `corrupt` 并重渲染而产物不变；上限 20 KiB 时 5 份 17,786 字节记录只留 1 份。开发中抳出自己编码器的两个真缺陷（digest 误用带长度前缀的读法、`Patch` 字段序与声明序不一致）。详见[渲染缓存裁决](reviews/2026-10-17-render-cache.md)。
- 2026-10-17 **实现 36 §1 AppSettings 落盘**（28 §4 的 “settings 未实现”项）：`core_model::SettingsStore` + CLI `settings show|init|set`，文件在 `DataDir` 里的 `settings.json`。**未知字段保留**（36 §3）——这与 batch spec 的 `deny_unknown_fields` 是**相反**策略且理由也相反：spec 拼错字段会跑错任务（必须拒），设置里有陌生字段只说明它来自别的版本（必须留）。**单个字段坏不废整份**（设置是偏好不是数据）；**坏文件不退不硬删**（改名成 `.bad-<ts>` 留证据）；`schema` 高于本程序只读不写；写入原子；非法值在**写入前**就被拒（`STK-INP-0322`）。已实测：`set` 后**新进程**读得到、未知字段往返不丢、`init` 拒绝覆盖（`STK-INP-0320`）、5 种非法取值全拒且不落文件。详见[设置裁决](reviews/2026-10-17-app-settings.md)。
- 2026-10-17 **实现 23 §文件 的大文件拒绝**：新增 `pipeline::read_bounded`，**先 `metadata()` 看长度再读**
  ——`fs::read` 会先整份读进内存，之后拿大小判断就已经晚了（机器内存已吃完，用户可能只得到一句无关的 OOM）。
  两层上限：字幕文本 1 GiB（真字幕高两个数量级）、`probe`/SUP/BDN/批量 4 GiB。
  错误码 `STK-INP-0113` 单独立一个而不并进 0110（“太大”要换输入，“读不了”要修权限）；
  详情给出实际字节数与上限，并指出本工具处理字幕而非视频。
  修复过程中发现 `probe` 是漏网的一格——它不经过 `load_text`，而它恰恰是用户“拿错文件”时最可能先跑的命令。
  测试自身也修了一次：第一版用例跑了 168 秒（造了 2 GiB 文件而 `probe` 用 4 GiB 上限，于是真的读了它）；
  改用稀疏文件超过上限 + 用窄接口钉住 `>`/`>=` 边界后回到 0.10 秒。
  详见[大文件拒绝裁决](reviews/2026-10-17-input-size-limits.md)。
- 2026-10-17 **实现 23 §DLL 的 PE arch 检测，并修正 `JobSnapshot` 里写死的 `arch`**：：
  把 `renderer-host` 里的 PE 头读取 `git mv` 到 `core_model::pe`（不是复制）——
  主程序与 host 都要这个判断，而**两份实现对同一个 DLL 会给出不同答案**，
  那就是一边放行一边拒绝。CLI 的 `libass_dll()` 现在在加载之前就校验位数，
  并把失败分成三个可分的码：不存在 `STK-INP-0141` / 不是有效 PE `STK-INP-0142` /
  位数不符 `STK-INP-0143`（详情写明“本程序是 x64，该库是 x86”+ 可照做的 action）。
  另修正一个真缺陷：`batch.rs` 里的 `arch` 写死为 `"x86_64"`，换 32 位库时会照样写
  `x86_64` —— 而 `JobSnapshot` 的全部用途就是回答“当时用了哪个二进制”（20 §Repro）。
  已改为读 PE 头，读不到就如实写 `unknown`。证据：`cli/tests/dll_arch.rs` 5 用例
  （含真 libass 通过检查并真的渲染出 BDN XML，以及“同位 PE 不被误判”的负基线）；
  `core-model/src/pe.rs` 的 4 用例把 Itanium（0x0200）正确归为 `Other` 而不是 x64。
  详见[PE 架构检测裁决](reviews/2026-10-17-pe-arch-check.md)。
- 2026-10-17 **补上 17 §2 的权威交叉核对**（此前只说“未使用 DirectWrite”，
  而这个记载本身没有可执行价值）：新增 `font-service::gdi`，用 GDI 的
  `EnumFontFamiliesExW`（**扁平 C API**，非 COM）枚举系统字体，与我们的库对照，
  `font scan --json` 的 `cross_check` 段给出两边差异。本机实测：GDI 原始 6020 face /
  过滤后 3233 / 库里 1376 / `missing_from_db` **2 个**（创艺简行楷、微软简老宋）。
  为何不用 DirectWrite：`IDWriteFactory` 是 COM，手写 vtable 一个字段错就崩，
  而那类崩溃无法用测试穷尽（同 23 §XML、PNG deflate、真 minidump 的取向）。
  开发中抳出一个真缺陷：`TEXTMETRICW` 的 `tmFirstChar`…`tmBreakChar` 在 `wingdi.h`
  里是 `WCHAR`（u16）而我写成了 `u8`，于是 `tmPitchAndFamily` 偏移差 4 字节、
  恒读为 0，**所有字体都被当成非 TrueType**（6020 个 face 过滤后只剩 0 个）。
  纯构造数据的单元测试看不见它 —— 只有真机用例能抳出来。已加编译期
  `offset_of!` 自检，让这类错不再能静默通过。另：GDI 的具名实例与带风格后缀的族
  （`Bahnschrift Light` 等）是同一份字体，需按尾部前缀剥离，否则会刷出 63 个假缺口。
  详见[GDI 交叉核对裁决](reviews/2026-10-17-gdi-font-cross-check.md)。
- 2026-10-17 **把 A/B 分屏对比从占位控件改成真实现**（05 §4.1）：交付前
  `ToggleSplitAb` **只翻转一个布尔值并改按钮文字**，画面毫无变化 ——
  而那正是 05 §1 第 3 条明令禁止的“占位控件”。现在 B 侧对**脚本文本**
  应用 Profile 的变换（HDR 调光 + 等比缩放）后再渲染，用的是与 `hdr` 命令、
  导出链**同一份** `transform_hdr` / `transform_resolution`（因为用户开对比的
  目的就是预判导出成什么样）。画面拼接在视图层做（左右分屏 + 中缝分隔线），
  状态里存两张独立帧，于是切布局不需重渲。
  开发中抳出三个真缺陷：① `HdrSettings` 的 `touch_styles`/`touch_inline`
  默认为 `false`，拿 `..Default::default()` 拼出来的设置**一个颜色都不改却报成功**
  （必须用 `reference_white()`/`percentage()` 构造函数；若无“像素真的变了”
  这条断言它会一路绿过去）；② `ReferenceWhite(203)` 是**恒等映射**
  （203 就是 SDR 基准）且恰是默认 profile 的取值，于是新用户会看到两张一样的图
  —— 现在注记会说清楚是恒等而不是功能坏了；③ `clear_frame` 只清 A 侧，
  于是下一次渲好时会拿**新的 A** 配**旧的 B**，而那两张图看起来都正常。
  证据：`gui/tests/ab_compare.rs` 9 用例（含真 libass 的像素差异断言与负基线）。
  详见[A/B 对比裁决](reviews/2026-10-17-ab-split-compare.md)。
- 2026-10-17 **实现 22 §6 的崩溃现场记录**（真 minidump 仍未做，且说明了为何不做）：
  新增 `renderer-host::crash`。host 在阶段边界（启动 / 载脚本 / 出第 N 帧 / 收尾）
  把现场写成一个 json；主进程崩后读回来，把「阶段、序号、provider 身份、
  上一个成功动作」拼进**错误卡**。手工验证：注入 abort 后退出码 3221226505（`0xC0000409`），
  错误卡里出现 `崩溃现场：阶段=startup；已运行=0ms；原因=注入 abort`。
  最关键的一条是 D-CRASH-04：**起 host 之前先删旧记录** —— 真崩溃常常发生在
  写不出记录的地方（访问违例、栈溢出），那时留着上一回的文件会让报告说出一个
  “斩钉截铁但完全错误”的阶段与 provider，**一份错的现场比没有现场坏得多**。
  已实现并验证：同目录先注入再不带注入重跑，记录已不存在。
  真 minidump 不做：它必须在**异常处理器**里调 `MiniDumpWriteDump`
  （一堆异步信号安全约束，写错比不写更危险），且 dump 含进程内存镜像
  —— 而本产品处理用户的字幕与内嵌字体，诊断包默认要能直接外发（22 §5）。
  记成**已知能力缺口**而不是用残缺的异常处理器假装做到。
  详见[崩溃现场记录裁决](reviews/2026-10-17-host-crash-record.md)。
- 2026-10-17 **给 AT-UI-001 补上实测证据**（此前只有结构证据）：新增
  `crates/gui/tests/ui_responsiveness.rs`（5 用例）。真重任务（12 份输入 × 4000 条 cue）
  在后台跑时反复计时 `SubtitleApp::update`，实测**最慢 18.5µs / 均值 9.5µs**，
  距离 37 §1 的 8ms 预算还有 430 倍余量（实测值由用例打出来，不只是一句“通过”）。
  另含：取消点击不阻塞且令牌与 worker 是同一份（历史上真出现过“取消按钮无效”）、
  `StartQueue` 真的置位运行状态并换新令牌、无输入时不假装在跑。
  开发中抳出一个**假绿**：第一版只调 `update` 不 poll 它返回的 `Task`，
  后台永不调度 —— 表现是“任务 60 秒没动而 update 一直很快”，
  那只证明了 UI 很快，因为后台压根没跑。
  至此 AT 覆盖变为 23/26（`tools/at_coverage.py`）。
  详见[UI 响应性裁决](reviews/2026-10-17-ui-responsiveness.md)。
- 2026-10-17 **实现 22 §2 correlation id**：新增 `core_model::corrid`。
  格式 `{J|O|R}-{8hex 共享段}-{12hex 唯一尾}`：前缀标出层级，共享段让“同一个 Job 的
  全部下级”在字面上就是一家人（按它分组即可聚一次执行）。信封、人读输出末尾的
  `corr-id: ` 行与日志 span 三处是**同一个值** —— 两边各自生成过一版，结果
  “都像 UUID 但对不上”，用户贴出的 id 在日志里搜不到；现在有一条测试专门钉这件事。
  过程中抳出两个真缺陷：批量路径的信封与报告是两个不同的 id
  （新增 `claim_corr_id` 区分“命令自己定的 id”），以及文件日志里 span 字段名带 ANSI
  转义（`layer.with_ansi(false)` 不覆盖 span 字段，需换自定义 field formatter）——
  后者是上一轮刚加的“日志文件无 ESC 字节”断言当场拦住的回归。
  `job_id`（用户给的任务名）与 `corr_id`（本次执行）刻意不合并。
  详见[关联 ID 裁决](reviews/2026-10-17-correlation-id.md)。
- 2026-10-17 **实现 22 §5 诊断包**（CLI `diagnose`）：把故障现场打成 zip（环境/版本/脱敏设置/依赖身份/缓存账目）。规格最要紧的一句是“**默认不包含原字幕/视频**”，所以默认是**白名单**而不是黑名单——诊断包天生要被贴到 issue、发邮件，“先收进来、发之前再删”会在没读说明的用户身上失效。脱敏判据是**键名**而不是值，且保留“这个键存在”这个事实。包内自带 `README.txt` 说明自己的边界。ZIP 写入器是手写的（`flate2` 已在依赖树里，不引 zip crate），因此**必须自己验**：独立读取器逐条解压 + 验 CRC，并用 Python `zipfile` 做第三方复核（`testzip() -> None`）。开发中抳出三个真缺陷，其中一个会被“写读自洽”的测试整批放过（中央目录项漏写名字：包开头签名全对而 EOCD 的 `cd_size` 偏小，Python 报 `Truncated central directory`）。详见[诊断包裁决](reviews/2026-10-17-diagnostic-bundle.md)。

## 项目边界

- 项目面向 Windows x64，正式 Rust target 为 `x86_64-pc-windows-msvc`。
- 当前交付形态是 CLI + Iced GUI + renderer host；安装包、Authenticode 签名与更新器未实现。
- Cargo 工作区包含 `subtitletool-cli.exe`、`subtitletoolkit-gui.exe` 与 `subtitletool-render-host.exe`。renderer host 用于隔离 libass DLL。
- `P0` 等级表示产品目标优先级，不表示该功能已交付。

## 已有实现

- ASS/SSA、SRT 解析与转换，时间、颜色和分辨率变换。
- CLI 批处理、输出事务、失败恢复、JSON 报告和 `doctor`。
- ffprobe adapter；视频探测只给建议，不自动改变字幕时间轴。
- `font scan`、`font subset`（含 `--embed-fonts` / `--mkv-attachments`）、`mkv identify|extract|remux`、`cache stat|clear`、`settings show|init|set`、`diagnose`、`batch`、`render`、`doctor`。
- 37 §2 两级渲染缓存：内存（批量内同 key 共享）与磁盘（配额、清理、损坏自愈、跨运行复用）。
- 36 §1 AppSettings 落盘（未知字段保留、坏文件不阻塞、原子写）。
- 22 §5 诊断包（默认白名单，附样本需显式点名；脱敏保留键的存在；含本程序自己的日志）。
- 22 §1 结构化日志（`tracing`），按日轮转 7 天 + 10 MiB 总量上限，随 `DataDir` 落盘。
- 22 §2 correlation id（`J/O/R` 三层 + 共享段）：信封、人读输出和日志三处是同一个值，
  batch 另有 `output_ids[]` 与 `renderer_id`。
- 22 §6 的崩溃现场记录（`renderer-host::crash`）：host 在阶段边界写 json，
  主进程读回来进错误卡（阶段/第几帧/provider 身份）；起 host 前先删旧记录。
  **真 minidump 未做**（需异常处理器 + 含内存镜像，与 22 §5 默认可外发相抵）。
- AT-UI-001 的**实测**：真重任务在后台跑时反复计时 `SubtitleApp::update`，
  最慢 18.5µs / 均值 9.5µs（预算 8ms，37 §1）；取消点击不阻塞且令牌与 worker 同源。
- 17 §2 的 GDI 权威交叉核对（`font-service::gdi`）：本机实测 GDI 原始 6020 face /
  过滤后 3233 / 库里 1376，`missing_from_db` 逐家族列出（本机 2 个）。
- 05 §4.1 的 A/B 分屏对比：B 侧对**脚本**应用 Profile 变换（HDR + 等比缩放）后重渲，
  用与导出同一份 `transform_hdr`/`transform_resolution`；恒等映射会说明原因；
  任一侧为空则两侧都清（防新的 A 配旧的 B）。
  用与导出同一份 `transform_hdr`/`transform_resolution`；恒等映射会说明原因；
  任一侧为空则两侧都清（防新的 A 配旧的 B）。
- 23 §DLL 的 PE arch 检测（单份实现在 `core_model::pe`，主程序与 host 共用）；
  CLI 把“不存在/不是 PE/位数不符”分成三个可分的错误码；`JobSnapshot` 的 `arch` 读 PE 头而非写死。
- 23 §文件 的大文件拒绝（`pipeline::read_bounded`）：**先 stat 再读**，
  文本 1 GiB / 二进制 4 GiB，错误码 `STK-INP-0113`，详情给实际字节数与上限。
- 23 §文件 的极端 cue 数上限（`core_model::limits`）：SRT `STK-PRS-0123` /
  ASS `STK-INP-0114`，上限 100 万条；超限报错而不静默截断。
- libass 渲染、renderer-host 进程隔离、BDN XML/PNG 输出和 Basic PGS 编码链。
- PGS 段解析、Fast Path 变换、分层校验及 ODS 内容摘要。
- 字体扫描、缺字报告、整字体复制与 manifest、渲染字体身份和字体注入。
- 全局环境测试加速：配置 `[profile.dev.package."*"] opt-level = 3`，纯算法与字体计算在 dev 模式下提速 10~50 倍。
- 优化了测试门禁：真实系统字体测试默认跳过（显式指定 `STK_REQUIRE_FONTS=1` 或 `STK_FONTS_DIR` 触发），`fontenv` 单测使用隔离单字体测试目录，彻底消除全量扫描 `C:\Windows\Fonts` 导致的单测超时卡死。
- 前端设计重构：将原本物理割裂的 10 个独立页面重塑为“三栏一体化主工作台 + 单帧实时预览 Canvas + 多 Profile 矩阵”，见 [05 UI 页面规格](specs/05_UI_SCREEN_SPEC.md) 与 [35 UI 交互规范](specs/35_UI_STATE_INTERACTION.md)。

实现边界和验收要求见[功能矩阵](specs/03_FEATURE_MATRIX.md)及[验收用例](specs/25_ACCEPTANCE_TESTS.md)。

## 未交付或未裁决

- CSRI/VSFilterMod provider、x86 renderer host 和发行安装链未实现。
- 字体服务未使用 DirectWrite 作为权威 resolver；但已补 **GDI 权威交叉核对**
  （`font scan --json` 的 `cross_check`）：GDI 看得到而库里匹配不上的家族会逐个列出。
  DirectWrite 的 **fallback 链**仍未做（需 COM，ISSUES AG1）。
  字体子集正确性尚无复杂文字渲染语料。
- PGS Basic 编码不等于 Strict Blu-ray 合规。PCS 帧率字段、palette range/matrix、ODS 图末标记尚无真盘/设备结论；当前 ffprobe 证据只覆盖 `.sup` 容器段布局与时钟。第 5 层的不变式取的是**必要条件**（不满足一定有问题），不是充分条件。
- Strict validator 已实现第 1–6 层（含第 5 层解码器模型）；**第 7 层设备语料**仍未取得，因此 `strict_badge_allowed` 仍恒为 false（报告给出 `coverage_gap`）。第 5 层本身未含缓冲区占用曲线与 acquisition/seamless 分支。
- 仓库已补根目录 MIT/Apache-2.0 license 正文（`LICENSE-MIT`、`LICENSE-APACHE`，2026-10-17）；portable 包可由 [tools/make_portable.ps1](../tools/README.md) 生成（含三个 exe、`portable.flag`、`sbom.cdx.json`、许可正文与 `SHA256SUMS.txt`）。安装器已由 [tools/make_installer.ps1](../tools/README.md) 交付（2026-10-17，见上），逐条验证用 `tools/verify_installer.ps1`。**Authenticode 签名仍未完成**（这是发布链上剩下的一项）；第三方 DLL 分发审计也未完成。
- 22 §1 日志文件已实现（`core_model::logging`，`tracing` + 按日轮转 + 启动清理，
  随 `DataDir` 落盘、便携包内自动进 `data/logs/`）；日志已进 `diagnose` 的诊断包。
  **22 §2 correlation id 已实现**（`J/O/R` 三层，日志与报告同一个值）。
  **22 §6 的崩溃现场记录已实现**（阶段/帧号/provider 身份进错误卡）；
  真 minidump 仍未做（需异常处理器，且含内存镜像与 22 §5 冲突）。
- 36 §1 的另外两层（ToolDefaults、Preset）未实现；Preset 需要 TOML 依赖获批（ISSUES E4）。

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

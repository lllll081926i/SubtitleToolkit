# 正式验收用例

本表定义验收条件，不代表用例已经通过。需要真实 DLL、播放器、真盘 SUP 或 GUI 的用例，必须明确记录依赖与样本；模拟测试不能代替互操作结果。当前交付状态见 [`../STATUS.md`](../STATUS.md)。

- AT-ASS-001：parse→write 未变换且未重新编码的已支持输入逐字节等价，未知 tag 不丢；语义检查不能替代字节比较。
- AT-ASS-002：HDR transform 只改目标颜色字段，时间/文本/几何 hash 不变。
- AT-CANVAS-001：1080→2160 特效字幕保持完整16:9坐标，关键定位×2。
- AT-CANVAS-002：2.39:1 视频辅助不得把 ASS 自动改为非16:9逻辑画布。
- AT-MULTI-001：单 ASS 一次生成 1080 ASS + 2160 ASS + 1080 SUP + SRT，所有 output 独立 verify。
- AT-MULTI-002：BDN+SUP 相同 render profile 只渲染一次（cache metrics 证明）。
- AT-TIME-001：24000/1001→25 speed conversion 长片无浮点累积漂移。
- AT-TIME-002：25→50 frame snap 不执行 1/2 变速，单个边界移动不超过半帧；短 cue 时长可能因量化变化。
- AT-PROBE-001：目标视频只填建议，不自动应用 speed conversion。
- AT-RENDER-001：设置 `STK_LIBASS_DLL` 后，真实 libass 经 renderer-host 输出正确 RGBA；故障只影响 host。
- AT-RENDER-002：取得可用 VSFilterMod provider 后，证明其崩溃只终止 renderer-host，GUI 可重试；自伤 `Poison` 测试不能代替该 provider 测试。
- AT-RENDER-003：仅在 x86 host 产物加入支持矩阵后，验证 x86 DLL 路由；当前缺少该产物，不验收自动路由。
- AT-PGS-001：对可合法使用的第三方 SUP 执行 +1000ms，ODS payload hash 完全一致；自制流往返不能代替外部样本。
- AT-PGS-002：Forced-only Fast Path 不重编码。
- AT-PGS-003：palette dim 只改 PDS，结构验证通过。
- AT-PGS-004：Basic Encoder segment length/RLE/object id/palette 正确。
- AT-BD-001：严格 Blu-ray profile 禁止 2160p PGS；UI 明确解释。
- AT-BDN-001：XML graphic width/height 与 PNG 一致；timecode frame 合法。
- AT-FONT-001：中文字体缺字被报告。
- AT-FONT-002：font environment 改变导致 render cache miss。
- AT-MKV-001：提取→处理→新MKV，原文件不改，字体附件存在。

> 2026-10-17 更新：已实现 `mkv identify|extract|remux`（18 §2/§4），并用真实 MKVToolNix v102.0
> 执行 **23/23 断言通过**：源文件 SHA-256 前后一致；产物经独立 `mkvmerge --identify` 认定合法且
> 逐轨元数据保持；字体附件经独立 `mkvextract` 抽回与原字体哈希一致；`--out` 等于输入时被拒（`STK-INP-0257`）。
> 集成用例 `crates/cli/tests/mkv.rs`（找不到 MKVToolNix 时明确跳过）。详见
> [MKV 助手裁决](../reviews/2026-10-17-mkv-container-helper.md)。**AT-MKV-001 已执行并通过**。
- AT-BATCH-001：100 文件×3 profiles，单文件失败不阻塞其他。
- AT-UI-001：重任务期间 UI 60s 内持续可响应、取消可点击。
- AT-PATH-001：中文/emoji/260+长路径正确。
- AT-SEC-001：恶意 PGS length/RLE 不崩溃、不超额分配。
- AT-OUT-001：写出后 verify 失败时 final 文件不替换旧文件。

## 执行记录合同

每次验收记录 AT ID、WBS ID、代码提交、输入路径/摘要、依赖版本/摘要、操作、判定依据、容差、结果与日志。结果分别记通过、失败、未覆盖；用例存在不等于执行通过。只有真实依赖可用且完成相应断言，才形成 native/设备结论。

## 字节与格式

**AT-ASS-001 / WBS-F021**：使用 `codec-ass/tests/fixtures/` 中 simple、unknown_tags、crlf、mixed_eol、comments_blanks、fonts 与 broken_lines。执行 parse→write，逐字节比较；BOM、换行、未知标签、注释与附件均不得变化。入口：`cargo test -p codec-ass --test roundtrip`；异常原文保留不表示输入语义有效。

**AT-PGS-001–004 / WBS-P002/P003/P004**：授权外部 SUP 与自制边界流分别登记。偏移、Forced、palette 操作各执行一次，断言白名单以外段字段不变，ODS payload 摘要与分片顺序不变。Basic 编码覆盖空帧、透明条目、引用与大 ODS 分片；独立解码 RGBA 对照不能用本项目 writer/reader 往返代替。现有入口为 `codec-pgs/tests/differential.rs`、`security.rs` 与 `cli/tests/cli.rs`；外部样本和设备报告不足时保持未覆盖。

> 2026-10-17 更新：已获得一份真实第三方商业 SUP（98.97 MB，SHA-256 `694f2e0d…9ad31`）并执行
> **解析、校验与 Fast Path（时间平移 / Forced / 调色板）**：`standard` 层 `error=0`；
> `pgs adjust --offset-ms 1000` 后 11,286 个 ODS payload 逐字节一致、所有段 PTS/DTS 精确
> +90,000 tick；Forced 只改 PCS、调色板只改 PDS，ODS 摘要均不变。期间修正了四个假阳性
> （ODS `data_length` 语义、`PGS.TML.02` 时间戳假设、Fast Path 逐段单调、对象跨集合复用），
> 见[真实 SUP 互操作裁决](../reviews/2026-10-17-pgs-real-corpus.md)。**AT-PGS-001/002/003 均已执行
> 并通过**；第 7 层设备语料仍待取得。

**AT-BD-001 / WBS-V002**：输入 2160p 与 Strict 组合，校验器应拒绝；已实现 CLI 验收参数拒绝，GUI 禁用解释在 GUI 阶段验收。不得把该拒绝测试当成其他 Profile 合规证明。

## 颜色与几何

**AT-ASS-002 / WBS-T020**：输入白/灰/彩色与透明颜色、动态 `\t`、未知扩展。按[11 的算法版本 1](11_HDR_COLOR.md)计算逐通道期待值；包括兼容减半 255→128、线性 50% 的 255→188、ReferenceWhite 203→比例 1。默认 alpha 不变；只允许目标颜色 token 改变。入口：`cargo test -p transform-hdr --lib` 和 `codec-ass/tests/colour_map.rs`。本组不证明实际显示 nit。

**AT-CANVAS-001/002 / WBS-T030/C003**：分别执行 Preserve 与 Rewrite，输入包含 PlayRes/LayoutRes、定位、drawing、黑边视频。Preserve 不改 ASS 字节；Rewrite 按已支持字段变换并报告未知项。bbox 放回完整画布后逐像素等价，跨 renderer 视觉阈值必须在对应素材中定义。入口：`transform-resolution`、`core-model::geometry`；真实 differential 语料单列。

## 时间与探测

**AT-TIME-001 / WBS-T010**：用 `24000/1001 → 25` 变换 120 分钟及非整帧边界。量化前与有理数公式完全相等，反向倍率还原；ASS/SRT/PGS 写出误差分别按百分之一秒、毫秒、tick 的舍入边界检查。入口：`cargo test -p transform-time --lib` 与 `core-model::time`。十进制 `23.976` 不作为该分数的替代输入。

**AT-TIME-002 / WBS-T010**：对短 cue 和半帧附近边界执行 FrameSnap；断言最近帧、半值规则、幂等与无 SpeedScale。负时间、控制点重合、非正斜率和溢出必须产生明确错误；不得静默丢事件。

**AT-PROBE-001 / WBS-A001**：使用 `media-probe/tests/fixtures/` 中视频和 JSON，执行真实 ffprobe，再核对 FPS/尺寸/HDR 元数据；同一任务的手动参数不得被建议覆盖。入口：`media-probe/tests/real_probe.rs`。JSON fixture 解析不代替真实子进程测试。

## 渲染与多输出

**AT-RENDER-001 / WBS-R001/R011**：固定 DLL 与字体环境，设置 `STK_LIBASS_DLL` 和 `STK_REQUIRE_LIBASS=1`，运行 `renderer-host/tests/host_isolation.rs`。真实 host 与直接调用输出 RGBA 逐字节一致，尺寸、非透明像素、身份和收尾正确。`render-libass/tests/render_real.rs` 另验文字、时间与字体注入。缺依赖不得提前返回后记通过。

**AT-RENDER-002/003 / WBS-R020/R012**：外部 provider 与 x86 路由需要实际产物。现有 libass host 的 Poison、timeout 和 x86 拒绝测试仅支持宿主故障/拒绝结论，不代替 VSFilterMod 崩溃行为或 x86 自动路由。

**AT-BDN-001 / WBS-B002**：真实 CLI 输出 XML+PNG，用独立 PNG 头解析核对尺寸，核对 XML 引用、事件顺序与 frame timecode。入口：`cargo test -p cli --test render`；这些断言不构成 authoring 消费者兼容认证。

**AT-MULTI-001/002 / WBS-R040/J003**：计划 1080/2160、不同颜色/字体/provider 及同组 BDN+SUP 组合。记录实际 renderer 调用次数；同组每个时间点一次，跨组分别调用。改变任一影响像素的条件应失效，输出分别校验。现有 CacheKey 分量测试只支持键变化；完整 CLI 共享计数仍待验收。

## 输出、字体与界面

**AT-OUT-001 / WBS-J004**：旧目标写入已知内容后制造 verify 失败，断言旧目标字节不变；再测安装失败、恢复失败与诊断路径。入口：`task-engine/src/lib.rs` 的真实文件事务测试。BDN 目录另测写至一半、锁文件和跨卷中断，未覆盖不得称断电安全。

**AT-BATCH-001 / WBS-J002**：生成 100 个输入与 3 个 profile，混入格式错误和输出冲突；核对逐 profile 结果、PartialSuccess、成功产物与恢复凭据。入口：`cli/tests/batch.rs` 与任务引擎模块；既有较小用例不自动满足本项容量验收。

**AT-FONT-001/002 / WBS-A003/J003**：使用本机授权字体与缺字内容，分别检查报告和改变字体字节后的 cache key。字体存在、匹配、覆盖、嵌入许可与再分发许可分开记录；入口为 font-service 与 CacheKey 测试。

**AT-PATH-001 / WBS-J004/A000**：在中文、emoji、长路径与受限目录运行真实读写和参数数组子进程；原路径保持不变。成功的短路径测试不扩展为所有 Win32 路径已验收。

**AT-SEC-001 / WBS-P004/SEC01**：对极端长度、RLE 截断、引用与尺寸运行现有 `codec-pgs/tests/security.rs`；断言错误、分配上限和无崩溃。PGS 文件测试不证明第三方 DLL 是权限沙箱。

**AT-MKV-001 / WBS-A002、AT-UI-001 / WBS-QA002**：当前未实现。MKV 验收使用真实工具逐轨核对 metadata/附件与源文件摘要；GUI 验收持续操作、取消、键盘、IME 与 DPI，不以设计文档或自制返回值替代。加入可执行模块后再记录真实证据。

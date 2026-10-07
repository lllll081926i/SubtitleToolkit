# V3.1 源代码级审计报告

> 审计日期：2026-10-05  
> 范围：只审与 SubtitleToolkit-RS 核心链直接相关的源码/API，不做泛泛功能点评。  
> 说明：当前沙箱网络禁止 `git clone`/直接下载仓库归档，因此本轮通过 GitHub/raw.githubusercontent 公共源码逐文件读取并核对；没有伪称本地 clone 成功。后续正式仓库建立后应把选定 commit 以 submodule/vendor-source-metadata 方式锁定并本地复跑本报告的 grep/tests。

## 1. 审计对象

| 项目 | 直接读取/核对 | 结论用途 |
|---|---|---|
| libass | `libass/ass.h` | renderer 初始化、坐标/画布、Windows 路径、字体注入 |
| Masaiki/ass2bdnxml | `ass2bdnxml.c`, `sup.c` | unified timeline、SUP高精度、PGS palette/ODS、变化检测 |
| SupMover | `main.cpp` | SUP Fast Path、PTS、palette brightness、forced/move/crop |
| kotorimiku/assfonts | `src/ass.rs`, `src/subset.rs` | font usage state、subset 风险 |
| ALASS | `alass-core/src/lib.rs` | constant/scaled/segmented sync 算法边界 |
| OxideAV | current workspace/sub-image/ASS feature surface | Rust parser/basic encoder bake-off |
| SUPer | current README/releases | advanced PGS author/buffering/bitrate 行为 |
| xy-VSFilter / VSFilterMod | current releases/repo | CSRI 与 legacy provider 边界 |

## 2. libass：我们原规划最需要修正的地方

### 2.1 `storage_size` 是硬要求，不是优化项
`ass.h` 明确写：renderer 使用前至少设置 storage size、frame size、fonts。storage size 会影响 3D transforms、source aspect、blur scale；没设置时 libass 会猜。

**工程决策**：RendererConfig 必须有：
```text
output_canvas: Size
storage_size: Size
pixel_aspect: Rational/Auto
font_environment: FontEnvironmentId
script_coordinate: ScriptResolution
layout_resolution: Option<Size>
```
任何字段都不能用另一个字段偷代替。

### 2.2 LayoutRes 不可机械缩放
合法 `LayoutRes*` 会优先影响 source image / PAR 计算。旧 V3 的“1080→2160 就全部 ×2”会对一部分现代 ASS 造成二次缩放。

**决策**：默认 Preserve Script；Rewrite Script Resolution 是高级 transform，并专门处理 LayoutRes。

### 2.3 Windows 文件路径
libass 自己的 file API 在 Windows 对 filename encoding 有专门约束。我们的 Rust 主程序已经能安全拿到 `PathBuf/OsStr`，没有必要再把路径窄化。

**决策**：字幕一律 `std::fs`/Win32 读 bytes → `ass_read_memory()`；字体 bytes → `ass_add_font()`。这同时方便 sandbox/provider-host 传输。

### 2.4 `detect_change` 不能当最终变化真值
API 只承诺“positions may have changed / content may have changed”。

**决策**：用它做 fast hint，最终事件 segmentation 用 composited RGBA hash/diff。

## 3. Masaiki/ass2bdnxml：值得直接吸收的架构思想

### 3.1 统一渲染时间线
当前源码有 `build_render_samples()`，并允许 SUP high precision 加 ASS event start/end；主循环只 render 一次，XML 和 SUP 分别消费自己需要的 samples。

**直接纳入架构**：`RenderTimelinePlanner`。

建议 IR：
```rust
struct RenderSample {
    pts90k: i64,
    render_ms: i64,
    reason: SampleReason,   // FrameBoundary/EventStart/EventEnd/Dynamic/Preview
    sinks: SinkMask,
}
```

多个输出 Profile 先求 sample union，再排序、去重、render。

### 3.2 可见图像比较
XML 路径直接比较实际 visible image，说明“renderer changed flag”在生产工具中不足以决定事件切分。

**决策**：FrameFingerprint = `(bbox, xxhash/Blake3 RGBA, optional byte compare)`。

### 3.3 PGS 写出细节
源码能作为我们 Basic Encoder 的 golden/reference：
- PDS 使用 Y/Cr/Cb/T；
- SD 与 HD 使用不同 RGB→YUV 系数选择；
- ODS 数据超过 segment payload 时分片；
- composition/window/object/palette 有状态；
- tight crop object 放在完整 composition canvas。

**但不能直接复用**：GPL；而且其 HD 路径主要是 BT.709，不是 HDR/BT.2020 的最终权威。

## 4. SupMover：Fast Path 很有价值，但 Strict 不能照抄

### 4.1 源码证明可以无损改大量字段
raw segment loop 中可以直接改：delay/resync、window/object position、forced、PDS brightness；不必重编码 ODS。

**决策**：`PgsTransformClass::MetadataOnly` 必须保证 ODS SHA-256 不变。

### 4.2 时间轴风险
源码的 resync/delay 主要改 `header.pts`。对播放器导向工具这很实用，但我们的 Strict 输出还需要考虑 DTS / decoding schedule。

**决策**：
- Fast Playback Transform 可以保留；
- Strict Transform 之后必须跑 decode scheduler validator；
- 失败则 re-author。

### 4.3 “tonemap”实际只是 Y scalar
源码把 limited-range Y 展开→乘系数→再压回 16..235，不动 chroma；项目 README 也说 B/W 才可靠。

**决策**：UI 不再用一个模糊 `Tonemap`：
- `Brightness (Luma only)`；
- `HDR/Color-managed palette transform`。

## 5. assfonts：Scanner 可借鉴，Subset 要更谨慎

### 5.1 Font state scanner
源码支持动态 ASS Format map，并在 Dialogue override 中跟踪 `\fn`, `\b`, `\i`, `\r`。

**决策**：这个行为加入我们的 font-usage tests，但仍由统一 ASS AST 提供 token，不再另写一套完整 parser。

### 5.2 Subset 风险
当前实现把 required chars 经 cmap 变 glyph IDs，再调 allsorts Minimal subset。源码短小清晰，但这并不能自动证明复杂 shaping 一定保持等价。

**必须验证**：GSUB/GPOS、ligature、Arabic、Indic、combining、vertical、variable、CFF/CFF2、TTC/OTC。

**回退原则**：subset 任一 equivalence test 失败 → 复制完整 face，而不是为了减体积损坏字幕。

## 6. ALASS：Smart Sync 的真实边界

源码层提供：
- `align_nosplit()`：constant shift；
- `align()`：允许 split；
- `overlap_scoring()`：适合 scaled subtitle / FPS correction。

核心吃的是 `TimeSpan`；视频参考需要 CLI 上层的 audio/VAD/ffmpeg。

**决策**：Smart Sync 结果必须解释成：
- Constant offset；
- Global scale/FPS-like drift；
- Segmented mapping。

Manual-first 产品中它继续是辅助工具，不自动覆盖用户文件。

## 7. OxideAV：仍是最有价值的纯 Rust bake-off 候选

当前 workspace 对 ASS/文本/bitmap subtitle 的覆盖面很大，尤其 `oxideav-sub-image` 宣称 stateful PGS decode + basic encode。它非常适合：
- parser/decoder；
- fuzz；
- basic encode；
- differential testing。

**风险**：项目仍在高速发展、社区规模小；不能因为纯 Rust 就跳过 version pin、corpus、player smoke、malformed/fuzz gate。

**决策**：Phase 0 继续保留候选；不直接授予 Strict Author 能力。

## 8. SUPer：Advanced Author 的重要模型

当前文档/发布说明体现了：palette update、normal case、ahead buffering、layout、acquisition、redraw、decoder buffer、事件丢弃决策。

重要修正：其 Maximum bitrate 是**测试输出**，不是主动 shaping。

**决策**：规划拆为：
- PeakBitrateValidator；
- BitrateShapingPolicy（未来独立能力）。

## 9. CSRI 与 VSFilterMod：接口必须拆清楚

2026 xy-VSFilter release 明确 external CSRI backend 使用标准 `csri_stream_ext` / BGRA，并说明某个 VSFilterMod r5.3.0 build 可作为 CSRI provider。

但 sorayuki 历史 VSFilterMod 主仓库主要暴露 VapourSynth/DirectShow 等路径，不能由文件名 `VSFilterMod.dll` 推断它一定是 CSRI provider。

**决策**：renderer-host 启动时 actual-probe：
```text
PE arch -> exported API -> CSRI capability -> VS/VapourSynth/AviSynth capability
```
绝不靠文件名选择调用 ABI。

## 10. 对 V3 的总体影响

没有推翻产品方向，但调整了实现优先级：

1. ASS coordinate model 提前成为 Phase 0 数据模型；
2. libass memory-input + DirectWrite font environment 提前；
3. RenderTimelinePlanner 提前到 BDN/SUP 之前；
4. PGS decoder/state machine 必须先于 Fast Path；
5. Fast Path 与 Strict Author 完全分层；
6. font subset 从“功能实现”变成“实现 + rendering equivalence gate”；
7. renderer provider 用 capability probe 而不是 DLL 名字；
8. 多输出直接从第一版 pipeline 就按共享采样/缓存设计，避免后期推翻。

## 11. 下一步正式开发前还要做的源码动作

当正式开发环境可 clone GitHub 时，必须：
- 锁每个参考仓库 commit SHA；
- 保存 `SOURCE_AUDIT_LOCK.json`（repo/url/commit/license/files-reviewed）；
- 只 vendor 许可证允许且决定直接依赖的代码；
- 对 GPL/AGPL 项目只保存来源/测试样本说明，不复制实现进 core；
- 把本报告中每条源码结论转换成 automated golden test；
- 每次升级 libass/OxideAV/provider 都跑 differential corpus 后再更新 lock。

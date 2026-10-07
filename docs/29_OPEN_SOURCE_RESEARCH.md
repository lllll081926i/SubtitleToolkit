# 开源项目与成熟工具研究（V3 取舍版）

## Subtitle Edit 5.2
值得吸收：批处理 step chain、格式探测、error list、同名视频辅助、批量 folder drag/drop、PGS/OCR 处理思想。  
不吸收：逐句编辑、波形、ASR/TTS/翻译等。  
来源：https://github.com/SubtitleEdit/subtitleedit

## Aegisub / maintained forks
参考 ASS script properties、PlayRes、CSRI renderer 生态和兼容 corpus；不做其编辑 UI。

## libass
默认 ASS renderer，ISC；作为 compatibility baseline。
来源：https://github.com/libass/libass

## VSFilterMod / xy-VSFilter modern forks
特效字幕关键。VSFilterMod 提供 TextSubMod；2026 xy-VSFilter 分支已支持 external CSRI backend，因此 V3 把 CSRI 调到优先 compatibility adapter。
来源：https://github.com/sorayuki/VSFilterMod / https://github.com/Masaiki/xy-VSFilter/releases

## ass2bdnxml
参考 ASS→BDN、HFR downsample、HDR dim、分辨率输出；其文档明确 2160p 非 Blu-ray compliant。
来源：https://github.com/cubicibo/ass2bdnxml/releases

## SUPer
Advanced PGS author 的重要行为参考：decoder buffer、palette/object reuse、ahead buffering、layout/bitrate 等。不要把代码直接拷入许可不兼容 core。
来源：https://github.com/cubicibo/SUPer

## BDSup2Sub / SupMover
参考 Fast Path：时间、位置、FPS、Forced、palette；以及位图 resize/re-encode 的边界。

## OxideAV subtitle crates
纯 Rust parser/encoder 候选，适合 bake-off 与 differential test；成熟度必须通过真实 corpus 决定。

## assfonts (Rust)
字体 indexing/matching/subset/embed 的优质参考。
来源：https://github.com/kotorimiku/assfonts

## ALASS
Rust 自动同步算法；GPL 隔离，作为外部 engine 候选，不进入 MVP hard dependency。

## FFmpeg/ffprobe
媒体 metadata adapter；不把 FFmpeg 变成视频处理核心。
来源：https://ffmpeg.org/

## MKVToolNix
容器 helper 的成熟后端；不重造 Matroska。
来源：https://mkvtoolnix.download/doc/mkvmerge.html

## zhhz/OpenCC
简繁/地区词汇转换作为可选自动 transform。
来源：https://github.com/ljh-sh/zhhz

## Iced
Windows-only Rust GUI 首选。
来源：https://github.com/iced-rs/iced/blob/master/CHANGELOG.md

## 逐项功能吸收矩阵

### BDSup2Sub / BDSup2Sub++
成熟功能值得吸收：resolution conversion、move、source/target FPS、global forced flags、free scaling、delay、minimum duration。它把“改变帧率”和“只用目标帧率对齐帧时间”分开，这个概念与 V3 的 Speed Conversion / Frame Snap 分层一致。

决策：
- 复刻功能语义，不复刻旧 Java/C++ UI；
- 单条字幕编辑、单帧可视编辑不进入产品；
- SUP 变换尽量 Fast Path。

参考：https://github.com/amichaelt/BDSup2SubPlusPlus/blob/master/src/help.htm

### SupMover
当前 CLI 已证明 PGS 结构层能高效完成 `--delay`、`--resync`、`--move`、`--crop`、`--setforced`、`--tonemap`，并明确说明 move/crop 某些路径不改变 image data。

决策：
- Fast Path 功能全部纳入；
- 增加 ODS hash invariant 测试；
- 它的 tonemap 对彩色字幕有限制，因此我们的 palette transform 必须支持完整 Y/Cr/Cb 颜色而不能只按黑白假设。

参考：https://github.com/MonoS/SupMover

### ALASS
能自动修正 constant offset、不同 framerate、广告/导演剪辑导致的 split，且有 Windows 64-bit executable；GPL-3.0。

决策：
- Smart Sync 的优秀参考；
- 不作为核心 hard dependency；
- 可以允许用户配置外部 alass.exe adapter；
- GUI 必须先展示拟合结果，再应用到副本。

参考：https://github.com/kaegi/alass

### FFsubsync
MIT，能用视频/音频或正确字幕作 reference，是 ALASS 之外的另一条同步路线。

决策：
- 后期 Smart Sync 可以提供多 engine；
- 若采用 Python CLI 会增加 runtime，因此首版不打包；
- 其 VAD/reference subtitle 思路可用于评估自研或轻量 adapter。

参考：https://github.com/smacke/ffsubsync

### OxideAV subtitle / ASS / sub-image
2026 年项目已经覆盖大量纯 Rust 文本字幕格式；`oxideav-ass` 提供 ASS/SSA parse/write；`oxideav-sub-image` 提供 PGS decode+encode、DVB/VobSub decode。PGS encoder 会按透明 bbox 裁对象，保留全 canvas composition，并对大 ODS 做分段。

决策：
- Phase 0 必须把 OxideAV 放进 parser/PGS bake-off；
- 适合作为基础 parser/basic encoder 候选；
- 不能直接把“每 frame 一个 display set”的 basic encoder 宣称为严格 Blu-ray author；
- bbox 裁剪逻辑与我们的“完整16:9逻辑画布 + 局部 object”规则相容。

参考：
- https://github.com/OxideAV/oxideav-subtitle
- https://github.com/OxideAV/oxideav-ass
- https://github.com/OxideAV/oxideav-sub-image

### ass-rs
MIT、模块化 Rust ASS 实现，但社区规模仍小。

决策：
- 和 oxideav-ass、内部 minimal parser 三方 bake-off；
- 核心评价指标不是 API 好看，而是 byte-preserving round-trip、未知 tags、附件、V4/V4+、畸形文件容忍与 fuzz。

参考：https://github.com/wiedymi/ass-rs

### Subtitle Composer
其功能面证明图形字幕 OCR、容器内字幕提取、编码检测、时间 scaling/shift/framerate 都是成熟需求；但它是完整编辑器。

决策：
- 吸收 OCR 前处理、demux、encoding detection、sync transform 思路；
- 不吸收 waveform/editor/scripting。

参考：https://github.com/maxrd2/SubtitleComposer

### Subtitle Workshop
其历史功能面证明“简单 UI + 大量小型自动处理”对于个人工具很有效。

决策：
- 工具页保持单一任务；
- 不复制其逐句 editing 模式。

参考：https://github.com/dekked/subtitleworkshop

## 当前最值得直接验证的 Rust 复用候选

| 领域 | 候选 | Phase 0 验证 | 通过标准 |
|---|---|---|---|
| ASS | oxideav-ass | 100+ corpus roundtrip | 无未知字段丢失；关键 tag 可重写 |
| ASS | ass-rs | 同上 | 同上 |
| Text formats | oxideav-subtitle | SRT/VTT/TTML samples | roundtrip + encoding policy |
| PGS | oxideav-sub-image | 第三方 SUP decode | 与参考 decoder frame/hash 一致 |
| PGS basic encode | oxideav-sub-image | encode→decode | 结构正确 + player smoke |
| Fonts | assfonts 思路/代码 | Windows fonts + TTC | family/face/glyph 准确 |
| Chinese | zhhz | phrase corpus | 与 OpenCC profile 基准一致 |

## 不直接加入当前核心的工具

- Subtitle Edit 源码：功能太大，许可证/架构与目标不同，只做行为基准。
- SUPer：Advanced PGS 的重要研究对象，但默认不嵌入核心。
- VSFilterMod：用户/可选 provider；renderer-host 隔离。
- VapourSynth/AviSynth+：legacy compatibility runtime，不让普通用户必装。
- FFsubsync：后期 engine，避免首版 Python runtime。


# 源码级复核（2026-10-05）

## libass `libass/ass.h`
直接读 API header 后确认：
- renderer 在使用前至少要设置 storage size、frame size 和 fonts；
- `ass_set_storage_size()` 影响 3D transform、source aspect 和 blur scale，缺失时会猜测；
- 合法 `LayoutRes*` 会优先影响相关 source/PAR 计算；
- ASS_Image 是必须按顺序 compositing 的链表；
- Windows `ass_read_file()` 有特殊 filename encoding 约束，因此本项目改为 Rust 读 bytes + `ass_read_memory()`；字体同理用 `ass_add_font()`。

**规划变化**：将 OutputCanvas、ScriptCoordinateSpace、LayoutRes、StorageSize 分离；`09` 与 `12` 已修正。

## Masaiki/ass2bdnxml 源码
直接读 `ass2bdnxml.c` / `sup.c`：
- 当前 fork 支持一个输出名写 XML、另一个写 SUP/PGS，并有 `--sup-high-precision`；
- 构建 unified render samples，SUP 可加入 ASS event exact boundaries，XML 仍消费 frame samples；
- 同一次 render loop 同时服务 XML/SUP；
- XML 变化检测比较实际 visible image，而不是只依赖 libass changed flag；
- `sup.c` 正确写 `Y, Cr, Cb, T` palette；
- 大 ODS 会做 segment fragmentation；
- composition/window/palette/object 有真实 state，不是“一帧一个完全独立包”的玩具 writer。

**规划变化**：新增 Render Timeline Planner、sink-specific sampling、visible frame hash、ODS fragmentation golden vectors。

## SupMover `main.cpp`
源码确认：
- `--resync/--delay` 修改 raw PGS segment header 的 PTS；
- tonemap 只修改 PDS luminance Y；
- move/crop/forced 可以在大量场景不触碰 ODS；
- 这证明 Fast Path 的价值，也暴露严格模式不能照搬“只改 PTS”的风险。

**规划变化**：Fast Path 加 ODS hash invariant；Strict time transform 要审查 DTS/decode schedule；HDR 分 Luma Scalar 与 Color-managed 两种。

## kotorimiku/assfonts
源码确认：
- ASS scanner 支持动态 Format map；
- 跟踪 `\fn`、`\b`、`\i`、`\r` 状态；
- subset 当前从 cmap 收集 glyph id 并调用 allsorts Minimal profile。

**规划变化**：字体 scanner 可参考，但 Windows resolver 仍由 DirectWrite 做权威；subset 必须对 GSUB/GPOS/复杂脚本做渲染等价验证，失败回退 full font。

## ALASS `alass-core`
源码确认：
- `align_nosplit()` 是单一 constant shift；
- `align()` 支持 split alignment；
- `overlap_scoring()` 明确适合 scaled/FPS-corrected subtitle；
- 核心处理 TimeSpan，视频 reference 的音频/VAD 属 CLI 层，并依赖 ffmpeg/ffprobe。

**规划变化**：Smart Sync UI 应把“constant / scaled / segmented”诊断展示给用户；仍保持 P2 外部 GPL adapter。

## SUPer README/行为层
进一步核对：Maximum bitrate 参数是测试输出，不主动 shape stream；而 compression/acquisition/normal/ahead/layout/redraw 等才会改变输出策略。

**规划变化**：Advanced Author 中拆分 `PeakBitrateValidator` 与未来 `BitrateShapingPolicy`。

## CSRI / VSFilterMod
2026 xy-VSFilter release 明确 external backend 使用标准 `csri_stream_ext` + BGRA，并指出特定 VSFilterMod r5.3.0 可作为兼容 CSRI provider。历史/其他 VSFilterMod 则可能只暴露 VapourSynth/AviSynth/DirectShow 接口。

**规划变化**：不再写“CSRI/VSFilterMod”是同一种接口；provider discovery 必须实际 probe。

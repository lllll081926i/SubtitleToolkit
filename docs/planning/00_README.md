# SubtitleToolkit-RS：产品与工程规划总纲（Windows-only）

> 文档审计日期：2026-10-05\
> V3.1 源码级复核：2026-10-05\
> 2026-10-07 复核：原始阶段顺序和状态表已调整；当前导航见 [`../README.md`](../README.md)，当前进度见 [`../STATUS.md`](../STATUS.md)。
> 2026-10-08 修订：新增文档审查建议按[采纳记录](AUDIT_ADOPTION_20261008.md)进入维护规格；原始 V3.1 内容保留在[导入基线](../subtitle_tool_rust_planning_v3_1/00_README.md)。
> 平台：**Windows x64 为唯一正式目标**；Windows 11 主验收、Windows 10 兼容目标；x86 helper 为后续可选能力。\
> GUI：**Rust + Iced 0.14**。\
> 产品定位：个人使用、面向具备字幕基础知识的用户，以**手动选择参数为首选**，视频自动探测/自动同步为辅助能力。\
> 非目标：不做字幕创作、不做逐句编辑、不做波形打轴、不做 Aegisub/Subtitle Edit 的编辑器替代品。

## 1. 一句话定位

一个 Windows 原生的强力字幕**转换、适配、批处理、特效渲染与 Blu-ray/PGS 工具箱**：用户已有字幕，通过明确的源/目标参数快速得到 ASS/SRT/BDN/SUP 等成品，并支持 HDR 亮度适配、帧率/时长变换、多分辨率多输出、特效滤镜、SUP 调整、字体与 MKV 辅助。

## 2. V3 的核心原则

1. **Manual-first**：懂参数的用户直接选源 FPS、目标 FPS、分辨率、HDR、格式、渲染器；软件不替用户“猜”。
2. **Assist-second**：目标视频可以拖入/选择，用于自动填充 FPS、分辨率、HDR metadata、时长；自动同步是单独的辅助工具。
3. **一份输入，多份输出**：一次任务可同时生成 720p/1080p/2160p ASS/BDN、自定义 SUP、严格 Blu-ray SUP 等多个 Profile。
4. **特效字幕固定完整 16:9 逻辑画布**：1280×720、1920×1080、3840×2160 等。内容 bounding box 只能作为压缩/对象裁剪，不能改变逻辑坐标系。
5. **渲染器分层**：libass 默认；CSRI/VSFilterMod 特效兼容优先；VapourSynth/AviSynth+ 仅作为 legacy adapter。
6. **格式正确优先于“能打开”**：PGS/BDN 必须有二进制 validator、round-trip 和真实播放器 corpus。
7. **Fast Path 优先**：能只改结构就不重编码位图；能共享渲染缓存就不重复渲染。
8. **UI 每个开关必须真实接通**：禁止“有按钮但无效果”。旧 AssRocket 审计中的空操作是明确反例。

## 3. 当前文档入口

原始编号规格保留在 `../specs/`；阶段、WBS 和实施清单位于本目录；架构决策位于 `../decisions/`；源码研究位于 `../research/`；历史状态和审查记录位于 `../history/`。路径导航和当前状态以 `../README.md` 为准。

## 4. 开发起点

旧序列把尚未裁决的 CSRI/x86/MKV 和 GUI 原型设成核心链的前置条件，已由 [`26_DEVELOPMENT_PHASES.md`](26_DEVELOPMENT_PHASES.md) 修订。当前先收口现有 CLI、渲染隔离、输出事务和回归测试；PGS 互操作、GUI 与可选适配器分别按证据门槛推进。

## 5. 外部研究与实施证据

上游版本、接口和许可研究集中维护在[来源清单](../research/30_RESEARCH_SOURCES.md)与[第三方来源锁](../../third_party/SOURCE_AUDIT_LOCK.json)，总纲不再维护另一份“最新版本”列表。

Iced 0.14 已落地至 `crates/gui`，任务界面调用真实 CLI batch 引擎。CSRI 导出存在只支持接口候选判断，不证明 DLL 可以加载或像素兼容。2160p custom 输出不宣称 Blu-ray 合规，实际 Profile 与设备结论见 [PGS 规格](../specs/13_PGS_BDN_BLURAY.md)。

## 6. 追加规格

UI 状态与交互、preset schema、性能容量、失败恢复和验收清单分别见 `../specs/35_*.md`、`../specs/36_*.md`、`../specs/37_*.md`、`../specs/38_*.md` 与 [`39_IMPLEMENTATION_CHECKLIST.md`](39_IMPLEMENTATION_CHECKLIST.md)。这些文件定义目标和验收条件，不代表功能已经交付。


## 7. 历史研究

V3.1 源码研究和 changelog 已迁入 `../research/` 与 `../history/`。它们记录当时来源和结论；依赖版本、ABI 与播放器行为在实施前仍需重新核验。

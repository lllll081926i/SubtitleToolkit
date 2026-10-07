# SubtitleToolkit-RS V3.1：Source-Audited 产品与工程规划（Windows-only）

> 文档审计日期：2026-10-05  
> V3.1 源码级复核：2026-10-05  
> 平台：**Windows 10/11 x64 首发与唯一正式平台**；仅为兼容旧式字幕滤镜提供可选 x86 helper。  
> GUI：**Rust + Iced 0.14**。  
> 产品定位：个人使用、面向具备字幕基础知识的用户，以**手动选择参数为首选**，视频自动探测/自动同步为辅助能力。  
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

## 3. 最终文档顺序

文档按“产品 → UI → 技术 → 格式/算法 → 工具链 → 质量 → 开发执行 → 调研/审计”的依赖顺序排列：

- `01_PRODUCT_VISION_SCOPE.md`：目标、边界、用户和成功标准。
- `02_USER_WORKFLOWS.md`：手动优先的主要工作流。
- `03_FEATURE_MATRIX.md`：全部功能、阶段、依赖、输出。
- `04_UI_INFORMATION_ARCHITECTURE.md`：导航和信息架构。
- `05_UI_SCREEN_SPEC.md`：每个页面、控件、状态、交互细节。
- `06_TECHNOLOGY_DECISIONS_WINDOWS.md`：Windows-only 技术选型。
- `07_SYSTEM_ARCHITECTURE.md`：进程、crate、接口、数据流。
- `08_FORMATS_AND_IR.md`：格式与内部 IR。
- `09_CANVAS_RESOLUTION_MULTI_OUTPUT.md`：16:9、分辨率缩放与多输出。
- `10_TIME_FPS_SYNC.md`：时间/FPS/时长/同步。
- `11_HDR_COLOR.md`：ASS/SUP HDR 亮度与颜色策略。
- `12_RENDERER_FILTERS_WINDOWS.md`：libass/CSRI/VSFilterMod/VapourSynth/AviSynth+。
- `13_PGS_BDN_BLURAY.md`：PGS/BDN/大金兼容。
- `14_CONVERSION_PIPELINES.md`：端到端流水线。
- `15_BATCH_TASK_ENGINE.md`：批处理、队列、缓存、多输出。
- `16_MEDIA_PROBE_SMART_ASSIST.md`：目标视频辅助，而非主流程。
- `17_FONT_MANAGEMENT.md`：字体扫描、缺失、subset、附件。
- `18_CONTAINER_HELPER.md`：MKV 字幕轨/附件辅助。
- `19_OCR_OPTIONAL.md`：位图→文本 OCR，后期可选。
- `20_CLI_AUTOMATION.md`：CLI、preset、headless。
- `21_DEPENDENCIES_LICENSES.md`：依赖与许可证。
- `22_LOGGING_DIAGNOSTICS.md`：日志、诊断包、错误码。
- `23_SECURITY_ROBUSTNESS.md`：不可信文件、DLL、路径安全。
- `24_TEST_QA_CORPUS.md`：测试体系与语料库。
- `25_ACCEPTANCE_TESTS.md`：正式验收用例。
- `26_DEVELOPMENT_PHASES.md`：阶段与里程碑。
- `27_WBS_DEPENDENCIES.md`：工作分解与依赖 DAG。
- `28_WINDOWS_BUILD_RELEASE.md`：构建、安装、签名、更新。
- `29_OPEN_SOURCE_RESEARCH.md`：开源项目参考与复用策略。
- `30_RESEARCH_SOURCES.md`：联网核验来源。
- `31_AUDIT_REPORT.md`：V2→V3 的全量审计与修正。
- `32_ADR_DECISIONS.md`：架构决策记录。
- `33_GLOSSARY.md`：术语。
- `34_V3_CHANGELOG.md`：V3 变更。

## 4. 开发起点

开发不得从 GUI 开始堆按钮。顺序必须是：

`格式探测/时间类型 → ASS AST round-trip → SRT → HDR/FPS transforms → CLI golden tests → Iced shell → libass renderer → multi-output cache → BDN → PGS parser/fast-path → PGS basic encoder → CSRI/VSFilterMod host → strict Blu-ray author/validator`。

## 5. 关键联网核验结论

- Iced 0.14 增加 hot reload、time-travel debugging、headless/E2E、table/grid，适合本项目长期调试：https://github.com/iced-rs/iced/blob/master/CHANGELOG.md
- libass 最新稳定 0.17.5（2026-06-24），ISC：https://github.com/libass/libass
- VSFilterMod 提供 `TextSubMod`，x86/x64 都存在；现代 xy-VSFilter 分支已经支持外部 CSRI renderer：https://github.com/sorayuki/VSFilterMod / https://github.com/Masaiki/xy-VSFilter/releases
- VapourSynth 当前文档 R80，VSScript4 应按正式 API 使用，禁止旧工程硬编码函数表：https://www.vapoursynth.com/doc/
- ass2bdnxml 明确 2160p 只属于 exotic output，**不符合 Blu-ray；UHD BD 的 PGS 不能超过 2K**：https://github.com/cubicibo/ass2bdnxml/releases

## 6. 追加实施文档
- `35_UI_STATE_INTERACTION.md`：页面状态机、ValueOrigin、拖放路由与 Dev Panel。
- `36_PRESET_CONFIG_SCHEMA.md`：Preset/JobSnapshot/Plan 配置模型。
- `37_PERFORMANCE_CAPACITY.md`：4K 内存、缓存、并发与 benchmark。
- `38_FAILURE_RECOVERY.md`：PartialSuccess、renderer crash、断点与取消。
- `39_IMPLEMENTATION_CHECKLIST.md`：正式开发逐项检查表。


## 7. V3.1 源码级审计追加
- `40_SOURCE_CODE_AUDIT.md`：libass/ass2bdnxml/SupMover/assfonts/ALASS/SUPer/OxideAV/CSRI 的源码级复核与架构修正。
- `41_V3_1_CHANGELOG.md`：V3.1 变更。

V3.1 已同步修订 `09/10/12/13/17/21/24/29/31/39`，这些文件优先于 V3 的旧结论。

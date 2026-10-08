# V2 → V3 全量审计报告

## 1. 结论
V2 的产品方向基本正确，但存在六类需要修正的问题：平台范围过宽、GUI 选型不再最优、Smart Adapt 位置过重、滤镜依赖链过重、许可证判断需要更新、4K/PGS 表述需要收紧。V3 已全部修正。

## 2. 文档顺序
V2 后期增量追加导致：Smart Adapt、feature review 放在 25/26，实际却反过来影响 PRD/UI/WBS；开发阅读会来回跳。  
V3 重新按依赖顺序排列：产品→用户流→功能→UI→技术→格式/算法→批处理/辅助→依赖/质量→开发→调研/审计。

## 3. 平台修正
V2 多处写“Windows 首发、核心跨平台”。现已按用户要求改为 Windows-only；删除 Linux/macOS CI、Fontconfig/CoreText、跨平台 renderer 约束。x86 只作为旧滤镜 helper。

## 4. GUI 修正
V2：Slint 首选。  
V3：Iced 0.14 首选。原因：纯 Rust、0.14 的 hot reload/time-travel/headless E2E/table/grid，以及文件拖放工作流更直接。Slint 不再是硬依赖。

## 5. Smart Adapt 修正
V2 把“字幕+视频 Smart Adapt”推到首页 Hero。  
用户明确目标用户有基础知识，因此 V3 改为 Manual-first：手动参数为主；视频 probe 是辅助填表；自动同步是独立工具。

## 6. 16:9 画布修正
V3 新增硬规则：所有特效字幕按完整 16:9 逻辑画布；bbox 只能裁对象。所有 resolution transform、renderer、BDN/PGS、多输出、测试共同遵守。

## 7. 4K/PGS 修正
V2 部分文档容易让“目标视频 4K”与“PGS 4K”混淆。  
V3 明确：2160p ASS/自定义 BDN 可做；严格 Blu-ray/UHD-BD PGS 不等于 3840×2160 bitmap。ass2bdnxml 当前文档也明确 2160p 非 Blu-ray compliant。

## 8. Renderer 修正
V2：libass + VSFilterMod，重点讨论 VapourSynth/AviSynth host。  
V3：libass → CSRI/VSFilterMod → VS/AviSynth legacy。现代 xy-VSFilter 已支持外部 CSRI backend，可减少 Python/runtime 依赖。

## 9. VapourSynth API 修正
旧审计里的函数表硬编码被正式禁止。当前 VapourSynth 文档 R80/VSScript4，adapter 必须使用正式 API 和版本探测。

## 10. License 修正
V2 把 libimagequant 作为优先量化候选不合适。当前 4.x 是 GPL-3.0-or-later / commercial 双许可；V3 从默认 core 移除。

## 11. Release 修正
V3 Windows-only 推荐 Inno Setup + Portable ZIP；MSIX 后置。正式发行明确加入 Authenticode/SmartScreen 策略。

## 12. 产品边界复核
仍然不加入字幕编辑器、波形、ASR/TTS/翻译。新加功能必须满足“已有字幕→自动/参数化处理→输出”的定位。

## 13. 旧 AssRocket 反例继续保留
- UI 选项必须真实接通；
- PGS 不可自创字段布局；
- 事件不能只渲染起始帧；
- 测试必须真正走生产 pipeline；
- 外部 DLL 不应硬编码 ABI；
- 中文/长路径必须原生支持。


# 14. V3 → V3.1 源码级审计修正

本轮不是只看 README，而是直接读取以下核心源码/API header：libass `ass.h`、Masaiki ass2bdnxml `ass2bdnxml.c/sup.c`、SupMover `main.cpp`、assfonts Rust `ass.rs/subset.rs`、ALASS `alass-core/src/lib.rs`；OxideAV/SUPer/xy-VSFilter 结合源码树与当前项目文档/Release 交叉核验。

修正项：
1. **ASS 分辨率模型**：旧 V3 “1080→2160 全部 ×2”表述过于粗；新增 LayoutRes/StorageSize/OutputCanvas 分层。
2. **libass 初始化**：明确 storage_size/frame_size/fonts 为硬要求，并改内存输入策略避免 Windows filename encoding 坑。
3. **多输出渲染**：采用统一 Render Timeline Planner，借鉴当前 ass2bdnxml 的 frame samples + exact event boundaries 思路。
4. **变化检测**：`detect_change` 降为 hint；用最终 visible RGBA hash/diff 决定 BDN/SUP event 合并。
5. **CSRI/VSFilterMod**：拆成 provider ABI 与 legacy DLL adapter；实际 capability probe 后才能选择。
6. **PGS Fast Path**：增加 ODS invariant 与 DTS/decode scheduling 验证；不能把只改 PTS 的播放器向实现当 Strict authoring。
7. **PGS Basic Encoder**：补 ODS fragmentation、erase display set、stateful palette/object/window、full-canvas vs tight-object 规则。
8. **PGS Strict**：最大码率“测量”与真正“shape stream”拆开。
9. **字体 subset**：把 assfonts 从“成熟可直接依赖”降为候选；增加 GSUB/GPOS/variable/TTC/CFF/复杂脚本渲染等价 gate。
10. **许可证**：SupMover 明确 AGPL-3.0；ALASS GPL-3.0；GPL/AGPL 外部进程仍要发布前做法律/许可复核。

结论：V3 产品方向不变，但 V3.1 在 renderer、ASS coordinate model、PGS strict、font subset 上更保守、更接近真实源码语义。

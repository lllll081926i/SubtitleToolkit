# SubtitleToolkit-RS V3.1 全文档静态审计结论

审计日期：2026-10-08。审计对象：V3.1 Source-Audited 基线 `subtitle_tool_rust_planning_v3_1/`，共 **42 份 Markdown，2472 行**。问题分级：P0（开工/产品正确性阻断）**10** 项，P1（正式测试/发布前必须闭环）**22** 项，P2（可放在后续迭代）**6** 项。

**性质声明**：这是对已有开发文档的逐文件静态一致性/完整性审计，并对关键公开规范和开源仓库做联网交叉验证；**没有运行待开发产品、没有完成Windows真实渲染器/蓝光实机测试、没有将所有第三方仓库固定到commit复验**。本审计不能替代运行时产品质量认证。

## 可保留的总体方案

- Windows-only；Rust桌面 GUI；手动参数优先、媒体探测辅助；不做字幕编辑器。
- 正确区分 `ASS AST`、`Text IR`、`PGS DisplaySet IR`；保留未知扩展。
- ASS原始PlayRes与完整16:9目标画布分层；高级Rewrite而非强制改写。
- 外部特效DLL放renderer-host，CSRI与legacy接口探测分开。
- PGS Basic、Advanced、Strict分级，不能把能生成SUP当大金认证。
- 不默认接入未经许可证审计的GPL/AGPL组件。

## 审计结论及最重要决策

1. **文档能指导预研，却还不能无歧义指导生产开发。** 19份文档不足40行，诸如HDR(37行)、时间/FPS(37行)、批处理(36行)、MKV(20行)、CLI(24行)需要算法和失败分支规格。
2. **先闭环 P0，再并行GUI和纯Rust核心。** P0并非代表已发现现成程序Bug，而是设计实现中高概率产生返工、错误输出或失真。
3. **一次输入多输出必须有 RenderGroupKey。** 同组共享采样及像素帧，不同分辨率/颜色/渲染器仅共享AST/可证明等价的中间层。
4. **“203 nit”是HDR参考白概念，不是ASS可保证的亮度。** 映射算法/播放器显示链必须写清。
5. **Strict报告应展示“本工具的验证级别”，而不是未经认证的“所有蓝光机保证兼容”。**
6. **开发顺序依赖须重新排 DAG。** 验证器先可独立开发，Advanced Author 依赖验证器，而非反过来。

## 开发可开工/不可开工的分界

现在可以启动：Windows Iced file-drop PoC、RationalTime、ASS AST bake-off、libass memory-input wrapper、FFprobe/MKV adapters PoC、PGS parser corpus 收集。

暂不应声称已达到生产完成：HDR nit 映射、Strict Blu-ray、跨Profile共享渲染缓存、第三方特效DLL通用自动兼容、PGS“完全无损”、字体subset可公开分发。

详情：`01_ISSUES.md`（逐项修正）、`02_ALL_DOCUMENTS.md`（42份逐文件结论）、`03_IMPLEMENTATION_CONTRACTS.md`（关键接口/算法）、`04_WBS_REBUILD.md`（修订开发顺序）、`05_VERIFIED_SOURCES.md`（联网资料与核实边界）、`06_REMEDIATION_GATES.md`（审计整改过门清单）。

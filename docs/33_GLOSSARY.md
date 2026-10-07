# 术语表

- ASS/SSA：Advanced/SubStation Alpha 文本+样式字幕。
- PlayRes：ASS 逻辑画布；本项目特效字幕按完整16:9解释。
- PGS/SUP：Blu-ray Presentation Graphics 位图字幕流。
- BDN：Blu-ray Disc Native XML+PNG 字幕交换形式。
- Display Set：PGS 中组成一次显示状态的一组 segments。
- PCS/WDS/PDS/ODS：PGS composition/window/palette/object segments。
- Epoch：PGS decoder 状态/资源生命周期分区概念。
- Fast Path：不重新解码/量化位图，只改结构字段的处理路径。
- CSRI：Common Subtitle Renderer Interface，外部字幕 renderer 接口。
- VSFilterMod：带扩展 ASS 特效的 VSFilter 系 renderer。
- libass：主流 ASS renderer。
- HDR Adapt：对字幕颜色/亮度预调整，不是给 ASS 添加 HDR metadata。
- Speed Conversion：因片源播放速度变化而整体缩放时间轴。
- Frame Snap：总时长不变，只对齐帧边界。
- Profile：一个输出目标的完整参数集合。
- Plan：执行前生成的可重放任务 DAG。

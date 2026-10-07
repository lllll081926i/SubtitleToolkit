# 用户工作流（Manual-first）

## 1. 基础转换
1. 拖入/选择一个或多个字幕文件。
2. 选择目标格式。
3. 若转换有损，界面显示“将丢失：样式/位置/绘图/动画…”摘要。
4. 选择输出目录/命名。
5. 加入任务或立即运行。

## 2. ASS HDR 适配
1. 选择 ASS。
2. 手动选择目标：HDR10/PQ 使用场景。
3. 选择亮度 preset：Compatibility / 203 nit / 百分比 / 自定义曲线。
4. 选择是否处理 inline override colors 和 VSFilterMod 扩展颜色。
5. 可同时添加 1080p HDR ASS、2160p HDR ASS 两个 output profiles。
6. 输出前执行 AST round-trip 与视觉 smoke test。

## 3. 帧率/时长适配
1. 明确输入“字幕对应源版本 FPS”。
2. 明确“目标版本 FPS”。
3. 选择模式：
   - Speed/Duration Scale；
   - Snap to frame only；
   - Offset only；
   - Two-point linear fit。
4. UI 显示倍率和影片末尾预计差异。
5. 23.976↔24 使用精确 24000/1001，不使用 23.976 浮点近似。

## 4. ASS → 多输出
一次输入一个 ASS，可定义 Profile 表：

| Profile | 画布 | FPS | HDR | Format | Renderer | Strict |
|---|---:|---:|---|---|---|---|
| Web-1080 | 1920×1080 | 23.976 | SDR | ASS | none | - |
| HDR-4K | 3840×2160 | 23.976 | 203nit | ASS | none | - |
| BD-SUP | 1920×1080 | 23.976 | SDR | SUP | VSFilterMod | Blu-ray |
| UHD-BD-SUP | 1920×1080 | 23.976 | HDR dim | SUP | VSFilterMod | UHD-BD |
| Archive-SRT | text | same | n/a | SRT | none | - |

注意：严格 UHD Blu-ray PGS 仍是 2K 级画布；“2160p SUP”只能标记为 non-standard/custom，不得标“UHD Blu-ray compliant”。

## 5. SUP 快速调整
1. 输入 SUP。
2. 解析并显示 resolution/FPS estimate/display-set 数量/forced。
3. 可选：offset、time scale、X/Y、crop、Forced、palette brightness。
4. 若仅结构层可实现，显示“无重编码 Fast Path”。
5. 若要求缩放 bitmap，明确提示“需要重编码图像”。

## 6. 蓝光兼容化
- Basic：结构合法、segment/length/PTS 单调检查。
- Standard：decoder buffer、显示集、object/palette 生命周期检查。
- Strict：按已验证模型优化 Epoch、buffer、ahead decode；输出 compatibility report。
- 只有 Strict 通过才允许显示“蓝光严格兼容”；开发早期不得用“大金过水完成”作为营销文本。

## 7. 目标视频辅助（备选）
手动参数页右上角提供“从视频读取参数”：
- 读取分辨率、r_frame_rate/avg_frame_rate、时长、primaries/transfer/matrix、HDR side data；
- 只填充候选值；
- 每个填充值旁显示来源（video metadata）；
- 用户可锁定字段防止覆盖。

## 8. 自动同步（备选）
单独工具页，不嵌入默认转换：
- Subtitle vs correct subtitle；
- Subtitle vs target video/audio；
- 先报告：offset / speed ratio / segmented fit / confidence；
- 用户点击“应用”后才改变时间。

## 9. 批处理
支持：
- 文件/文件夹拖入；
- 递归；
- 规则匹配；
- 同名视频自动关联但默认关闭；
- 每输入文件 N 个输出 profile；
- 失败后继续；
- 任务重试与断点恢复。

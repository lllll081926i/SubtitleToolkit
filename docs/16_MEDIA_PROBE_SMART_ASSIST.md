# 目标视频辅助与 Smart Assist（备选能力）

## 1. 产品位置
不是首页唯一入口，不自动主导专业用户；作为：
- 首页辅助卡；
- 各参数页“从视频读取”；
- 批量同名视频关联选项。

## 2. ffprobe 字段
- width/height；
- avg_frame_rate/r_frame_rate；
- duration/time_base；
- color_primaries/color_transfer/colorspace/color_range；
- HDR side data（如可用）；
- stream/container tags。

## 3. 规则
- 3840×2160 + smpte2084 → 建议 HDR preset；
- 目标 23.976 → 填目标 FPS，但不执行 speed transform；
- input ASS PlayRes 1920×1080 + target 3840×2160 → 建议添加 2160p ASS profile；
- 若目标是 UHD-BD SUP → 仍建议 strict 2K PGS profile，而不是 2160p PGS。

## 4. Confidence
每条 suggestion 显示 High/Medium/Low + evidence。Low 默认不勾选。

## 5. 安全
视频路径、ffprobe JSON、探测错误不能阻塞基础手动流程；ffprobe 不存在时整个软件仍可用。

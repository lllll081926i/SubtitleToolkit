# 时间、FPS、时长与同步

## 1. 三件事必须分开
1. Playback speed conversion：影片总时长变化，全部时间按比例缩放。
2. Frame snapping：影片总时长不变，只把 cue 边界对齐目标帧。
3. Offset：全部时间加减固定量。

## 2. 精确 FPS
预设存 rational：
- 24000/1001；24/1；25/1；30000/1001；30/1；50/1；60000/1001；60/1。

## 3. Speed conversion
`t_new = t_old * fps_source / fps_target`，但只有用户选择 Speed mode 才执行。

## 4. 两点线性拟合
输入 `(source t1 -> target t1)` 和 `(source t2 -> target t2)`，求 `a,b`：`t' = a*t+b`。适合不同 speed + offset。

## 5. Segmented sync
后期 Smart Sync 可以输出多段 piecewise mapping，用于广告/剪辑差异；默认工具页不自动启用。

## 6. PGS Fast Path：展示时间与解码调度要分开
源码审计 SupMover 发现，其 `--resync/--delay` 直接改 PGS segment header 的 PTS；这是非常实用的播放器向 Fast Path，但不能把这种实现直接等同于严格 authoring。

本项目规则：
- **Playback Fast Path**：允许对既有 SUP 做 presentation timeline 变换，但必须先解析 segment 类型、检查时间单调性、负值/32-bit wrap、display set 边界。
- **Authoring-aware Path**：若输入流具有独立 DTS / decode lead，缩放/偏移必须保持合法的 decode-before-display 关系；不能机械令 DTS=PTS，也不能只改 PTS 后仍声明 Strict。
- 任何 time scale 后运行 structural + timeline validator；若 decode scheduling 失效，则自动降级为重新 author 或拒绝 Strict 输出。
- 内部始终用有理数 + 90 kHz integer ticks，最后一步才舍入。

## 7. BDN
timecode writer 从统一 rational time 计算，禁止旧工程“秒字段与帧字段用不同舍入源”的错误。

## 8. 目标视频辅助
ffprobe 读取 FPS 只能作为候选；`avg_frame_rate != subtitle source fps` 不能直接推断 speed conversion。

## 9. VFR
MVP 不做“把 ASS 变 VFR frame-number subtitle”；时间型字幕本身仍是时间。只有 renderer 在需要 frame stepping 时读取 timecode map。

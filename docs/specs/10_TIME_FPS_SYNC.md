# 时间、FPS、时长与同步

文档修订：2026-10-08。当前使用 `core-model::time` 的有理数秒与精确 FPS；`transform-time` 负责模式校验。本文中的未来策略须通过能力检查后才能启用。

## 1. 三件事必须分开
1. Playback speed conversion：影片总时长变化，全部时间按比例缩放。
2. Frame snapping：影片总时长不变，只把 cue 边界对齐目标帧。
3. Offset：全部时间加减固定量。

## 2. 精确 FPS
预设存 rational：
- 24000/1001；24/1；25/1；30000/1001；30/1；50/1；60000/1001；60/1。

## 3. Speed conversion
`t_new = t_old * fps_source / fps_target`，但只有用户选择 Speed mode 才执行。

输入 `23.976` 按精确十进制分数保存，并不自动等于 `24000/1001`。需要 NTSC 分数帧率时显式使用 `24000/1001`，避免长片比例误差。

## 4. 两点线性拟合
输入 `(source t1 -> target t1)` 和 `(source t2 -> target t2)`，求 `a,b`：`t' = a*t+b`。适合不同 speed + offset。

`a = (target2-target1)/(source2-source1)`，`b = target1-a*source1`。拒绝源控制点重合、`a <= 0` 与算术溢出；不自动退回 Offset。当前配置还要求 target FPS，不能因为拟合公式未用 FPS 就宣称 CLI 无须该参数。

## 5. Segmented sync
后期 Smart Sync 可以输出多段 piecewise mapping，用于广告/剪辑差异；默认工具页不自动启用。

### 实现状态（2026-10-17）

已实现：`transform-time::PiecewiseMap` + CLI `time --piecewise 源,目标`（可重复）。

- **映射由调用方给出**：本层不做广告/剪辑点自动检测。自动检测需要内容比对
  （音频指纹/场景检测），是另一条研究线，错了会静默把字幕移到错处。
  这里只做确定性执行，与本节原话一致（重点是**输出**那个映射）。
- **控制点是绝对的 `(source, target)` 对**，相邻两点连成一段。
  于是①映射天然连续（无跳变）②每段由端点唯一确定 ③用户从播放器量到的观测可直接填。
- **控制点自动排序**：从播放器量点往往不是按时间顺序记的，为此报错没有意义。
- **段外用最近段的斜率外推**，不是钳到端点值（钳住会让片头/片尾整段字幕堆在同一时刻）；
  外推的条数会进 `LossReport` 的 `user_attention`——那是我们缺信息的地方。
- **分段没有单一倍率**：`preview::mapping_ratio` 对 Piecewise 直接报错，
  不编一个“代表值”（那会让界面显示一个不存在的数字）。

校验（每条都指认第几个点/段）：至少 2 个点；source 严格递增；每段斜率 > 0；
映射后不得出现负时间（拒绝而非钳零）。

仍然**不做**的：自动检测剪辑点；与视频帧做内容对齐；多段映射暂不写入可搬运 preset。TOML preset 已实现通用输出目标配置，但当前 schema 不包含 PiecewiseMap；见[Preset 配置规格](36_PRESET_CONFIG_SCHEMA.md)。

## 6. PGS Fast Path：展示时间与解码调度要分开
源码审计 SupMover 发现，其 `--resync/--delay` 直接改 PGS segment header 的 PTS；这是非常实用的播放器向 Fast Path，但不能把这种实现直接等同于严格 authoring。

本项目规则：
- **Playback Fast Path**：允许对既有 SUP 做 presentation timeline 变换，但必须先解析 segment 类型、检查时间单调性、负值/32-bit wrap、display set 边界。
- **Authoring-aware Path**：若输入流具有独立 DTS / decode lead，缩放/偏移必须保持合法的 decode-before-display 关系；不能机械令 DTS=PTS，也不能只改 PTS 后仍声明 Strict。
- 任何 time scale 后运行 structural + timeline validator；若 decode scheduling 失效，则自动降级为重新 author 或拒绝 Strict 输出。
- 内部保留有理数秒，在 PGS 边界转换为 90 kHz ticks。现有 `Rational` 分量为 `i64`，中间算术使用 `i128`；审查包建议的 `i128` 持久时间结构不是当前 API。

SUP 段头使用 32-bit PTS/DTS 字段。写出与快速变换必须检查字段范围，不能把通用时间模型的较宽上限当成 SUP 写出许可；当前不自动回绕溢出时间戳。

## 7. BDN
timecode writer 从统一 rational time 计算，禁止旧工程“秒字段与帧字段用不同舍入源”的错误。

## 8. 目标视频辅助
ffprobe 读取 FPS 只能作为候选；`avg_frame_rate != subtitle source fps` 不能直接推断 speed conversion。

## 9. VFR
MVP 不做“把 ASS 变 VFR frame-number subtitle”；时间型字幕本身仍是时间。只有 renderer 在需要 frame stepping 时读取 timecode map。

VFR 精确贴帧需要真实 frame timestamp map。没有该 map 时只能提供明确的恒定 FPS 网格或时间型变换，不以 `avg_frame_rate` 代表每帧时刻。

## 10. 舍入、负时间与量化

- FrameSnap 基元为 `round(t*fps)/fps`；舍入到最近整数帧，半值远离零。单个边界移动不超过半帧，cue 长度可能随边界量化变化；“不变速”不等于每条 cue 时长逐字不变。
- ASS 写出按百分之一秒，SRT 按毫秒量化；PGS 按 90 kHz tick 量化。量化前计算均保留有理数，误差在写出边界评估。
- BDN 从 `floor(t*fps_exact)` 得帧号，再按 nominal FPS 生成 timecode。不得混用墙上秒与另一套帧字段舍入。
- 当前变换拒绝产生负时间，不静默钳到零或删除 cue。Clamp/Drop 等策略尚未纳入当前配置。
- 零时长、倒序、最小显示时长和重叠分别由报告与输出验证处理；不把“变换成功”当成“所有目标格式均可写出”。

验收使用精确有理数比较、反向倍率与写出后时间解析。不得用浮点显示值相等证明时间无漂移，见 [AT-TIME-001/002](25_ACCEPTANCE_TESTS.md#时间与探测)。

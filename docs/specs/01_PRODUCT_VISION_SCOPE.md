# 产品愿景、范围与成功标准

本文描述产品目标。当前 CLI 能力与未实现格式/界面见[功能矩阵](03_FEATURE_MATRIX.md)。目标输入或输出不等于目前均可执行；Windows 版本支持按[选型与验收边界](06_TECHNOLOGY_DECISIONS_WINDOWS.md#2-平台)记录。

## 1. 产品愿景

目标不是“字幕编辑器”，而是让有现成字幕、懂基本字幕概念的用户，在一个 Windows 工具里完成过去需要多个程序、多个命令行和大量中间文件才能完成的转换与适配工作。

典型输入：ASS/SSA/SRT/SUP/BDN XML/IDX+SUB/MKV 中的字幕轨。  
典型输出：ASS/SRT/BDN XML+PNG/SUP、调整后的 SUP、带字幕与字体的新 MKV。

## 2. 核心用户

### 2.1 特效字幕使用者
- 已有 ASS；
- 需要 1080p/4K 适配、HDR 降亮度、不同帧率版本；
- 需要 VSFilterMod 特效正确渲染；
- 需要 ASS→BDN/SUP。

### 2.2 Blu-ray/UHD 收藏处理用户
- 已有 SUP/PGS；
- 调整延迟/FPS/位置/Forced/亮度；
- 需要蓝光兼容化、严格验证；
- 需要 MKV 轨道/字体附件辅助。

### 2.3 批量收藏整理用户
- 同一系列几十集；
- 一次生成 1080p ASS、2160p ASS、1080p SUP、SRT 等；
- 固定 preset 与目录命名；
- 希望失败隔离和断点恢复。

## 3. 绝对不做

- 逐句文本编辑器；
- 时间轴/波形手工打轴；
- ASS Style Manager、Visual Typesetting、ASS Draw；
- 卡拉 OK 模板创作；
- ASR/TTS/翻译/AI 改写；
- 视频编码器和烧录器；
- 完整 Blu-ray authoring；
- 完整 MKVToolNix 替代品。

允许的“文本处理”仅限自动 transform：编码规范化、简繁转换、重排、样式剥离 ASS→SRT、合并等。

## 4. Manual-first 产品原则

首页和所有专业页面都必须允许用户直接指定：
- 源格式与目标格式；
- 源 FPS / 目标 FPS；
- 源逻辑分辨率 / 目标逻辑分辨率；
- HDR 处理模式与强度；
- 渲染器；
- 输出 profile；
- 严格模式等级。

“读取目标视频”只负责**辅助填表与建议**。用户随时可覆盖；自动同步必须显式进入“同步助手”，不得因为用户选了一个 25fps 视频就自动改时间轴。

## 5. 成功标准

### 易用
- 80% 常见任务在 3 次主要点击内进入队列；
- 同一窗口可保存为 preset；
- 高级参数默认折叠，但永远可查。

### 强大
- 支持特效渲染、HDR、FPS、多输出、PGS fast path、Blu-ray strict、字体/MKV 辅助；
- 多个输出 profile 共享解析；渲染按 RenderGroupKey 分组，量化仅在输入与策略相同时共享。

### 可靠
- 原文件默认只读；
- 临时文件 + atomic rename；
- 每个输出有 verify 状态；
- “完成”必须意味着目标文件被重新读回并通过最低验证。

### 可诊断
- 每个任务都有 plan、日志、输入摘要、依赖版本；
- 外部 renderer 崩溃不拖死 GUI；
- 一键生成诊断包。

## 6. 优先级

P0：ASS/SRT、HDR、FPS、分辨率、多输出、批处理、libass、BDN。  
P1：PGS parser/fast path/basic encode、字体、MKV helper、CSRI/VSFilterMod。  
P2：Advanced PGS + strict Blu-ray、大金兼容、OCR。  
P3：小众专业格式与更复杂容器。

# 功能矩阵、阶段与依赖

“首发”表示产品目标，不表示已经交付。当前状态只记录代码面，不代替真实依赖或设备验收。

| 功能 | 目标优先级 | 当前状态 | 主要依赖与验收边界 |
|---|---|---|---|
| ASS/SSA parse + rewrite | P0 | CLI 已实现 | 未知字段/标签保留；按语料验证往返 |
| SRT parse/write | P0 | CLI 已实现 | UTF-8、边界时间和往返测试 |
| ASS↔SRT | P0 | CLI 已实现 | 有损报告，不伪造样式 |
| FPS/Duration、Offset、two-point fit | P0 | CLI 已实现 | 有理数时间；边界输入不得回绕 |
| ASS HDR 与 16:9 分辨率变换 | P0 | CLI 已实现 | 只改目标字段；画布语义保持完整 16:9 |
| 多输出与批处理 | P0 | CLI/任务引擎已实现 | 每个输出独立校验；部分失败可定位 |
| libass render | P0 | libass + renderer-host 已有实现 | 第三方 DLL 只在 host 加载；真实 DLL 测试按依赖条件运行 |
| BDN XML+PNG | P0/P1 | CLI 已实现 | XML、PNG 尺寸和时间码互相核对 |
| PGS parser/Fast Path | P1 | 已实现基础路径 | 外部 `.sup` 语料与解码器互操作仍需扩充 |
| PGS Basic Encoder | P1 | 已实现 | Basic 结构校验不等于 Strict Blu-ray 合规 |
| CSRI/VSFilterMod host | P1（待裁定） | 未实现 | 先确认 ABI、可分发 provider、许可和真渲染样本 |
| VapourSynth/AviSynth adapter | 可选 | 未实现 | legacy 可选项，不阻塞 libass |
| Font health | P1 | 基础扫描与报告已实现 | DirectWrite 权威、完整字体环境和外部渲染仍需核验 |
| Font subset | 后续 | 已实现（`font subset`） | `allsorts` 子集 + cmap 级等价检查 + 真 libass 逐像素 RGBA 对照；复杂脚本一律回退整份 |
| MKV helper | 后续 | 已实现（`mkv identify\|extract\|remux`） | 调用外部 MKVToolNix；原文件不改，产物重读逐轨校验（AT-MKV-001 已执行通过；无 EBML 自实现） |
| Media probe | P1 | ffprobe adapter 已实现 | 真实工具与视频样本验证；只提供建议，不自动改速度 |
| Smart sync | 可选 | 未实现 | 外部依赖和 GPL 边界先行评估 |
| Advanced PGS author / Strict validator | 后续 | 未实现完整能力 | 需要 buffer 模型、真盘样本和播放器矩阵 |
| OCR SUP→SRT | 可选 | 未实现 | 独立研究线，不进入 CLI MVP 前置条件 |
| Windows GUI | P0 产品目标 | 未实现 | CLI 数据契约稳定后实施 |
| Installer、portable、签名与更新 | 发布阶段 | 未实现 | 只发布实际构建和验收过的目标 |

## 明确不做的功能矩阵

| 功能 | 决策 |
|---|---|
| 波形/视频手工打轴 | 不做 |
| 逐句字幕编辑 | 不做 |
| Style Manager | 不做 |
| Visual Typesetting | 不做 |
| ASS Draw | 不做 |
| ASR/TTS/翻译 | 不做 |
| 视频转码/烧录 | 不做 |
| 完整蓝光 Authoring | 不做 |

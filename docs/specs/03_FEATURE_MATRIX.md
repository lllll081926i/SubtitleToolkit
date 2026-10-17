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
| Smart sync | 可选 | 已实现确定性执行（`time --piecewise`） | `PiecewiseMap` 多段线性映射，控制点由调用方显式给出；**不做**剪辑点自动检测（需要内容比对，另一条研究线）。RL01 的自动对齐仍未做 |
| Advanced PGS author / Strict validator | 后续 | 第 1–6 层已实现（含第 5 层解码器模型） | 第 7 层设备语料未取得，`strict_badge_allowed` 仍恒为 false；第 5 层未含缓冲区占用曲线与 acquisition 分支 |
| OCR SUP→SRT | 可选 | 未实现 | 独立研究线，不进入 CLI MVP 前置条件 |
| Windows GUI | P0 产品目标 | 已接通（Iced 工作台 + 真 batch 通路） | AT-UI-001 的 60 秒响应实测、A/B 对比渲染未做（设置持久化已交付：36 §1 AppSettings 落盘） |
| Installer、portable、签名与更新 | 发布阶段 | portable 包已实现（checksums + SBOM + notices 三件套） | 安装器、Authenticode 签名、更新器未做 |
| 36 §1 AppSettings | 已交付 | `settings show\|init\|set`；未知字段保留、坏文件不阻塞、原子写 | ToolDefaults 与 Preset（需 TOML 依赖获批）未做 |
| 37 §2 渲染缓存 | 已交付 | 内存共享 + 磁盘（配额、清理、损坏自愈、跨运行复用） | 内存那级不是容量受限 LRU；benchmark 阈值（37 §6）未做 |
| 22 §5 诊断包 | 已交付 | `diagnose`；默认白名单不含用户内容，附样本需显式点名 | 22 §1 日志文件、§2 correlation ID、§6 minidump 未做 |

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

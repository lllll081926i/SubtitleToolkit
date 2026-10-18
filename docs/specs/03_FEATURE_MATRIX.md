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
| Windows GUI | P0 产品目标 | 已接通（Iced 工作台 + 真 batch 通路）；AT-UI-001 已**实测**（最慢 18.5µs / 预算 8ms）；A/B 分屏对比已接真变换 | 真人交互/IME/DPI 实测未做；A/B 不演示时间轴变换 |
| Installer、portable、签名与更新 | 发布阶段 | portable 包已实现（checksums + SBOM + notices 三件套）；**安装器**已交付（`tools/make_installer.ps1` + `verify_installer.ps1`，28 §3 五条逐项验过）；**更新器**第一版已交付（`check-updates`，只做版本比对、不出网） | **Authenticode 签名未做**；all-users 模式只核对静态声明；x86 host **可构建但未随包分发**（前提是 x86 DLL 许可审计，21 §6）；更新器默认 URL 不是真实地址 |
| 36 §1 AppSettings | 已交付 | `settings show\|init\|set`；未知字段保留、坏文件不阻塞、原子写 | Preset 未做（TOML 依赖未获批，ISSUES E4，外部决策） |
| 36 §1 ToolDefaults | 已交付 | `tool-defaults show\|init\|set`；`pgs encode`/`render`/`mkv`/`font subset` 在用户没给参数时真的查它；报告里 `resolved_from` 报出每一项来源；非法值报 `STK-INP-0219` 不静默退回 | `batch` **有意不接**（spec 是 JobSnapshot 层）；GUI 无入口 |
| 17 §5 subset equivalence corpus | 已交付 | `font-service/tests/subset_corpus.rs`（9 用例）逐项回答规格列的 11 项；字符级必回退、字体级能做就做且等价、Latin ligatures 必须真做 | **缺 golden 图像比较**（规格要的 rendered RGBA；需固定语料与参考图，先撞许可审计 21 §6）；variable 的**轴保真未证明** |
| 37 §2 渲染缓存 | 已交付 | 内存共享 + 磁盘（配额、清理、损坏自愈、跨运行复用） | 内存那级不是容量受限 LRU；benchmark 阈值（37 §6）未做 |
| 05 §4.1 A/B 分屏对比 | 已交付 | 左原版/右变换（HDR 调光 + 等比缩放），用 `transform_hdr`/`transform_resolution` 同一份实现；恒等映射会说明原因 | 分隔线不可拖；时间轴变换不参与 |
| 22 §5 诊断包 | 已交付 | `diagnose`；默认白名单不含用户内容，附样本需显式点名 | 真 minidump（需异常处理器 + 含内存镜像）未做 |
| 22 §1 日志 + §2 关联 ID + §6 崩溃现场 | 已交付 | tracing 按日轮转；`J/O/R` 三层 id（日志与报告同一个值）；host 崩溃现场进错误卡 | 真 minidump 未做（见裁决）；日志无 json 输出模式 |

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

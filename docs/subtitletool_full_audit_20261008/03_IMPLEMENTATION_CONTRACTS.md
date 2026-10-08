# 必须补到“能直接编码”的实施契约（审计整改建议）

> 本文件是**建议的 V3.2 实施契约**，不是现有 V3.1 已实现内容。涉及“未知/需实验”的参数一律不得擅自填成经验常量后声称标准合规。

## C-01 统一数据模型

### 设计边界
- `SourceArtifact`：原文件bytes、hash、可访问路径（Windows `PathBuf`）、探测格式、来源编码、BOM、换行特征、完整诊断。
- `AssDocument`：无损 token/span AST，`Script Info`、Styles与Events动态Format、Attachments、Comments、未知私有标签；可对原始字节执行局部补丁。
- `TextCues`：只用于SRT等弱样式格式，禁止将ASS先强制扁平化成文本后再输出ASS。
- `PgsStream`：raw segments与typed view共存，能独立重组ODS并追踪epoch、palette、object、window状态。
- `CanvasSpec`：分别保存 Source PlayRes、可选LayoutRes、StorageSize、PixelAspect、OutputCanvas，特别区分原始输入比率与完整16:9输出。
- `TimeValue`：`ticks:i128 + timebase:Rational`，媒体与PGS writer边界才能转换时基。所有舍入函数必须有显式RoundingMode。

```rust
struct Rational { numerator: i128, denominator: i128 } // denominator > 0, gcd normalized
struct FrameRate(Rational);
struct CanvasSize { width: u32, height: u32 }
enum ScriptResolutionPolicy { Preserve, RewriteValidated }
struct CanvasSpec {
    source_playres: Option<CanvasSize>,
    layout_res: Option<CanvasSize>,
    video_storage: CanvasSize,
    pixel_aspect: Rational,
    output_canvas: CanvasSize,  // special-effect outputs: 16:9 only
    script_policy: ScriptResolutionPolicy,
}
```

不要误用 `PathBuf.to_string_lossy()` 作为Windows文件打开依据。Rust自己读bytes后再送 `ass_read_memory()`；第三方legacy API若不能处理Unicode，优先选能处理内存输入的桥或用户可控的短ASCII staging目录，防碰撞和权限泄漏。

### ASS 无损改写（必须明确）
- 未做transform：必须逐字节输出相同bytes；解析失败不得部分覆盖。
- 做transform：尽量只patch所需字段原始ByteSpan；BOM/换行/空白、注释、顺序、未知标签原样保留。
- Color transform：Style `Format`动态列寻址；dialogue override区分 `\c`/`\1c`…、`\alpha`、颜色嵌套在`\t(...)`内的token；未知VSFilterMod颜色语法保留并发warning。
- 合并多个ASS：必须连同 `Dialogue.Style`、内联 `\rStyle`、Style.Name、字体引用按per-source映射重写；发生不可兼容脚本属性/附件冲突则拒绝自动无损合并，记录LossReport。

## C-02 RenderGroupKey与统一时间线

### 正确的共享层次
1. 全部输出共享 `SourceArtifact`/ASS AST（仅当无各Profile私有改写）。
2. 源时间戳和事件索引可共享；每个输出自己应用FPS/offset/time mapping。
3. 生成 `RenderGroupKey`：

```rust
struct RenderGroupKey {
  transformed_script_hash: Hash,
  renderer_build_hash: Hash,
  renderer_options_hash: Hash,
  output_canvas: CanvasSize,
  source_storage: CanvasSize,
  pixel_aspect: Rational,
  font_environment_hash: Hash,
  color_render_policy_hash: Hash,
  sampler_policy_hash: Hash,
}
```

4. **同RenderGroup内**，将所有sink需要的时间点取并集、排序并去重；只做一次RGBA renderer调用（sink分别按自己的时间网格消费）。
5. **跨Group**仅允许复用解析、归一化时间映射、无关的格式分析，不得因为timestamp相同就误用别的canvas/renderer/color的RGBA。
6. 若`ASS→HDR ASS`仅修改字节值，不需要任何位图渲染；除非用户要求验证/预览。不同分辨率默认不共享RGBA，即使时间戳完全相同。

### 采样与渲染状态
- 使用半开时间区间 `[start,end)`；采样点应区分EventStart、EventEnd、FrameBoundary、Dynamic, Preview。
- 帧边界 rational 计算，绝不在每帧累计浮点误差。
- 动态标签（move/fad/transform/karaoke/private）按frame采样或有证据支持的adaptive策略；不允许“每事件起始取一帧”。
- `libass detect_change` 是效率hint，真正写出时根据最终合成的visible RGBA与上次有效样本比较。需要从合成背景和alpha mask中正确剥离透明空白。bbox+强哈希仍须在碰撞或高保真阈值下逐字节比较。
- 缓存使用内存/磁盘配额、LRU、依赖版本、生命周期与运行中引用计数，保证4K RGBA每帧约31.64MiB（十进制约33.18MB）不爆RAM。

### 必测组合
| Profile组合 | 应共享 | 不得共享 |
|---|---|---|
| 相同画布/renderer/ASS变换，BDN+SUP | render frame | writer自身PGS状态机 |
| 1080 vs 2160 | AST、可共享时间变换 | RGBA像素帧 |
| SDR vs HDR ASS | 原始词法AST | HDR后颜色hash不同的render帧 |
| libass vs VSFilterMod | 原始字幕与通用事件元数据 | RGBA、glyph layout |
| 两次相同任务、字体文件变更 | 原始输入hash | rendered frames |

## C-03 HDR算法的可执行契约

### 输入和输出假设必须显示
- ASS存的是样式及Override RGB字节，本身不记录PQ亮度保证。播放器合成路径、色域和显示设置会影响最终亮度。
- `CompatibilityPercent`：逐sRGB编码RGB通道执行确定的比例映射；保持alpha与非颜色结构。必须明确rounding（推荐half-away或half-even，**统一全工程**）、clip到0..255；提供传统工具兼容参考。
- `LinearLightPercent`：先用标准sRGB EOTF转线性值→依据选择的线性比例缩放→sRGB OETF编码→8bit量化。与编码空间逐通道缩放完全不同，不能共用“33%”而不显示模式。
- `ReferenceWhiteTarget`：203 cd/m²来自ITU-R HDR参考白概念，不等于`ASS白色自动变成203nit`。必须要求用户选择目标播放器色彩合成假设，或将其明确标为“基于指定播放链的近似映射”，给出可校准测试与真实HDR验证环境。没有播放链证据时只提供兼容百分比模式。
- HDR10与HLG色彩/曲线应明确区分；文档不得只依据视频尺寸认定HDR（看transfer/primaries/matrix/side data）。
- `color-transfer` 配置至少记录 source primaries, transfer, matrix, range, target primaries, transfer, white level policy, quantization rounding, alpha policy。
- SUP PDS Fast Path：只改Y亮度用于灰/白字幕且按基准limited range限制；彩色走明确定义的Y/Cr/Cb→RGB线性处理→目标编码→重建PDS。不要凭`.sup`后缀推断是否709/PQ，应询问/识别来源视频。

### 可执行验收
- 除被允许的颜色tokens外，ASS bytes/timing/drawing/style positions一致；alpha默认不改。
- 16进制sRGB颜色输入/输出用固定CSV golden向量（黑/白/灰/红/青/高透明/\\t嵌套）；算法版本一变化就更新fixture并出diff。
- 彩色PGS执行仅修改PDS且不重编码图像时的ODS raw payload保持不变的Fast PDS路径必须单独检验，保持所有非允许字段字节不变。
- 无HDR监视器时产品UI仅显示*估算/对比*，不显示“显示器上203 nit已验证”。

## C-04 Time/FPS 与PGS时序规则

- FPS精确保存为 `24000/1001`，显示层可写`23.976`，实际倍率计算基于分数。
- `new_time = old_time * source_fps / target_fps`仅SpeedChange模式；Snap则映射帧号/帧边界而非变速；Offset只加常量。
- TwoPoint至少要求两组不同源时间且有足够间隔；禁止负斜率、零斜率、溢出和最终倒序；支持有偏移的线性模型。
- 当转换产生负开始时间：策略明确为Reject/ClampShiftAll/Drop(只在用户允许有损时)。转换得到零时长要分级报告，不能默默删。
- PGS读写90 kHz时基，必须有PTS/DTS分开模型及32-bit包装边界规范、SegmentPTS时间顺序与Epoch生命周期独立检查。
- ExistingSUP时间调整分两条：QuickPlayback仅做播放器适配并声明级别，AuthoringAware必须满足display/decode合法关系或重新author。
- PGS `ODS payload` 不改可确认无位图重编码，但`Forced/Time/Position/Palette`本身可改变观看结果，UI只称“无位图重编码”，不能笼统称“无损”。
- VFR：需要真实frame timestamp map才能做frame snapping；否则提供时间型偏移/比例变换，不假定avg_frame_rate即所有帧时刻。

## C-05 PGS/BDN标准参照与Strict报告

### 独立的验证器任务（无需先完成高级Authorer）
1. `pgs-structure-check`：同步字、header、长度/边界、段顺序、ODS分片长度、RLE结束、字段类型、对象/调色板引用。
2. `pgs-timeline-check`：PTS/DTS解码显示顺序、epoch序列、时间精度、clear display set。
3. `pgs-color-check`：PDS每条 `color_id,Y,Cr,Cb,alpha` 五个字节，数值范围与颜色矩阵由Profile策略控制。
4. `pgs-profile-check`：BD/特定UHD BD允许的分辨率/目标FPS/色彩传递组合矩阵，禁止任意2160p SUP假称合规。
5. `pgs-buffer-model-check`：buffer模型、object cache、acquisition与带宽窗口；**没有正式规范引用与实机对照之前不能称为“已完整实现”**。
6. `pgs-device-corpus`：明确列播放软件、型号、固件、受控样本；这是行为覆盖，不是“全部设备认证”。

### BasicEncoder最低字节测试
- PCS/WDS/PDS/ODS/END及empty frame；palette/object/window ID/version一致；正确alpha。
- 大ODS跨多个segments分片，边界短1/等于/长1字节和错误truncate都可复现。
- 重新读取输出的DisplaySets并与reference decoder比较合成RGBA，包括两个重叠对象和对象复用。
- `T=alpha`的具体alpha语义需reference decoder证实，不能仅依旧源码按猜测转换。
- 16:9全画布Composition与被裁剪bbox object坐标分离，支持原片带黑边。
- BDN PNG导出是否需要索引色由**目标Authoring消费者要求**决定；不应因PGS只支持调色板就强制所有BDN输出先量化损失画质。

### 结果分级
- `Parsed / BasicDecoded / StructuralValid / TimelineValid / BufferModelValid / DeviceTested`作为独立指标。
- 公开UI用“结构通过 / 时间通过 / 模型校验通过 / 已在列明设备上实测”，避免笼统“严格过大金保证所有蓝光机”。
- 每项报告必须有validator build/规则版本、input/output hash、是否可重放、已知局限、具体违规offset/DisplaySet。

## C-06 外部滤镜进程与安全

### Host协议
- 主程序x64 Rust/Iced，不加载任何第三方ASS滤镜DLL；由x86或x64 renderer-host负责probe/render。
- NamedPipe控制消息包含固定magic、protocol version、message type、request id、长度上限、CRC/完整性；RGBA大数据走命名FileMapping，包含 width/height/stride/pixelformat/alpha模式/实际长度/帧时间戳。
- 通过PE架构/导出表探测接口；同一DLL不以名字来决定CSRI/VapourSynth/AviSynth调用。
- Host启动应设置DLL search directories为受控目录，避免PATH/current-dir注入；显式绝对路径`LoadLibraryExW`，依赖DLL策略写清。
- 协议有Open/Render/Cancel/Close/Ping/Probe/Error和限定超时；所有输入长度与count checked。
- Job Objects能限制资源/子进程并便于清理，**不等于安全沙箱**；插件访问系统文件/网络需要单独设计受限令牌/完整性级别或明确告知用户风险。
- 插件崩溃不会让GUI崩溃，但渲染行为正确性、内存访问权限、字幕内容保密性仍另需保障。

## C-07 批量/多输出/输出事务

### 状态机
`Queued → Probing → Ready → Running → Verifying → Committed`；任何Running可进入Cancelling、Cancelled、Failed；多Profile部分成功归档PartialSuccess；缓存恢复须经artifact hash检查。

### 输出计划
- `JobSnapshot`包含所有输入hash/size/mtime、归一化参数、renderer身份、插件/字体hash、输出Profile排序、应用与schema版本。
- 执行前全部输出路径进行collision预检查（大小写、NTFS保留名、尾随空格/点、重复目录），并显示预计覆写/重命名。
- 输出写入最终目标同卷临时位置；多文件BDN按staging目录及manifest发布，避免XML指向半完成的PNG集合。Windows锁定文件/跨卷异常要有fallback及恢复日志。
- 每个Stage有可取消检查点，但取消不能在已经提交单个文件的中间破坏旧文件；renderer-host和child process取消超时后逐层升级终止。
- FFprobe/MKVToolNix外部进程使用参数数组，工作目录和环境固定，stdout/stderr size限制，路径不经shell拼接。

## C-08 UI合同（仅保留“处理工具”而不变编辑器）

| 页面 | 核心可见操作 | 高级操作 | 禁止/异常状态 | 核心验收 |
|---|---|---|---|---|
| 转换 | 输入、格式、输出路径、开始 | 编码、换行、损失报告、文件名模板 | 不兼容转换禁执行并解释 | REQ-Convert → Stage → AT-ASS/SRT |
| ASS/HDR | 选择ASS/亮度模式/输出ASS | 色域假设、扩展颜色、参考白说明 | 未知颜色扩展给warning，不声称已处理 | 白/彩/透明/\\t黄金向量 |
| FPS/时间 | 源FPS、目标FPS、Speed/Snap/Offset | 两点拟合、负时间策略、帧网格 | 源FPS未知不得自动Speed | AT-TIME系列 |
| BDN/SUP | 画布、Renderer、帧率、输出profile | 采样、HFR policy、quantizer | strict+2160p等非法组合禁用 | render corpus +PGS tests |
| SUP | offset、move、forced、brightness | DTS复核、色域、bitmap resize | 不支持fast则说明将重编码 | ODS invariant，time validate |
| 批量 | 多输入、profile表、执行队列 | glob、并发、内存/磁盘上限 | 同名产物冲突必须预检 | 100×3+部分成功/恢复 |
| 字体/MKV | 扫描字体、提轨、封装新文件 | subset、字体授权、track flags | 未授权字体默认不可打包 | font render equivalence,MKV metadata |
| 同步助手 | 加目标视频/参考字幕、建议 | ALASS engine与段拟合 | 必须明确应用到副本 | offset/scale/segmented可解释 |
| 诊断 | 依赖、版本、自检、日志 | 导出脱敏报告 | renderer-host不可信须明示 | crash/privacy golden |

全局UI要求：中日韩IME输入、纯键盘操作、焦点可见、可访问性标签，1366×768/1080P/4K和100%/150%/200%DPI验收；过窄时抽屉默认收起而不是遮掉输出按钮。每项交互必须可追踪至配置字段、stage和测试，不可放无效按钮。

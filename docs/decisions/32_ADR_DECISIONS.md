# ADR：关键架构决策

`Accepted` 表示采用该设计方向，不表示相关代码或验收已经完成。ADR-001–012 保留原规划记录；2026-10-08 的细化决定如下，替代相关旧条款中的冲突表述。当前能力以[状态](../STATUS.md)为准。

## ADR-001 Windows-only
Accepted。减少跨平台复杂度，集中满足目标用户和字幕滤镜生态。

## ADR-002 Iced 0.14
Accepted。替代 V2 Slint-first。

## ADR-003 Manual-first / Assist-second
Accepted。自动 probe 不覆盖用户明确参数。

## ADR-004 16:9 full canvas
Accepted。特效字幕永远以完整 16:9 逻辑画布缩放和定位。

## ADR-005 External renderer isolation
Accepted。第三方 DLL 只进 renderer-host。

## ADR-006 CSRI before VapourSynth/AviSynth
Accepted。现代 VSFilterMod 兼容链优先走更窄接口。

## ADR-007 Rational time
Accepted。禁止核心使用 float FPS/seconds。

## ADR-008 Multi-output DAG/cache
Accepted。一份输入多个目标，复用昂贵阶段。帧共享条件由 ADR-013 细化；不得仅以 timestamp/canvas/fps 相同判定复用。

## ADR-009 PGS maturity labels
Accepted。Basic Encoder 与 Strict Blu-ray 完全分级。

## ADR-010 No libimagequant hard dependency
Accepted。许可原因。

## ADR-011 External ffprobe/MKVToolNix first
Accepted。成熟工具 adapter 优先于早期自研容器/媒体解析。

## ADR-012 Inno Setup + portable
Accepted for V1 release；MSIX optional later。

## ADR-013 按完整渲染条件分组

状态：Accepted。日期：2026-10-08。对应 AUDIT-A001。

决定：同 RenderGroupKey 内合并采样并复用 RGBA；跨组只共享原始 AST 与可证明相同的分析结果。键覆盖脚本变换、渲染器、几何、字体、颜色与采样策略，定义见[09](../specs/09_CANVAS_RESOLUTION_MULTI_OUTPUT.md#6-rendergroupkey-与时间采样)。

替代：所有 profile 的时间点取并集后全局渲染一次。拒绝原因：不同画布、字体或 provider 的同一时间戳不会产生相同像素。

影响：WBS-R040/J003 与 AT-MULTI-002 必须有跨组隔离及同组计数证据。现有缓存键和采样模块不代表完整执行链已验收。

## ADR-014 验证器独立于 Advanced Author

状态：Accepted。日期：2026-10-08。对应 AUDIT-A003/A004/A008。

决定：解析器先支持结构与时间验证，缓冲模型可先用外部样本开发；Advanced Author 依赖经过验证的模型。报告逐项记录规则版本与设备覆盖。

替代：先完成高级编码器，再开始 Strict 验证器。拒绝原因：形成错误的实施顺序，并让 writer 与 validator 共用未经独立裁决的假设。

影响：WBS-P011 不再依赖 P010；P010 依赖 P011。Basic 输出与 ffprobe 能读取容器均不升级为全设备合规声明。

## ADR-015 保留导入证据，按现有工程采纳

状态：Accepted。日期：2026-10-08。对应 AUDIT-A004/A011/A028/A036。

决定：两份导入资料保留原始内容；维护规格进入分类目录。以[采纳记录](../planning/AUDIT_ADOPTION_20261008.md)连接审查问题、适用规格与未完成验收。

替代：将建议包的 59 项 WBS 和 M0–M5 全量覆盖当前计划。拒绝原因：其审查对象是原规划，不包含当前实现证据，且任务编号有不同含义。

影响：沿用当前 Phase 与任务 ID。导入包引用行号保持可复查；建议被采纳不等于问题的行为验收关闭。

## ADR-016 宿主隔离与传输方案分开验收

状态：Accepted。日期：2026-10-08。对应 AUDIT-A007/A009。

决定：保留当前有界匿名管道协议作为 libass 基线；NamedPipe/FileMapping 仅在测量与需求支持时升级。Job Object 不称为权限沙箱。

替代：将共享内存、Cancel、heartbeat 和受限令牌全部视为当前宿主的必备已实现能力。拒绝原因：混淆目标与现有接口，且无法用实际测试支持。

影响：现有协议版本 2 的回归继续适用。新增 provider、x86 路由、权限限制与传输升级各有独立 capability 和真实依赖验收。

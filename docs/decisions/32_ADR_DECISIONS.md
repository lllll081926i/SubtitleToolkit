# ADR：关键架构决策

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
Accepted。一份输入多个目标，复用昂贵阶段。

## ADR-009 PGS maturity labels
Accepted。Basic Encoder 与 Strict Blu-ray 完全分级。

## ADR-010 No libimagequant hard dependency
Accepted。许可原因。

## ADR-011 External ffprobe/MKVToolNix first
Accepted。成熟工具 adapter 优先于早期自研容器/媒体解析。

## ADR-012 Inno Setup + portable
Accepted for V1 release；MSIX optional later。

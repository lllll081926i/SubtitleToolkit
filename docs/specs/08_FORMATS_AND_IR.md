# 格式体系与内部 IR

## 1. 不使用“万能字幕结构”

### TextSubtitle IR
适合 SRT/WebVTT 等弱样式文本字幕：cue(start,end,text,position-basic)。

### ASS AST
必须最大限度保留原文：
- Script Info 顺序/未知 key；
- Styles Format 动态字段；
- Events Format 动态字段；
- Comments；
- Attachments/Fonts；
- override blocks 原 token；
- 未识别 VSFilterMod tag。

原则：**只改目标字段，其他内容原样 round-trip**。

### PGS IR
- DisplaySet；PCS/WDS/PDS/ODS/END；
- palette/object/window/version/lifecycle；
- raw segment bytes 可保留，以支持 Fast Path。

### BDN IR
- metadata；format/frame rate；event InTC/OutTC；graphic path/width/height/x/y；forced。

## 2. 时间
内部不存 float seconds。
- 当前 `Time(Rational)` 保存有理数秒，分子/分母为 `i64`，中间算术使用 `i128`；
- FPS 单独保存精确有理数，舍入规则见[时间规格](10_TIME_FPS_SYNC.md)；
- PGS 边界转换为 90 kHz ticks，并检查 SUP 字段范围；
- BDN timecode 在 writer 边界转换。

## 2.1 原始字节与变换边界

未变换的 ASS 在已支持编码路径上应逐字节往返；语义等价只能作为额外检查。局部变换只改目标字段或 token，保留其余 BOM、换行、注释、顺序与未知扩展。编码检测不确定或需要重新编码时，报告编码变化；禁止将替换字符后的文本标为字节保真。

当前 ASS AST 与局部字段改写已有实现。审查建议的统一 `SourceArtifact`、`ByteRangePatch` 类型不是当前公开 API；实施时应先明确现有结构能否满足契约，再决定新增类型。

## 3. 格式探测
扩展名只是 hint。
- Magic/header；
- text signature；
- XML root/namespace；
- SUP `PG` segments；
- SUB/IDX 配对。

## 4. Loss Model
每次转换返回：
- preserved；
- approximated；
- dropped；
- non-standard；
- user_attention。

ASS→SRT 至少提示：styles、position、drawing、karaoke、transform、clip、fonts 会丢失或扁平化。

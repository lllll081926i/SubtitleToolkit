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
- `Rational {num, den}`；
- `TimeNs` 或有理数 tick；
- PGS 90 kHz ticks；
- BDN timecode 在 writer 边界转换。

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

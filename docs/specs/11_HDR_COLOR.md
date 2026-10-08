# HDR / 字幕亮度与颜色适配

## 1. 产品语义
“ASS HDR 适配”= 对原有 ASS 颜色值做映射，使其在 HDR 播放链中达到期望主观亮度；不是给 ASS 写入 HDR metadata。

## 2. 处理范围
- Style Primary/Secondary/Outline/Back；
- inline `\c/\1c/\2c/\3c/\4c`；
- 支持确认过的 VSFilterMod gradient/color extension；
- Alpha 独立，默认完全不改。

## 3. 模式
### Compatibility
模仿社区成熟工具的 RGB dim 行为，目标是结果可预期。

### Percentage
每通道/线性化后按定义比例缩放；算法版本必须写进 metadata/report。

### Reference White
用户选择目标 reference-white nit（如 203），根据定义的 SDR reference 与 HDR pipeline 映射。UI 必须说明这是“字幕颜色预调整”，播放器实际合成仍可能影响最终亮度。

## 4. SUP palette dim
Fast path 只修改 PDS palette，保持 ODS bitmap/rle 不动；颜色空间解释必须基于 PGS palette 的 Y/Cr/Cb 语义，而不是旧 AssRocket 的错误 5-byte RGBA 假设。

## 5. Preview
GUI preview 只是相对比较：
- Original swatch；
- Adapted swatch；
- numerical RGB/YCbCr；
- 不宣称普通 SDR monitor 能真实预览 HDR nit。

## 6. 测试
- 白、灰、彩色、描边、阴影、透明色；
- inline tags；
- 动态 transform 内颜色；
- VSFilterMod extension；
- round-trip 不改时间/文本/几何。

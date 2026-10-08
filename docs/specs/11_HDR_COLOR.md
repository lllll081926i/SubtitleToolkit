# HDR / 字幕亮度与颜色适配

文档修订：2026-10-08。当前算法位于 `crates/transform-hdr/src/lib.rs`，`ALGORITHM_VERSION = 1`。本页描述该实现；播放器色彩管理与真实 HDR 显示验收属于后续工作。

## 1. 产品语义
“ASS HDR 适配”= 对原有 ASS 颜色值做映射，使其在 HDR 播放链中达到期望主观亮度；不是给 ASS 写入 HDR metadata。

## 2. 处理范围
- Style Primary/Secondary/Outline/Back；
- inline `\c/\1c/\2c/\3c/\4c`；
- 支持确认过的 VSFilterMod gradient/color extension；
- Alpha 独立，默认完全不改。

## 3. 模式
| 当前模式 | 计算域与比例 | 输入约束 | 含义 |
|---|---|---|---|
| `Off` | 不变换 | 无额外比例 | 文档保持原样 |
| `Compatibility` | 编码 RGB 字节乘 `1/2` | 当前比例固定 | 确定的通道减半，不能解释为线性亮度减半 |
| `Percentage` | sRGB 解码到线性光后乘 `basis_points / 10000` | `basis_points` 为 1–10000，即 0.01%–100% | 线性光百分比；与编码通道百分比区分 |
| `ReferenceWhite` | 在线性光域乘 `min(reference_nits / 203, 1)` | `reference_nits > 0` | 当前算法的近似预压暗参数，203 是计算基准 |

`ReferenceWhite = 203` 的比例为 1，当前算法不会改变 RGB；更大值也不提亮。该参数不向 ASS 写入 PQ/HLG 信息，不保证字幕实际显示为指定 nit。未来采用播放链校准算法时必须升算法版本，不能沿用旧名称悄悄改变结果。

### 逐通道公式

令输入通道 `v` 为 0–255，比例为 `f`。Compatibility 输出 `floor(v*f + 0.5)` 并限制到 0–255。

Percentage 与 ReferenceWhite 先令 `c = v/255`，再计算：

```text
L = c/12.92                         当 c <= 0.04045
L = ((c+0.055)/1.055)^2.4           其他情况
L' = clamp(L*f, 0, 1)
c' = 12.92*L'                      当 L' <= 0.0031308
c' = 1.055*(L')^(1/2.4)-0.055      其他情况
v' = clamp(floor(255*c'+0.5), 0, 255)
```

非负通道采用四舍五入，半值向上。50% 参数下，255 的编码通道减半得到 128，线性光缩放得到 188。此差异是模式定义的一部分。

### 字段与失败处理

按 Style `Format` 动态定位四个颜色字段；内联处理 `\c`、`\1c`–`\4c` 及可识别的 `\t(...)` 内颜色。Alpha 默认不变。未知扩展或无法解析的颜色保留原文，并计入报告；不得将“保留”记为“已适配”。

报告记录模式、参数、算法版本、修改数量与未处理数量。完整 source/target 色域、传递函数和播放链配置仍是目标，当前百分比模式以 sRGB 假设处理颜色字节。

## 4. SUP palette dim
Fast path 只修改 PDS palette，保持 ODS bitmap/rle 不动；颜色空间解释必须基于 PGS palette 的 Y/Cr/Cb 语义，而不是旧 AssRocket 的错误 5-byte RGBA 假设。

ASS 颜色预调整与 SUP 调色板调整是不同路径。SUP 来源视频的色彩信息未知时，不以 `.sup` 后缀或画布尺寸推断 PQ/HLG；Y-only 与彩色映射的边界见 [PGS 规格](13_PGS_BDN_BLURAY.md#33-palette-brightness)。

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

当前相关回归用例位于 `transform-hdr` 模块内，包括 `compatibility_halves_rgb_by_integers`、`percentage_and_reference_white_use_linear_light`、`alpha_is_untouched_by_default` 和 `nested_animation_color_is_scaled`。这些是测试定位，不表示本次文档修改重新执行了测试。验收步骤见 [AT-ASS-002](25_ACCEPTANCE_TESTS.md#颜色与几何)。

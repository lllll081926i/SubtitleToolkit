# 特效字幕画布、坐标空间、分辨率适配与多输出

## 1. 全局硬规则：输出画布始终是完整 16:9
所有特效字幕输出都以完整 16:9 视频帧作为 **Output Canvas**：
- 1280×720；
- 1920×1080；
- 3840×2160；
- 其他自定义输出默认要求保持 16:9，除非用户明确选择实验性自定义画布。

即使正片内容为 2.35:1/2.39:1 且带上下黑边，也**不得**自动把字幕逻辑画布缩到有效画面高度。字幕位置仍属于完整视频帧坐标。

## 2. 必须区分四个坐标/尺寸概念
源码审计后，V3.1 不再把“ASS 画布”简单等同于一个 PlayRes：

1. **Output Canvas**：最终渲染输出尺寸，例如 1920×1080 / 3840×2160；永远遵守本项目完整 16:9 规则。
2. **Script Coordinate Space**：ASS 自身的 PlayResX/PlayResY；是样式、`\\pos`、drawing 等脚本坐标的基础。
3. **Layout Resolution**：若 ASS 含 `LayoutResX/LayoutResY`，其语义不能被忽略或机械跟 PlayRes 一起改写。libass 0.17.5 源码明确指出，合法 LayoutRes 会优先影响 source image size / pixel-aspect 计算。
4. **Storage Size / Pixel Aspect**：实际视频存储尺寸与像素宽高比。调用 libass 时必须显式设置 `ass_set_storage_size()` 和 `ass_set_frame_size()`；非方形像素再设置 PAR。

### 2.1 默认策略
- 原样渲染：保留 PlayRes / LayoutRes；只把目标完整 16:9 Output Canvas 与真实 Storage Size 告知 renderer。
- “视觉等效升分辨率”：只有在用户明确选择 **Rewrite Script Resolution** 时，才重写 PlayRes 与相关坐标/尺寸。
- 遇到 LayoutRes：默认保留，进入专门 transform；不得无脑 ×2。
- 所有修改必须经过 libass + VSFilter/CSRI differential corpus。

## 3. Bounding Box 的正确用途
允许渲染后对透明区域做对象裁剪，以降低 PNG/PGS 体积、减少 PGS coded buffer 压力；但是：
- bbox 只产生局部 object bitmap；
- 必须保存其相对于完整 16:9 canvas 的 x/y；
- 不改变 ASS 脚本坐标系；
- 不改变 composition canvas；
- 多 object 时必须分别记录对象 bbox 与 placement。

## 4. ASS 1080→2160 的两种不同模式

### A. Render Upscale（默认推荐）
脚本保持原 PlayRes/LayoutRes，renderer 输出到 3840×2160。优点：最大程度遵守原字幕作者坐标语义，也避免重复缩放脚本参数。

### B. Rewrite Script Resolution（高级）
把 1920×1080 脚本改写为真正的 3840×2160 脚本。scale factor = 2 时至少审计：
- PlayResX/Y；
- Style Fontsize；
- MarginL/R/V；
- Outline/Shadow；
- `\\pos`, `\\move`, `\\org`；
- vector `\\clip/\\iclip` coordinates；
- drawing `\\p` path coordinates（同时考虑 `\\pN` 自身缩放语义）；
- `\\bord/\\xbord/\\ybord`、`\\shad/\\xshad/\\yshad`；
- `\\fs`, `\\fsp`, `\\fscx/\\fscy` 等尺寸相关 override；
- blur/be 是否缩放必须通过 differential corpus 证明，不凭感觉统一乘倍数；
- VSFilterMod 私有 tag 必须列白名单；未知 tag 原样保留并给 warning。

若存在 LayoutRes，Rewrite 模式必须先判断其用途；不能简单同时把 PlayRes、LayoutRes、所有 tag 都 ×2，否则可能产生“双重缩放”。

## 5. Multi-output Profile
每个 Profile 是不可变配置：
- name；
- format；
- output canvas；
- script-resolution policy（preserve / rewrite）；
- fps / time mode；
- HDR；
- renderer；
- PGS strict level；
- output suffix/path template。

## 6. Render Timeline Planner（V3.1 新增）
同一输入生成多个输出时，先求所有输出所需采样时间点的并集：
- BDN：视频帧边界；
- 普通 SUP：目标 PGS 采样点；
- High Precision SUP：帧边界 + ASS event 精确 start/end + 动态特效需要的采样；
- 预览：用户请求的 timestamp。

统一 timeline 只渲染一次，然后由不同 sink 消费。静态相邻样本可通过 **visible composited-frame hash/pixel diff** 去重；不能只相信 libass `detect_change`。

## 7. 典型组合
一次输入同时输出：
- `name.1080p.SDR.ass`
- `name.2160p.HDR.ass`
- `name.1080p.BD.sup`
- `name.UHD-BD.HDR.sup`（仍使用合规 2K PGS 画布）
- `name.srt`

其中 ASS 多分辨率可以选择 Preserve Script / Rewrite Script 两种策略；PGS/BDN 均由统一 16:9 composition canvas 派生局部 object bbox。

## 8. 2160p 与 Blu-ray 的标签
- 2160p ASS：支持。
- 2160p BDN XML/PNG：支持为 custom/exotic output。
- 2160p PGS：仅 custom/non-standard 实验性输出；绝不标 Blu-ray/UHD-BD compliant。
- UHD Blu-ray strict profile：使用规范允许的 PGS 画布/编码约束，不把视频 3840×2160 等同于字幕 bitmap 3840×2160。

## 9. 验收硬规则
- 2.39:1 视频不得改变完整 16:9字幕坐标。
- Preserve Script 在 1080→2160 时不得修改源 ASS 文本。
- Rewrite Script 必须在 corpus 中达到目标 renderer 的视觉等价阈值。
- LayoutRes corpus 必须单独测试 libass / VSFilterMod / CSRI。
- bbox 裁剪前后在完整 canvas 上 composite 的 RGBA 结果必须像素等价（允许量化阶段另定义误差阈值）。

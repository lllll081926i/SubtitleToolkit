# PGS / SUP / BDN / Blu-ray 兼容

## 1. 分阶段能力
作者链按 Probe、Parser/Decoder、Fast Transform、Basic Encoder、Advanced Author 分阶段。验证器独立开发：Parser → 结构验证 → 时间/生命周期验证 → 缓冲模型验证；Advanced Author 消费验证结果，不作为开发验证器的前置条件。

目标报告分别记录作者链能力与已完成的验证项。Basic Encoder 的存在不推导结构、时间、模型或设备验证全部通过。

## 2. PGS parser / decoder 是有状态机
至少正确处理 PCS/WDS/PDS/ODS/END，并跨 display set 维护：
- epoch / composition state；
- palette id + version；
- object id + version；
- windows；
- palette-only update；
- acquisition / epoch start / normal case；
- ODS fragments 重组；
- erase / empty composition；
- overlapping objects + alpha composite；
- forced flag。

未知/扩展数据若要 round-trip，必须定义 preserve policy，不得静默丢弃。

## 3. Fast Transform
适合：
- timeline offset/scale；
- composition/window position；
- Forced flag；
- palette luminance / color transform；
- crop metadata（在可证明不改 ODS 的场景）。

### 3.1 ODS invariant
产品称“无位图重编码快速路径”。时间、位置、Forced 和调色板修改可能改变观看结果。输出前后：
- 所有 ODS payload SHA-256 必须相同；
- object fragmentation 顺序不变；
- 只允许修改白名单 segment/field。

### 3.2 timeline 注意
SupMover 源码证明“直接改 segment PTS”非常实用，但它的 resync/delay 路径主要修改 PTS。我们的 Strict pipeline 不能直接照搬：修改后必须重新检查 DTS / decode lead / monotonicity；不满足就 re-author。

### 3.3 palette brightness
SupMover 的 tonemap 仅修改 limited-range Y，并且项目文档明确提醒只适合 B/W 字幕，彩色可能异常。因此我们的 HDR/SUP palette transform 分两种：
- Luma Scalar：兼容传统“降亮度”；只改 Y；UI 明确标注适合白/灰字幕；
- Color-managed Transform：解码 Y/Cr/Cb→linear/color model→目标→重新量化 Y/Cr/Cb/T；用于彩色特效。

## 4. Basic Encoder
目标：结构正确、主流播放器可读，不自动宣称“硬件蓝光严格兼容”。

必须：
- composition canvas 始终是完整 16:9 profile；
- bitmap object 可以 tight transparent bbox；
- transparent palette entry / alpha 约定明确；
- RGB→PGS Y/Cr/Cb/T 矩阵与 limited range 正确；
- PDS palette id/version 正确；
- ODS RLE、declared object-data length 正确；
- ODS 超过单 segment payload 时正确分片；
- object/window id 一致；
- PCS object placement 使用 full-canvas coordinate；
- 90 kHz PTS/DTS 合法；
- display set segment order；
- END marker；
- 全透明 frame 要能表达 erase display set。

源码参考中，Masaiki ass2bdnxml 的 `sup.c` 已明确按 `Y, Cr, Cb, T` 写 palette，并对大 RLE 用首片 65508 / 后续 65515 字节级分片；这些应进入我们的 golden vectors，而不是复制其 C 代码。

本节是实现要求，不构成播放器互操作结论。当前 Basic Encoder 使用 limited-range YCbCr，SD 选择 BT.601、HD 选择 BT.709；选择依据和设备接受度仍需真实 SUP/播放器语料裁决。内部往返和 `ffprobe` 容器读取不能证明这些图像字段对设备正确。

## 5. Advanced Author
参考 SUPer 的行为模型，但独立实现：
- object reuse / object redefinition；
- palette update / palette animation；
- epoch planning；
- windows layout optimization；
- ahead buffering；
- coded object buffer model；
- acquisition point；
- normal case / prefer normal 策略；
- long-event redraw strategy；
- duplicate visual event elimination；
- decoder bandwidth 下的 event drop/merge policy；
- image quantization quality vs decode cost。

### 5.1 “最大码率”必须拆成两项
SUPer 当前 README 明确：Maximum bitrate 是**输出后的测试阈值**，不会主动 shape stream。因此 V3.1 分开：
- `PeakBitrateValidator`：测量/报错；
- `BitrateShapingPolicy`：若未来实现，通过 acquisition/object reuse/palette/quantization/layout 策略实际改变流。

不能有一个“最大码率=xxx”参数却让用户误以为 encoder 一定会压到该值。

## 6. Strict Validator
分层输出：
1. **Structural**：segment 长度、顺序、IDs、fragment、palette/object refs。
2. **Timeline**：PTS/DTS、单调性、display/decode 顺序、帧边界。
3. **Lifecycle**：epoch、palette/object/window 定义与引用。
4. **Profile**：允许分辨率/FPS/bit-depth/profile。
5. **Decoder Model**：coded object buffer、window/object decode time、acquisition。
6. **Bitrate Test**：瞬时/滑窗/目标 mux budget。
7. **Device Corpus**：软件播放器 + 已确认的硬件/authoring tool smoke tests。

报告 severity：info / warning / error / fatal，并给 segment offset、display set number、PTS、修复建议。

## 7. BDN XML / PNG
- BDN 是交换/中间表示，不等于最终 PGS strict；
- PNG 实际尺寸与 XML Graphic Width/Height 必须一致；
- x/y 属于完整16:9 canvas；
- event 必须排序；
- timecode 从统一 rational frame clock 生成；
- identical visible frames 应去重；
- BDN 仍以视频 frame boundaries 为主；High Precision SUP 的 exact event boundary 不强行塞回 BDN；
- 2160p BDN 只能标 custom/exotic export。

## 8. 多输出共享渲染
ASS→BDN+SUP 时采用统一 Render Timeline Planner：
- 先按 [RenderGroupKey](09_CANVAS_RESOLUTION_MULTI_OUTPUT.md#6-rendergroupkey-与时间采样)分组；以下共享规则仅在同组内成立；
- BDN 只消费 `xml_frame=true` 的帧采样；
- SUP 可以消费 frame samples + exact event boundaries；
- 同一个 timestamp 的 RGBA 只 render/composite 一次；
- sink 分别维护 event segmentation / palette / PGS state。

## 9. “大金过水”产品命名
UI 主名称：`Blu-ray Compatibility / 严格兼容化`。
帮助页：说明社区常称“大金过水”。

当前不得显示 Strict badge。后续报告分别显示结构、时间、生命周期、Profile、缓冲模型与列明设备的结果；每项附规则版本、样本摘要、依赖版本和未覆盖范围。设备实测只支持列明型号与固件的结论。外部 Scenarist 可作为人工/实验室对照，不作为用户日常必要依赖。

## 10. 尚未裁决的互操作字段

- 调色板 full/limited range 与 SD/HD 矩阵：现行编码按 limited range、576 行及以下 BT.601、以上 BT.709 写入；待真盘样本与播放器对照后再定验收值。
- PCS 第 5 字节：编码器默认写 0，也允许调用者指定；字段取值表没有真盘证据，严格验证器不得把默认值当成已裁决值。
- ODS 图末标记：当前编码器在末行行结束符后写 `00 00 FF FF`，解码器也接受没有该标记的流；参考实现之间存在差异，需真盘与播放器语料决定兼容策略。

只有 SUP 容器段布局和时钟已由真实 ffprobe 验证。上述图像语义仍是待决项，见历史问题记录 C16、C17、C19 与 E3。

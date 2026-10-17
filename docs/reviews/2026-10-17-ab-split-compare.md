# 裁决记录：05 §4.1 的 A/B 分屏对比（2026-10-17）

## 需求

05 §4.1：「提供 `[分屏对比]` 开关：左半屏显示原版（SDR / 原始尺寸），
右半屏显示应用当前 Profile 变换后（HDR 调光 / 16:9 缩放）的渲染画面，
调光调色效果一目了然。」

05 §1 第 3 条另有硬约束：「严禁在界面上放置任何未实现功能的占位符控件……
所有出现在界面上的按钮、下拉框均必须由底层实际已交付的能力驱动。」

## 交付前的真实状态（本次改动的全部理由）

`Message::ToggleSplitAb` **只翻转一个布尔值并改按钮文字**：

```rust
Message::ToggleSplitAb => self.preview.toggle_split_ab(),
pub fn toggle_split_ab(&mut self) { self.is_split_ab = !self.is_split_ab; }
```

画面毫无变化。那正是 05 §1 第 3 条明令禁止的那类缺陷：**看起来能用**。

## 交付

- `preview_worker::render_ab`：同时间点渲两次，B 侧先对**脚本文本**应用变换；
- `render_single_frame` 与 `render_ab` 共用一个 `render_script`（同一套 host 启动、
  同一套 provider 定位），所以对比画面与导出用的是同一个渲染路径；
- `app::request_preview`：三个入口（选中输入 / 改时间 / 切 A/B）共用一处派发；
- `app::ab_plan`：从当前启用 Profile 推出两侧画布与 HDR 设置；
- `preview_view::split_ab`：左右拼接 + 中缝分隔线；
- 画面上方一行说明：`左：原版… | 中：分界线 | 右：当前 Profile —— <做了什么>`。

## 决定与理由

### D-AB-01：变换作用于**脚本**，不作用于渲染出的像素

规格里 B 侧要展示的是"应用当前 Profile 变换后的画面"，而 Profile 的变换在流水线里
就是对脚本做的（`transform_hdr::apply`、`transform_resolution::apply`）。
如果在这里改像素，A/B 与真正导出的结果就会是两套颜色 ——
而用户开对比模式的目的正是"预判导出成什么样"。

用的是**同一份** `HdrSettings` 与 `Geometry` 构造路径，不是另写一遍映射。

### D-AB-02：必须用核心的构造函数，不能拿 `..Default::default()` 拼

这是开发中真正抳出问题的那个决定。`HdrSettings` 的 `touch_styles` / `touch_inline`
默认为 `false`（留给"只想看报告"的调用），所以：

```rust
// 第一版（错）
HdrSettings { mode: HdrMode::ReferenceWhite, reference_nits: nits, ..Default::default() }
// 结果：一个颜色都不改，却**报告成功**（applied.styles == 0）
```

正确写法是 `HdrSettings::reference_white(nits)` / `percentage(bp)` ——
它们会把两个 `touch_*` 置为 `true`。

A/B 探针抓到的现象是：`note` 显示"改了 0 个样式色 + 0 个内联色"，
两侧像素逐字节相同（0/921600 差异）。**如果没有"像素真的变了"这条断言，
这个缺陷会一路绿过去** —— 因为设置合法、变换返回 `Ok`、渲染也成功。

### D-AB-03：`ReferenceWhite(203)` 是恒等映射，而且必须说出来

`transform_hdr::SDR_REFERENCE_NITS == 203`，所以 `ReferenceWhite(203)` 的因子是
`203/203 = 1` —— **映射是恒等的**。而 203 恰好是 GUI 默认 profile 的取值，
也就是新用户第一次点 A/B 会看到的东西：两张一模一样的图。

这不是 bug（203 nit 就是 SDR 基准），但必须说清楚，否则用户以为功能坏了。
所以注记在那时会写成：「HDR 参考白 203 nit 的映射是恒等（203 nit 就是 SDR 基准）：颜色不变」。

**这条是探针抳出来的**：第一版探针用 203，看到 0 差异，先怀疑实现；
改用 120 nit 后差异立刻出现（`&H00808080` → `&H00646464`，2 个样式色被改）。
两次对照才把"实现错了"与"参数是恒等映射"分开。

### D-AB-04：拼接在视图层，状态里存两张独立帧

存拼接结果会让"切换布局"变成"重新渲染"。两张独立帧还让"关掉对比时"
不需要换数据源（换数据源会闪回旧帧）。

### D-AB-05：尺寸不同时不做重采样，居中贴进各自的半屏

A 侧是"原始尺寸"、B 侧可能被缩放到目标画布，而拼接要求同一像素网格。三条路：

1. 按较大的一侧画布居中摆放，小的一侧按其真实像素居中（**选它**）；
2. 缩放到相同尺寸再拼 —— 引入一次不必要的重采样，直接污染对比；
3. 拒绝拼接 —— 用户什么都看不到。

选 (1)：两侧的**像素内容**不做任何重采样（对比的可信度取决于这一点），
差异体现在"画布里的占比"上 —— 而那正是缩放变换的真实效果。

### D-AB-06：任一侧为空就两边都清

`clear_frame` 原本只清 A 侧。不清 B 的后果很隐蔽：下一次渲好时
会把**新的 A** 与**旧的 B** 拼成一对，而那两张图看起来都正常。
对比功能最不能给的错就是"拿两个不同时刻的画面对比"。

有两条用例分别钉"两侧都空"与"只有一侧空"。

## 实测证据

`crates/gui/tests/ab_compare.rs`（9 用例，真 libass）：

| 用例 | 断言 |
|---|---|
| `the_b_side_is_really_transformed_and_differs_from_a` | **核心**：120 nit 下 B 侧像素真的不同（0 差异即失败） |
| `an_identity_mapping_says_so_instead_of_looking_broken` | 203 nit 两侧相同，且注记说明是恒等 |
| `sdr_with_the_same_canvas_takes_the_no_op_path_and_says_so` | 短路路径两侧一致 + 注记写明"未做变换" |
| `a_different_canvas_scales_the_b_side_and_reports_the_change` | B 侧尺寸=目标画布，A 侧保持原尺寸，注记含 `640x360`→`320x180` |
| `transform_and_scale_can_both_apply` | 两件变换同时发生时都在注记里 |
| `toggling_ab_requests_a_render_instead_of_just_flipping_a_flag` | 开关打开时标记 `is_rendering` 并清旧对照帧 |
| `ab_frames_land_in_both_slots_with_the_note_and_provider` | A/B 各就各位，注记与 provider 落到位 |
| `a_failed_ab_render_clears_frames_instead_of_showing_a_stale_pair` | 失败清两侧 |
| `a_half_empty_ab_result_also_clears_both_sides` | 只有一侧空也要两边都清 |

探针实测数据（临时用例，已删）：

- `ReferenceWhite(120)`：`&H00808080` → `&H00646464`，2 个样式色被改，像素有差异；
- `ReferenceWhite(203)`：0 个样式色被改，文本无变化（恒等），像素 0 差异。

## 未做（诚实边界）

- **分界线的拖动**未做：规格只要求"左半/右半"，固定中缝满足它。
  可拖动分隔条是另一种交互（用户可以任意比例），规格没有要求。
- **并排的独立缩放/平移**未做：两侧按各自真实像素居中，
  没有"把 B 放大到与 A 等宽再看"这类查看器操作。
- **时间轴联动的对比**未做：A/B 用同一个时间点，不能"左边看 1 秒、右边看 10 秒"。
  规格没有要求它，而且那会带来"两个时刻能不能对比"的语义问题。
- **`time_adjust`（偏移/变速）不参与 B 侧**：spec 里没有该字段，
  与 `spec.rs` 里那条"未应用"说明一致 —— 也就是说 A/B 目前**不演示时间轴变换**。
  这是已知的能力缺口，而它影响的是"看不到时间轴变换的效果"，
  不是"看到了错误的变换"。

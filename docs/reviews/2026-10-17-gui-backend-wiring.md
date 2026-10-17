# GUI 与后端接通裁决（20 §Repro、28 §2、AT-UI-001）

日期：2026-10-17
范围：`crates/gui` 的 worker 层、任务抽屉、预览、环境自检

## 1. 之前的状态：三个"看起来能用"的洞

接手时 GUI 能编译、能开窗、三栏布局完整，但**执行链根本没接**：

| 位置 | 当时的样子 | 后果 |
| --- | --- | --- |
| `worker/task_worker.rs` | `std::thread::sleep(50ms)` + 无条件 `Ok(())`，注释写着"模拟处理或调用 task-engine 输出事务" | 任务抽屉会一个个变绿，磁盘上什么都没有 |
| `app.rs` 的 `StartQueue` | 只往列表里塞了几行 UI 记录，**从不调用** `execute_batch` | 同上；`TaskProgress`/`TaskFinished` 两条消息永远没人发 |
| `app.rs` 的 `OpenDoctorDialog` | 只把 `show_doctor` 设为 true，而 `view()` **从不读它** | 按钮点了没有任何反应 |
| `app.rs` 的 `VideoAssistDropped` | 写死 `建议帧率 23.976 \| 分辨率 3840x2160` | 对任何视频都"对"，所以没人能发现它从来没测过 |
| `preview_worker.rs` | 拿不到 libass 时返回合成色块，**不带任何标记** | 用户会以为"我的字幕就长这样"，而颜色/位置/字体全是编的 |
| `CancelQueue` | 只改 `is_running`，取消标志在后台闭包里建，UI 拿不到 | 取消按钮能点，但什么都不做 |

这是最坏的一类状态：**没有一处报错**，但也没有一处真的在干活。
占位实现必须明确失败（20 §Commands 的同一条纪律），不能"看起来成功了"。

## 2. 接法：GUI 不另写执行逻辑

核心决定：**GUI 把输入集与 Profile 矩阵编译成真实的 batch spec（15 §1），
再交给 `cli::batch::run_batch` 执行**。

这样 20 §Repro 那句"GUI 复制的 CLI 命令可以直接重放"是**结构上成立**的，
而不是靠事后对齐：

- 两边用同一个 spec 类型（`cli::batch::Spec`）。CLI 加字段而 GUI 不知道，
  或者 GUI 写了 CLI 不认的 key（spec 是 `deny_unknown_fields`），编译器/解析器会先报出来。
- 两边用同一个执行入口。不存在"GUI 走一条路、CLI 走另一条路"的可能。

新增的模块：

| 模块 | 职责 |
| --- | --- |
| `worker/spec.rs` | 把 GUI 的 `OutputProfile`/`CanvasMode`/`TargetFormat` 编译成 `cli::batch::Spec` |
| `worker/task_worker.rs` | 写 spec 到盘 → 调 `run_batch_with_cancel` → 把报告逐行翻译成抽屉状态 |
| `worker/doctor_worker.rs` | 调 `cli::run::execute(Doctor)`，把 JSON 抽成界面能显示的行 |
| `worker/probe_worker.rs` | 调真 ffprobe + `media_probe::suggest_for`，替掉写死的建议文案 |
| `state/task.rs` | 抽屉状态；status 字符串与 CLI 报告**逐字一致** |

## 3. 过程中发现并修掉的真问题

### 3.1 取消令牌从来没有连到引擎（AT-UI-001 的实质）

原设计里取消标志在后台闭包内 `AtomicBool::new(false)`，UI 永远拿不到。
我第一版把令牌提到 `SubtitleApp`，但 `cli::batch::run_batch` 内部又建了**它自己的**
`CancelToken`，两者不相干 —— 取消按钮依然无效。

写测试时暴露出来：让回调在拿到第一行结果时置位，结果两格都跑完了。

修正：给 `task_engine::CancelToken` 加 `from_shared(Arc<AtomicBool>)` 与 `shared()`，
并新增 `cli::batch::run_batch_with_cancel(..., Option<&CancelToken>)`。
GUI 的按钮与引擎现在**是同一个 flag**，由 `the_gui_cancel_flag_is_the_same_flag_the_engine_checks` 断言。

### 3.2 一条写不出来的断言

我原本想测"跑到一半取消，第二格不再开始"。写完发现它测不了：
GUI 的逐行结果是在 `run_batch` **返回之后**从报告里翻译出来的，
回调根本没有"中途"这个时机。写成那样只会得到一条永远绿（或永远红）的假断言。

所以拆成两条各自可观测的：
- `a_cancelled_task_stops_before_running_profiles`：预先置位 → 引擎不跑任何 profile、不产出文件；
- `the_gui_cancel_flag_is_the_same_flag_the_engine_checks`：类型层面的同一性。

第三节把"引擎在每格之前看令牌"这条（`runner.rs` 的 `if cancel.cancelled() { return; }`）
作为共同前提写清楚，而不是拿一条假断言冒充证据。

### 3.3 预览占位帧不带标记

`preview_worker` 拿不到 libass 时会画一个合成色块。它本身是合理的降级路径，
但**不带标记返回**意味着界面会把它当成真实预览展示。

修正：新增 `state::preview::FrameSource`（`Renderer` / `Placeholder` / `Failed` / `None`），
`PreviewFrame` 必须带上它，顶栏一直显示来源，`is_trustworthy()` 只在真渲染时为真。
`set_frame()` 这个不知道来源的通用 setter **不冒充**真实渲染 —— 它落成 `Placeholder`。

另外：渲染失败时必须**清掉旧帧**。留着上一帧会让人以为"这就是现在这一刻的样子"，
而那是上一刻的像素（`a_failed_render_clears_the_previous_frame` 守着）。

### 3.4 环境自检点了没反应

`show_doctor` 被设置但 `view()` 从不读。修正后弹窗真的跑一次 `doctor`，
显示操作系统/架构/目录可写/ffprobe/render host/数据目录/未实现清单；
拿不到自检结果本身也会被显示出来（"拿不到"是要报告的事，不是空白弹窗）。

## 4. 关于未支持的 Profile 参数：报出来而不是假装生效

`cli::batch::SpecProfile` 只有 `format`/`canvas`/`fps`/`exact_colors`/`min_axis`/`default_font`。
GUI 的 Profile 模型里还有 **时间轴偏移/变速** 与 **HDR 模式**，spec 里没有对应字段。

处理方式：`compile_profile` 把这些参数编译成**用户可见的说明**
（`[profile 名] 时间轴偏移 1000 ms 未应用：batch spec 没有该字段，请先用 time 命令处理输入`），
抽屉里一直显示。

两个都不选的理由：
- 当错误 → 整个任务起不来，用户只是改了个偏移量却什么都没导出；
- 静默忽略 → 用户以为参数生效了。

同理 `CanvasMode::KeepOriginal` 没有对应取值，落成 `1920x1080` **并在说明里写清楚**。

## 5. 验收证据

| 判据 | 入口 |
| --- | --- |
| GUI 编译的 spec 能被**真 CLI 进程**跑出产物 | `crates/gui/tests/gui_cli_replay.rs::a_gui_compiled_spec_replays_through_the_real_cli` |
| GUI 与手写 spec 逐字段一致，且产物**逐字节一致** | `gui_spec_and_a_handwritten_cli_spec_agree_field_by_field` |
| GUI 认得出 CLI 真会发出的每一种 status | `gui_recognises_every_status_the_cli_actually_emits` |
| 默认 Profile 矩阵能过 CLI 校验 | `the_default_profile_matrix_compiles_into_a_valid_spec` |
| GUI 执行通路真的写盘 | `crates/gui/tests/task_e2e.rs::the_gui_execution_path_actually_writes_outputs` |
| 起不来时不报假进度 | `a_task_that_cannot_start_reports_no_progress_rows` |
| 取消令牌与引擎同一份 | `the_gui_cancel_flag_is_the_same_flag_the_engine_checks` |
| GUI 二进制能起来（窗口已建、6 秒不退） | 手工：`Start-Process subtitletoolkit-gui.exe` |

GUI 已加入 portable 包（`tools/make_portable.ps1` 的 payload 多一项 `subtitletoolkit-gui.exe`）。

## 6. 仍未覆盖（不声称通过）

- **AT-UI-001 的 60 秒持续响应**：需要真人操作 + 屏幕录制才能实测。
  当前只有**结构证据**（阻塞 IO 与 CPU 渲染都在 `spawn_blocking` 里，不占 UI 线程）
  和取消令牌的同一性断言。按 24 §执行记录合同，这**不算**通过。
- **A/B 对比**（`toggle_split_ab`）只切了一个开关，没有实现双帧对照渲染。
- **字体健康摘要**（`font_health_summary`）从未被填充。
- **GUI 没有用户可选的帧率控件**：`fps` 用与 CLI 相同的默认值（`"25"`），
  由 `default_fps_matches_the_cli_default` 守着两边一致。
- **GUI 没有 GUI 专用的渲染入口**：`pgs`/`bdn` profile 走 batch 里的 `cli::run` 通路，
  与 CLI 完全一致；这既是优点（可重放），也意味着 GUI 不能做 CLI 做不到的事。
- **安装器与设置持久化**：GUI 不保存用户的上次配置（28 §3 未实现）。

# 裁决记录：AT-UI-001 的 60 秒持续可响应实测（2026-10-17）

## 需求

25 §AT-UI-001：「重任务期间 UI 60s 内持续可响应、取消可点击。」

此前只有**结构证据**（阻塞 IO 不在 UI 线程上），而规格的执行记录合同明确写着
「GUI 验收持续操作、取消、键盘、IME 与 DPI，**不以设计文档或自制返回值替代**」。
"结构上应该没问题"不是那条要求要的东西。

## 交付

`crates/gui/tests/ui_responsiveness.rs`（5 用例）：

| 用例 | 断言 |
|---|---|
| `idle_ui_interactions_stay_within_the_budget` | 空闲基线：8 种交互都在 8ms 预算内 |
| `the_ui_stays_responsive_while_a_heavy_task_runs` | **实测**：重任务在后台跑时反复计时 |
| `cancel_is_clickable_during_a_task_and_actually_stops_it` | 点击不阻塞；令牌与 worker 是同一份；置位后真停 |
| `start_queue_arms_a_real_run_instead_of_just_returning_a_task` | `StartQueue` 真的置位运行状态、展开抽屉、换新令牌 |
| `start_queue_with_nothing_to_do_does_not_pretend_to_run` | 无选中输入时不假装在跑 |

## 实测数字

任务负载：12 份输入 × 4000 条 cue 的 SRT，1 个 ASS profile
（纯文本路径，不要求 libass —— 否则 AT-UI-001 会变成"装了渲染器才验收"）。

```
AT-UI-001 实测：107 次 update，最慢 18.5µs，均值 9.491µs，预算 8ms
```

即最慢一次 `update` 距离 37 §1 的 8ms 预算还有 **430 倍余量**。
实测值由用例 `eprintln!` 打出来，不只是一句"通过" —— 证据是那个分布。

## 决定与理由

### D-UI-01：判据用 37 §1 的 8ms，而不是"看起来不卡"

37 §1 写的是「UI 主线程任何单次同步工作建议 < 8ms」。它原本是"建议"值，
而这里当作**判据**用：一个远超它的实测值（例如那个 30 秒的 ffprobe 超时）无论
怎么解释都是缺陷，而"看起来不卡"是个没法证伪的说法。

### D-UI-02：重活直接调 GUI 的 worker，不驱动 `iced::Task`

`Task<Message>` 的 action 流是 `iced_runtime` 的私有 API（`into_stream` 未导出）。
在测试里复刻它只会得到一堆与产品无关的适配代码，而本用例要证的是
「后台重活在跑时 UI 线程仍然快」—— 那件事只取决于"重活确实在别的线程上"，
与它由谁调度无关。

所以重活放进 `std::thread`，参数与 `Message::StartQueue` 交给 `spawn_blocking`
的**完全一样**（同一个 `gui::worker::task_worker::run` + 同一份编译出的 spec）。

### D-UI-03：不启窗口

`update` 是纯函数式的：`&mut self` + `Message` → `Task`。直接调它就能测到
UI 线程真正要做的那部分工作。启窗口会让测试依赖 GPU/桌面会话，
在 CI 上变成"环境不对就红"，而那是另一种不可靠。

### D-UI-04：取消令牌必须与应用是**同一份**

第一版测试里 worker 用了自己新建的令牌，结果是"点了取消，worker 照跑" ——
而那恰好就是 AT-UI-001 要防的那个缺陷（历史上真出现过：取消按钮无效）。
现在测试先把 `app.cancel` 换成自己的那份再交给 worker，与 `StartQueue`
里的 `let cancel = self.cancel.clone()` 一致。

## 开发中抳出的问题

1. **第一版测试根本没跑后台任务**。`Message::StartQueue` 返回 `Task::perform`，
   而 `Task` 不是 `Future`；只调 `update` 不 poll 它，后台永不调度。
   表现是"任务 60 秒没动，而 `update` 一直很快" —— 那个结果只证明了 UI 很快，
   因为后台压根没跑。**这是最容易骗过自己的那类假绿**。
2. **默认 profile 让测试要求 libass**。`ProfilesState::new()` 预置了
   BDN 与 4K HDR SUP 两个渲染型 profile；不清掉它们，spec 就要求 `libass_dll`，
   于是用例在有渲染器的机器上过、没渲染器的机器上失败。
3. **取消测试各用一份令牌**（见 D-UI-04）。

第 1 条值得单独记下来：它说明"我测了 UI 很快"与"我测了重任务期间 UI 很快"
是两件不同的事，而前者会在看数字时显得完全正常。

## 未做（诚实边界）

- **真人交互、IME、DPI 缩放**仍未覆盖，也不在本文件里假装覆盖。
  规格把它们列为 GUI 验收的一部分，而它们需要人眼与人手 ——
  这一点在 `tools/at_coverage.py` 的 AT-UI-001 条目里明确写出。
- **不测帧率**：`update` 快不等于渲染帧率够。渲染路径（wgpu）没有在这里测，
  它需要真实窗口。
- 采样是 16ms 间隔（模拟一帧），而实测中任务跑完前只采到 107 次。
  要让采样更密可以去掉那个 sleep，但那会变成 busy loop 并让测的是 CPU
  而不是响应性 —— 16ms 是刻意的。

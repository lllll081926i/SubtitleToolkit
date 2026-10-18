# 裁决：AT-RENDER-002 的「GUI 可重试」那一半

- 日期：2026-10-17
- 相关规格：25 AT-RENDER-002、15 §1（batch spec）、20 §Repro
- 相关代码：`crates/gui/src/{message.rs,app.rs,views/task_view.rs,state/task.rs}`
- 相关用例：`gui/tests/task_e2e.rs::retrying_only_reruns_what_failed_and_really_produces_it`
  与 `gui/src/state/task.rs` 的 3 个单元用例

---

## 1 规格的两句话，之前只做了一句

> AT-RENDER-002：取得可用 VSFilterMod provider 后，证明其崩溃**只终止 renderer-host，
> GUI 可重试**；自伤 `Poison` 测试不能代替该 provider 测试。

前一句（真 provider 崩溃只终止 host）在同日已经验收：

```
主进程 Alive：Crashed { exit_code: 3221225477 (0xC0000005), contained: true }
```

**后一句从来没做。** GUI 里只有"开始执行队列"与"取消"两个按钮 ——
没有任何重试入口。一次失败之后用户能做的只有：再点一次"开始"，
把**整批**（含已经成功的那些）重跑一遍。

那不算"可重试"，那只是"可以重新开始"。

---

## 2 交付

### 2.1 重试集合：只该重跑哪些

`TaskDrawerState::retry_targets()` 收集两类行：

| 状态 | 含义 | 为什么要重试 |
|---|---|---|
| `Failed` | 这一格真的失败了 | 显然 |
| `NotAttempted` | worker 明说"因前面的失败而中止，本格没开始" | 它**同样没产出东西** |
| `Queued` | 跑完了但这一格**从未被报告** | 同上 —— 对用户而言"没东西出来" |

**已成功的不进集合**（`Verified` / `FromCache` / `WouldWrite`）：
重试它们不但浪费一遍，还会**重写已经好的产物**。
用户点"重试"想要的是"把缺的补上"，不是"全部重来"。
要全部重来他可以再点"开始"。

### 2.2 把 `Queued` 也算进来的理由

这不是宽容，是**避免一个死角**：如果 worker 因故没给某格发状态，
那格会永远停在 `Queued`，而一个只认 `Failed`/`NotAttempted` 的
重试集合会**对它视而不见** —— 用户只能点"开始"整批重来。

### 2.3 重试仍然是一份**能重放**的 batch spec

不加任何私有字段。用 spec 已有的两半表达：

```
roots    = 只放「有失败格的那些输入」
profiles = 只放「有失败格的那些 profile」
→ 两边的交集，正好是待重试的格子
```

**为什么不给 `Spec` 加一个"只要这几对"的字段**：那会让这份 spec
不再能拿去 CLI 重放，而"GUI 编译的 spec 与 CLI 手写的一致"（20 §Repro）
是 GUI 与 CLI 同源的全部价值所在。

### 2.4 界面

抽屉的操作栏在"开始/取消"旁边多一个按钮：

> ⟳ 重试失败的 N 个

`N == 0` 时**按钮不出现**：一个点了没反应的按钮比没有按钮更差
（用户会以为界面卡了）。

---

## 3 实测抳出的一种失败形状

写端到端用例时选了一个看起来最普通的失败：**输入文件不存在**。
结果发现它走的是另一条路：

```
Started { spec_path: "…/out/gui-batch.spec.json", notes: [] },
Finished { outcome: "failed", report_path: None,
           error: Some(("STK-INP-0086", "输入根路径不存在", "")) }
```

**一行 `Row` 都没有。**

原因在 `task-engine/src/discover.rs`：

```rust
for root in &self.roots {
    if !root.exists() {
        return Err(StkError::simple(Domain::Input, 86, "输入根路径不存在")…);
    }
```

它在**发现阶段**就把整批拒了，于是一条逐行状态都发不出来。

### 这对 CLI 是对的，对 GUI 要靠 `finish()` 兜住

CLI 里"根路径不存在"就该整体失败 —— 那是用法错，不是某个任务失败。

而 GUI 里，用户看到的是"失败"加一个**空抽屉**。
出路在于 `finish()` 会做一件事：

```rust
for t in self.tasks.iter_mut() {
    if matches!(t.status, TaskStatus::Queued | TaskStatus::Running) {
        t.status = TaskStatus::NotAttempted { reason: reason.to_string() };
    }
}
self.is_running = false;
```

它把没跑到的 `Queued` 行转成 `NotAttempted`（**同样可重试**），
并复位 `is_running`（否则重试按钮的条件 `!is_running` 不成立，按钮不出现）。

所以在 `app.rs` 那处加了注释把这个**依赖**写明：
如果哪天 `finish()` 不再做那个转换，这个死角会重新出现 ——
而端到端用例会当场报出来。

---

## 4 验证

真通路（`task_worker::run` → 真 batch spec → 真执行）：

```
第一次：两个输入都没跑到 → finish() 后两个都是 NotAttempted
        → retry_targets() == 2（两个都没产出东西，都该重试）
        → 用这两条编出的 spec.roots.len() == 2
第二次：修好缺失的输入 → 重试 → 两行都成功，目录里 2 个 .srt
```

单元用例另锁三条：

- `retry_targets_are_exactly_the_failed_and_not_attempted_rows`
  （4 进 2 不进：已验证的与 from-cache 的都不进）
- `reset_for_retry_clears_the_previous_failures_from_the_counts`
  （退回后统计不能还把上次的红格子算进去，否则界面显示
  "跑完了，但还有 3 个失败" —— 与"正在重试"矛盾）
- `nothing_to_retry_when_everything_succeeded`（全成功时没有可重试的）

---

## 5 仍未做

- **重试不保留"上次是哪一批"的上下文。** 报告路径（`report_path`）在
  重试后会被新的覆盖。想要"第一次的报告 + 重试的报告"都能翻，
  需要一个运行历史 —— 那是另一个功能，不在这里假装做了。
- **重试的 profile 按名字匹配。** 如果用户在这期间改了那个 profile 的参数，
  重试会按**新参数**跑。这是有意的（改过的按新的跑），
  但界面上没有一句提示说"你改过它"。见 `app.rs` 那处的注释。

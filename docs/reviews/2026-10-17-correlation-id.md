# 裁决记录：22 §2 correlation ID（2026-10-17）

## 需求

22 §2 全句只有两句：「每个 Job/Output/renderer session 都有 UUID；
用户看到的错误卡可复制 ID。」

## 交付前的真实状态

**完全没有 correlation id**。`uuid_like()` 存在，但它是 `output.rs` 内部的私有函数，
只用来给临时文件名去重 —— 不出现在任何报告里。定位一次执行靠 PID 与输出路径，
而路径会被脱敏、PID 会被复用。

## 交付

- `core_model::corrid`：`CorrId` + `Scope` + `span_for`；
- 信封（`output.rs`）：顶层 `corr_id`，成功与失败都带；
- 人读输出：末尾单独一行 `corr-id: J-…`；
- 日志（22 §1 的联动）：span 自动带 `corr_id=…`；
- 批量报告：`corr_id`（本次执行）+ `output_ids[].corr_id` + `renderer_id`。

## 决定与理由

### D-CORR-01：格式是"可读前缀 + 共享段 + 唯一尾"

纯随机 UUID v4 能满足唯一性，但用户抄下来之后没法核对"这是我那次"。
`J-5c23f616-e1cb51fc9b3b` 这种形状里：前缀标出层级（一眼看出是哪一层），
中间那段是**本次 Job 的共享段**（同一 Job 的所有 Output 都有它，
于是"这三条是一家人"在字面上就成立），最后一段是各自的唯一尾。

不用 `uuid` crate：只需要 122 位随机性，而 `getrandom` 不在这棵依赖树上；
为 16 个随机字节引一条新依赖（21 §11 要审计来源与许可）不成比例。
Windows 上直接调 `BCryptGenRandom`（系统 CSPRNG，与 `rand` 库同一个底层），
其他平台退回时间+进程+计数器混合（splitmix64）。
**它不用于任何安全用途**，只要"两次运行的 id 不撞上"。

### D-CORR-02：日志与报告必须是**同一个值**

这是整件事的要害，也是开发中真正抳出缺陷的那一条。

第一版让 `Envelope::ok`/`fail` 各自生成 id，`dispatch` 又生成一个 ——
结果是"两边都像 UUID 但对不上"：用户把错误卡上的 id 贴出来，在日志里搜不到。
correlation id 的全部用途就是让这两处是同一个，看起来像不够。

所以 id 在 `dispatch` 生成一次，同时进 span 与信封。

### D-CORR-03：命令自己定的 id 优先（批量那一格）

批量路径会**自己**生成 id 并用它的子 id 标识每个输出（`output_ids` 的共享段就是它）。
如果 `dispatch` 无条件覆盖，报告里就会出现两个不同的 `corr_id`，
而 `output_ids` 的共享段对不上其中任何一个。

用一个显式的 `came_with_corr_id` 标记区分"这个 id 是命令自己定的"，
而不是靠猜（例如"看它是不是等于某个值"）。`with_corr_id` 只换显示值，
`claim_corr_id` 声明"由我定"。

### D-CORR-04：id 不携带用户数据

它会被贴进公开的 issue。所以结构上只由固定前缀（`JOR`）与十六进制组成，
没有路径/文件名/用户名。有一条测试专门用带中文与空格的目录名验证这一点，
另有一条对字符集做逐一断言。

### D-CORR-05：`job_id` 与 `corr_id` 是两回事

`job_id` 是**用户给的** spec 名字（"我这次任务叫什么"），
`corr_id` 是**本次执行**的 id（"这一次跑"）。合并它们会让同一个 spec 的多次运行
无法区分，而"定位到那一次"正是用户最需要的。

## 实测证据

单元测试（`core-model/src/corrid.rs`，6 用例）：

| 用例 | 断言 |
|---|---|
| `ids_have_the_documented_shape` | `J-8hex-12hex`，字符集受限 |
| `children_of_one_job_share_their_stem_and_differ_in_the_tail` | 三个子 id 共享 stem、各自 unique、前缀区分层级 |
| `independent_jobs_do_not_collide` | 2000 次不撞 |
| `parse_roundtrips_and_rejects_junk` | 往返一致；9 种畸形串被拒 |
| `an_id_carries_no_user_data` | 只含固定前缀 + 十六进制 + `-`，不含 `\ / :` 与空格 |
| `every_scope_has_a_distinct_prefix` | 三前缀互不相同 |

集成测试（`crates/cli/tests/correlation_id.rs`，6 用例）：

| 用例 | 断言 |
|---|---|
| `every_envelope_carries_a_well_formed_correlation_id` | 成功与失败的信封都有，形状合法 |
| `the_id_in_the_log_is_the_same_value_as_the_one_in_the_report` | **最重要的那条**：日志里搜得到报告里的 id |
| `two_runs_get_different_ids` | 两次执行不同 |
| `the_text_output_has_a_copyable_id_line` | 人读输出有一行 `corr-id: `，可直接抄，且与日志一致 |
| `a_batch_gives_one_job_id_and_one_id_per_output` | 信封 == `data.corr_id`；两个输出共享 Job 段 |
| `an_id_never_carries_user_data` | 中文+空格目录名不会渗进 id |

手工交叉验证：失败命令的信封 `corr_id=J-e3d573fc-fa94d024a74c`，
日志里那一行是 `ERROR stk{corr_id=J-e3d573fc-fa94d024a74c scope="job" …}: 执行失败 code=STK-INP-0110`
—— 逐字相同。

## 开发中抳出的两个真缺陷

1. **批量路径的信封与报告是两个不同的 id**（见 D-CORR-03）。
   集成用例 `a_batch_gives_one_job_id_and_one_id_per_output` 直接断言两者相等，
   改之前它报 `False`。
2. **文件日志里的 span 字段名带 ANSI 转义**。`layer.with_ansi(false)` 只关掉
   事件字段的着色，而 span 的 `corr_id=` 这类字段名走 `DefaultFields` 自己的开关。
   结果：22 §1 那条"日志文件里没有 ESC 字节"的断言（上一轮刚加的）当场红了。
   修法是给文件层换一个无 ANSI 的自定义 field formatter。

第 2 条是"上一轮的负基线在下一轮抓出新问题"的实例 —— 那条断言本身是对的，
而它拦住的是一个我刚引入的回归。

## 未做（诚实边界）

- **renderer session 的 id 只出现在批量报告里**。`renderer-host` 子进程自己的
  日志里没有这个 id（跨进程传递要在协议里加字段）。当前从 host 侧定位仍靠 PID
  与时间戳，报告里的 `renderer_id` 只能证明"这次执行用了哪个 session"。
- id 不进 `jobstate`（`.jobstate.json`）：恢复后的执行是新的一次跑，应当有新 id。
  当前行为正确（`JobState` 里没有这个字段），但没有一条测试明确钉这件事。

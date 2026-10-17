# 裁决记录：22 §1 结构化日志（2026-10-17）

## 需求

22 §1：`tracing` structured logging；默认 `%LOCALAPPDATA%/SubtitleToolkit/logs`；
滚动 7 天/容量限制。

此前完全未实现（没有 `tracing` 依赖，程序只往 stderr 写人可读的错误），
而 22 §5 的诊断包要收 `logs` —— 那一格一直是空的。

## 交付

- `core-model::logging`（`crates/core-model/src/logging.rs`）：初始化 + 按日轮转 + 启动清理；
- CLI 在 `main` 里于**参数解析之前**初始化（否则早期错误落不了盘）；
- `execute_ctx` 记开始/成功/失败三条结构化 event；
- `diagnose` 把 `logs/` 下所有日志收进诊断包。

## 决定与理由

### D-LOG-01：选 `tracing` 而不是 `log`

22 §1 写的是 "structured logging"，而 structured（带字段的 span/event）正是
`tracing` 相对 `log` 的全部差异。用 `log` 就只剩 `log::info!("{}", s)`，
要结构化只能自己拼字符串再解析 —— 那与 22 §3/§4 的结构化错误模型相抵。

`tracing` 与 `tracing-subscriber` 都是 MIT，符合 21 §1。默认特性关掉，
只开 `fmt`/`ansi`/`std`/`env-filter`/`registry`，把无关传递依赖降到最低（依赖 493→501）。

### D-LOG-02：同时保留 stderr 与文件

22 §1 只说日志落盘，但 CLI 是**管道里的程序**：把日志完全藏进文件，用户在终端
看不到任何东西，而"为什么这次跑得这么慢"这类问题需要现场看。

两份写的是同一个 event，不是两套日志。**文件那份不带 ANSI 颜色**：
它会被塞进诊断包，转义码在那里只是噪声；终端那份带颜色。有专门的测试
断言文件里不含 `0x1b`。

### D-LOG-03：不引 `tracing_appender` 的 rolling

那要再引一条时区链依赖，而这里需要的滚动规则只要"按日切文件 + 启动时清理"。
自己写两段小逻辑，能把**哪些文件会被删**写成可被测试断言的事实 ——
删文件是破坏性动作，不该藏在一个黑盒 appender 里。

清理**只碰自己生成的文件名**（`stk-YYYY-MM-DD.log` 与 `stk-YYYY-MM-DD.log.N`）。
`stamp_of` 对所有不匹配的名字返回 `None`，于是"不认识的一律不动"是代码结构
而不是一处需要记住的约定。有测试列出 7 种近似名（`app.log`、`stk-2026-13-01.log`、
`notes.txt` 等）确认都不被认领。

### D-LOG-04：级别来源与覆盖顺序

级别取自 36 §1 的 AppSettings `log_level`。`STK_LOG` 环境变量**优先于设置**：
与 `STK_DATA_DIR` 同一理由 —— 排查"为什么日志没出来"时，需要一条不依赖
设置文件的路径（设置文件本身可能坏、可能被写成 `off`）。

解析不了的级别退回 `info` 并**记一条警告**，不让程序起不来，也不静默忽略。

### D-LOG-05：日志里只记命令名与结果，不记路径

22 §5 要求诊断包**默认可以直接外发**。文件路径含用户名，把它写进日志会让
"日志可以外发"这个前提失效 —— 而日志恰恰是诊断包里唯一会持续变大的东西。
所以 `info!` 只带 `command`/`json`/`dry_run`/`has_out`，失败那条带
`code`/`title`（22 §4 的错误码，与用户看到的报告同一个标识）。

路径仍然可查：报告与 `doctor` 里都有；只是不落进日志。

### D-LOG-06：日志不是"用户内容"

`diagnose` 收日志时不标 `user_content`：它们是本程序自己的输出，且（按 D-LOG-05）
不含字幕文本。标成用户内容会让报告把"可以外发"的东西说成需要审查，
反过来损害那条标记的可信度。

## 实测证据

单元测试（`core-model/src/logging.rs`，5 用例）：

| 用例 | 断言 |
|---|---|
| `civil_date_conversions_match_known_values` | 日期换算与 Python `datetime` 独立核对过的值一致（含 2000-03-01 这个两月边界），且往返一致 |
| `only_our_own_file_names_are_recognized_for_pruning` | 7 种近似名都不被认领；同日轮转的 `.log.N` 认领 |
| `pruning_removes_only_old_or_excess_own_files` | 30 天前的删掉、今天的留着、别人的不动 |
| `pruning_enforces_the_total_size_budget_from_the_oldest` | 16 MiB → 压到 10 MiB 以内，最新的那份留着 |
| `an_unknown_level_falls_back_to_info_and_is_reported` | 6 个已知级别不报警，未知值报警并说出是哪个值 |

集成测试（`crates/cli/tests/logging.rs`，5 用例）：

| 用例 | 断言 |
|---|---|
| `both_success_and_failure_are_written_with_the_error_code` | 新进程写同一份；失败那条含 `STK-INP-0110` 与 `command="probe"` |
| `the_log_file_has_no_ansi_escapes` | 文件里没有 ESC 字节 |
| `the_settings_log_level_actually_filters` | `log_level=off` 之后不再有新记录 |
| `the_environment_override_beats_the_setting` | `STK_LOG=debug` 压过设置里的 `off` |
| `the_diagnostic_bundle_carries_the_logs_and_does_not_call_them_user_content` | 包里有 `logs/`、`contains_user_content=false`、内容是真日志 |

手工验证：`doctor` 留 `INFO ... 执行完成 command="doctor" ok=true`；
`probe` 一个不存在的文件留 `ERROR ... 执行失败 command="probe" code=STK-INP-0110 title=无法读取输入文件`。

## 开发中修掉的一个错

`days_from_civil(2026,10,17)` 的期望值我手算成 20728，实际是 **20743**。
实现是对的，**测试期望值错了**。用 Python 的 `datetime` 独立核对后改正，
并在注释里写明这些常量是外部核对过的而不是手算的 —— 日期算术是那种
"手算看起来合理、错一天也不会有人发现"的地方。

## 未做（诚实边界）

- **22 §2 的 correlation ID** 仍未实现。当前用 PID + 输出路径定位一次执行。
- **22 §6 的 SEH minidump** 仍未实现。
- 日志没有 JSON 输出模式：22 §1 只要求 structured，当前用 `tracing` 的
  紧凑文本格式（字段可机读），没有做 `--log-format json`。

# 裁决记录：22 §6 的崩溃现场记录（2026-10-17）

## 需求

22 §6：「单独 stdout/stderr capture；超时、exit code、SEH crash/minidump 分离记录。」

前三项此前已实现（`Host::log_tail`、`HostFailure::Timeout`、`HostFailure::Crashed`）。
缺的是第四项：主进程拿到的只有一句「退出码 3」，而查这类问题要知道的是
**「在做什么的时候死的、当时加载的是哪个 provider」**。

## 交付

- `crates/renderer-host/src/crash.rs`：`Tracker` + `CrashNote` + `read`/`summarize`；
- host 在阶段边界打点（启动 / 载脚本 / 出第 N 帧 / 收尾）；
- `HostSpec::crash_dir` → `STK_HOST_CRASH_DIR`（启动环境，不是控制面）；
- `HostFailure::Crashed { crash_note }` + `crash_summary()`；
- CLI 把它拼进错误卡的 details。

## 为什么不是真 minidump（D-CRASH-01）

完整 minidump 要 `DbgHelp` 的 `MiniDumpWriteDump`，而它有两条硬前提：

1. **必须在异常处理器里调用**，而那个处理器要满足一堆异步信号安全的约束
   （不能分配内存、不能拿锁、不能调大部分 Win32 API）。写一个**不正确**的
   异常处理器比不写更危险：它会掩盖真正的崩溃原因，或者在最需要它的时候二次崩溃。
   本仓库一以贯之的取向是"不自己实现加密/压缩/安全边界"（同 23 §XML、PNG deflate），
   这条属于同一类。
2. **dump 含进程内存镜像**。本产品处理的是**用户的字幕与内嵌字体**，而诊断包默认
   要能直接外发（22 §5）。含内存镜像的 dump 与那条承诺直接冲突。

所以做的是**崩溃现场记录**：host 在自己能控制的边界上（每个阶段前后）写下
「我在做什么、用的是什么」。它不含内存内容，但能回答"哪一步死的"——
而那正是绝大多数崩溃调查的第一个问题。

真 minidump 记成**未做**，而不是用半成品顶上。这与"占位实现必须明确报错"
是同一条纪律（20 §Commands）。

## 三条实现纪律

### D-CRASH-02：写盘走临时文件 + 改名

半截的 json 会让主进程在解析时失败，而那时用户已经丢了崩溃信息。
与其它产物同一条规矩（15 §6）。

### D-CRASH-03：写不进去不影响渲染

崩溃记录的用途是"事后来看"。让一个诊断文件的写失败把正常渲染搞坏，
是拿主功能去换附属功能。所以调用点一律 `let _ = tracker.write(...)`。

同理，`Tracker::new(None)`（拿不到目录）仍然工作，只是不落盘 ——
拿不到目录是常态（没设环境变量），它不该让 host 起不来。

### D-CRASH-04：起 host 之前先删旧记录

**这是本格里最危险的一格。** 真崩溃常常发生在写不出记录的地方
（访问违例、栈溢出），那时目录里如果还留着上一回的文件，报告会说出一个
**斩钉截铁但完全错误**的阶段与 provider —— 而调查会照着它走错方向。
**一份错的现场比没有现场坏得多。**

所以在 `Host::start` 里、`spawn()` 之前就 `remove_file`。
失败不算错（没有旧文件就是常态）。

### D-CRASH-05：目录由启动环境指定

`STK_HOST_CRASH_DIR` 与 `STK_HOST_CRASH` 同一类：**启动环境**而不是控制面。
客户端无法在运行时把它改到别处，也就不存在"让 host 往任意目录写文件"这条路径。

## 实测证据

单元测试（`crates/renderer-host/src/crash.rs`，8 用例）：

| 用例 | 断言 |
|---|---|
| `a_note_survives_a_write_read_roundtrip` | 字段一个不少 |
| `a_second_write_replaces_the_first` | 后写覆盖先写，且不留 `.tmp` |
| `without_a_directory_nothing_is_written_and_nothing_fails` | 没目录不落盘也不报错，内存现场仍可用 |
| `an_unwritable_directory_reports_an_error_instead_of_panicking` | 返回 `Err` 而不 panic |
| `a_missing_or_corrupt_note_reads_as_none` | 缺文件/坏 json/缺字段都是 `None` |
| `the_summary_names_the_phase_and_the_provider` | 摘要含阶段/序号/provider/上一成功动作；哈希截断显示 |
| `phase_labels_are_stable_strings` | 标签是 `"load-script"` 这种稳定串（进 json，改名会让旧记录读不出） |
| `a_phase_without_a_step_is_representable` | 启动/收尾没有序号时不该编一个 |

集成测试（`crates/renderer-host/tests/crash_record.rs`，4 用例）：

| 用例 | 断言 |
|---|---|
| `an_abort_leaves_a_parseable_note_naming_the_phase` | 退出码异常 + 记录可解析 + 阶段是 `startup` + 记录 < 4 KiB |
| `every_injection_kind_is_recorded_distinctly` | abort/exit/panic 三种各自原因不混淆 |
| `a_normal_host_startup_leaves_no_note` | **负基线**：不崩就不该有记录 |
| `a_stale_note_is_not_reported_as_this_run_crash` | 伪造的旧记录必须被删且不被当成现场的 |

**手工端到端验证**（真 CLI + 真 host + 注入）：

```
STK-RND-0091: renderer-host 终止（退出码 3221226505）
  - STK-RND-0204：Crashed { exit_code: Some(3221226505), …, 
      crash_note: Some(CrashNote { phase: "startup", …, reason: "注入 abort" }) }
  - 崩溃现场：阶段=startup；已运行=0ms；原因=注入 abort
```

`3221226505` = `0xC0000409`（STATUS_STACK_BUFFER_OVERRUN），即 abort 的真实退出码。

后续验证（同一目录）：先注入制造一份记录，再**去掉注入**重跑 ——
成功且 `host-crash.json` 已不存在。即 D-CRASH-04 那条在真实路径上生效。

## 未做（诚实边界）

- **真 minidump 未做**（见 D-CRASH-01）。这是**已知的能力缺口**，不主张已覆盖。
  它的实际影响是：面对访问违例这类崩溃，目前能知道"哪一步、什么 provider"，
  但拿不到栈帧与寄存器。
- 崩溃记录**不含调用栈**。要它就得在崩溃点捕获栈，而那同样落回
  "在异常处理器里做事" 的约束。—— 换句话说：**只要不做真 minidump，
  就没有安全的栈信息可用**，这一点是那整件事未做的代价，而不是遗漏。
- `hang` 注入不写记录（它在写记录之前就进了死循环）。它由 client 的超时用例覆盖，
  而超时本身有 `HostFailure::Timeout` 那条路。
- SSE/AVX 之类的硬件异常与托管异常没有单独分类：它们与 abort 一样表现为
  异常退出码，而记录只能说"死在哪个阶段"。

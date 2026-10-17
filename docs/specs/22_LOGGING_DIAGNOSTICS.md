# 日志、错误码与诊断

## 1. 日志
`tracing` structured logging；默认 `%LOCALAPPDATA%/SubtitleToolkit/logs`；滚动 7 天/容量限制。

**实现状态（2026-10-17）**：**已实现**（`core_model::logging`，`tracing` 0.1 + `tracing-subscriber` 0.3，
两者均 MIT，符合 21 §1）。

| 规格要求 | 落法 |
|---|---|
| `tracing` structured logging | `tracing` 的 event 字段，不拼字符串 |
| 默认 `%LOCALAPPDATA%/…/logs` | 走 `DataDir`，于是便携包内自动落在包内 `data/logs/` |
| 滚动 7 天 | 文件名 `stk-YYYY-MM-DD.log`（**本地**日期），启动时删 7 天前的 |
| 容量限制 | 启动时把总量压到 10 MiB 以内（从最旧的开始）；单文件超 2 MiB 同日轮转成 `.log.N` |

四点实现决定：

- **同时保留 stderr 与文件**。CLI 是管道里的程序，把日志完全藏进文件会让用户
  在终端看不到任何东西。两份写的是同一个 event，不是两套日志。
- **文件那份不带 ANSI 颜色**：它会被塞进诊断包（§5），转义码在那里只是噪声。
- **级别来自 36 §1 的 `log_level`**，`STK_LOG` 环境变量优先于设置（与 `STK_DATA_DIR`
  同一理由：排查“为什么日志没出来”需要一条不依赖设置文件的路）。
- **不引 `tracing_appender` 的 rolling**：那要再引一条时区链依赖，而这里只需要
  “按日切 + 启动清理”。自己写两段小逻辑能把“哪些文件会被删”变成可断言的事实 ——
  删文件是破坏性动作，不该藏在黑盒 appender 里。清理**只碰自己生成的文件名**
  （`stk-YYYY-MM-DD.log` 与 `.log.N`），不认识的路径一律不动。

日志里只记**命令名与结果**，不记文件路径：诊断包默认要能直接外发（§5）。

## 2. Correlation ID
每个 Job/Output/renderer session 都有 UUID；用户看到的错误卡可复制 ID。

**实现状态（2026-10-17）**：**已实现**（`core_model::corrid`）。

格式 `{层级前缀}-{8hex 共享段}-{12hex 唯一尾}`，前缀 `J`ob / `O`utput / `R`enderer。

| 规格要求 | 落法 |
|---|---|
| 每个 Job 有 | 信封顶层 `corr_id`（成功与失败都带） |
| 每个 Output 有 | batch 报告的 `output_ids[].corr_id` |
| 每个 renderer session 有 | batch 报告的 `renderer_id` |
| 错误卡可复制 ID | 人读输出末尾单独一行 `corr-id: J-…`（无空格、无路径，直接可抄） |

四点设计决定：

- **必须能对上**：日志里那一条的 `corr_id` 与用户报告里的**是同一个值**。
  两边各自生成过一版，结果“都像 UUID 但对不上” —— 用户贴出来的 id 在日志里搜不到。
  已有一条测试专门钉这件事（它抳出了批量路径的一个真缺陷，见裁决文档）。
- **必须能分层**：同一个 Job 的全部下级共享中间那段（`stem`），
  于是“这三条记录是一家人”在字面上就成立，按共享段分组即可聚一次执行。
- **不携带用户数据**：id 会被贴进公开的 issue，所以结构上只由固定前缀与随机字节组成，
  没有路径/文件名/用户名。
- **不引 `uuid` crate**：只需要“两次运行的 id 不撞上”，Windows 上走 `BCryptGenRandom`
  （系统 CSPRNG），拿不到时退回时间+计数器混合。它**不用于任何安全用途**。

`tracing` 的 span（22 §1）自动带上这个 id，所以后续每条 event 不必重复写。

## 3. 错误域
INPUT / PARSE / TRANSFORM / RENDER / FILTER / IMAGE / PGS / BDN / CONTAINER / OUTPUT / VALIDATE / DEPENDENCY。

**实现状态**：已实现，另有一域是本文原表未列的 **CACHE**（`STK-RCA-####`，37 §2 的磁盘渲染缓存）。
单独立域而不是并进 `OUTPUT` 的理由：输出失败意味着用户拿不到东西，缓存失败只意味着慢一点。
混在一个域里，读报告的人分不清「产物坏了」和「缓存坏了」。

## 4. 错误格式
`STK-PGS-0104` + short title + user action + technical details expandable。

**实现状态**：已实现（`core_model::StkError`）。跨 crate 的编号唯一性由
`crates/core-model/tests/error_code_uniqueness.rs` 把关。

## 5. 诊断包
zip 内容：app/version、OS、settings-redacted、job plan、logs、dependency versions/hashes、crash dump（若有）、**默认不包含原字幕/视频**；用户显式选择才附样本。

**实现状态**：**已实现**（2026-10-17）。CLI `diagnose`（`crates/cli/src/diagnostic.rs`）。

| 规格要求 | 当前实情 |
|---|---|
| app/version | `environment.json` 的 `app_version` + OS + arch |
| settings-redacted | `settings.json`；与路径/凭据相关的**键**替换为 `⟨已脱敏⟩` |
| job plan | `--job-state FILE` 时收进 `job-state.json`；未实现自动定位 |
| logs | **已实现**：`log_dir()` 下所有 `stk-*.log*` 都收进 `logs/` |
| dependency versions/hashes | `dependencies.json`（ffprobe / render host / libass DLL 的路径） |
| crash dump | **未实现**（需要一个能捕获 SEH 的宿主包装；当前只向 `renderer-host` 传 `STK_HOST_CRASH` 做注入测试） |
| 默认不含原字幕/视频 | 已实现，且报告里 `contains_user_content` / `user_content_entries` 明确说出来 |
| 用户显式选择才附样本 | `--include-input FILE`（可重复）；附上的条目放在 `input/` 下 |

三条实现纪律：

- **默认内容必须是可以直接外发的**。诊断包天生要被贴到 issue、发邮件，所以任何
  「反正先收进来，发之前再删」的做法都会在某个人身上失效 —— 那个人恰好是没读说明、
  直接拖进邮件窗口的用户。因此默认是白名单，不收任何来自用户内容的东西。
- **脱敏判据是键名而不是值**。值的形态无穷无尽，键名是有限且可枚举的。
  脱敏后保留 `键=⟨已脱敏⟩` 而不是删掉整行：「这个设置存在但没给你看」与
  「没有这个设置」是两件不同的事，支持人员需要能区分。
- **包自己说明自己的边界**。包里有一份 `README.txt` 写明默认不含用户内容、
  `input/` 是用户显式附上的。收包的人不必去猜。

## 6. renderer-host
单独 stdout/stderr capture；超时、exit code、SEH crash/minidump 分离记录。

**实现状态**：部分。超时、exit code 与「host 死了」的区分已实现
（`renderer-host` 的 `HostFailure` 变体）；**SEH crash 的 minidump 未实现**。

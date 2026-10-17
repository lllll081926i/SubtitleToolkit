# 日志、错误码与诊断

## 1. 日志
`tracing` structured logging；默认 `%LOCALAPPDATA%/SubtitleToolkit/logs`；滚动 7 天/容量限制。

**实现状态（2026-10-17）**：**未实现**。当前没有 `tracing` 依赖，程序只往 stderr 写人可读的错误；
日志文件路径与滚动策略都还没落地。`diagnose` 报出的 `logs_dir` 是**位置上应当**的目录，
不代表现在真有日志写在那儿。本节不能当作已实现来读。

## 2. Correlation ID
每个 Job/Output/renderer session 都有 UUID；用户看到的错误卡可复制 ID。

**实现状态**：**未实现**。当前用 PID 与输出路径定位一次执行（`diagnose` 的包里就有），
没有逐 Job 的 UUID。

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
| logs | **未实现**（§1 还没有日志文件可收） |
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

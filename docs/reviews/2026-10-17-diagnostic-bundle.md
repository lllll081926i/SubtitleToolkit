# 裁决记录：22 §5 诊断包（2026-10-17）

## 需求

22 §5：诊断包 zip 内容含 app/version、OS、settings-redacted、job plan、logs、
依赖版本与哈希、crash dump（若有），且**默认不包含原字幕/视频**；用户显式选择才附样本。

## 交付

- `crates/cli/src/diagnostic.rs`：`Bundle` + ZIP 写入器 + `verify_zip` 回读校验 + 脱敏；
- CLI `diagnose`（`--include-input FILE` 可重复、`--job-state FILE`、`--out`）；
- 默认输出 `<数据目录>/diagnostics/stk-diag-<pid>.zip`。

## 决定与理由

### D-DIAG-01：默认是白名单，不是黑名单

诊断包天生要被贴到 issue、发邮件。任何「反正先收进来，发之前再删」的做法都会在
某个人身上失效 —— 那个人恰好是没读说明、直接拖进邮件窗口的用户。
所以默认只收版本/平台/脱敏设置/依赖身份/缓存账目，**没有一样来自用户内容**；
要附样本必须点名 `--include-input`，且报告里 `user_content_entries` 单独列出来。

### D-DIAG-02：脱敏判据是键名而不是值

值的形态无穷无尽，键名是有限且可枚举的。脱敏后保留 `键=⟨已脱敏⟩` 而不是删掉整行：
「这个设置存在但没给你看」与「没有这个设置」是两件不同的事。

实际拦住了这个：`settings.json` 里的 `path` 含 `STK_DATA_DIR` 的真实路径（含用户名），
集成测试断言该片段**不出现**在包里。

### D-DIAG-03：包自己说明自己的边界

包里放一份 `README.txt` 写明默认不含用户内容、`input/` 是显式的。
收包的人不必去猜 —— 这也是「默认不外发」这条承诺的**可核对形式**。

### D-DIAG-04：手写 ZIP 而不是引 zip crate

`flate2` 已在依赖树里（`image-core` 的 PNG 用它）。ZIP 的 stored/deflate + 中央目录
这部分格式稳定、有公开规范，而我们只需要写不需要读。为这一个用途引一个 zip crate
会多一条要审计来源与许可的依赖（21 §11），收益不成比例。

压缩本身仍交给 flate2 —— 与 PNG 那条「不自己实现 deflate」的理由一致。

**代价是必须自己验。** 于是有了两层独立证据：
`crates/cli/src/diagnostic.rs` 里的测试读取器（逐条解压 + 验 CRC，不复用写入侧代码），
以及**开发期用 Python `zipfile` 的第三方复核**（`testzip() -> None`，条目字节一致）。

### D-DIAG-05：产物必须回读校验

`pipeline::emit` 的 `verify_reparse` 传 `diagnostic::verify_zip`：确认 EOCD 位置、
条目数与中央目录的 `cd_size`/`cd_offset` 自洽、逐条走完正好落在中央目录结尾。
理由与其它产物一致：**能写不能读的产物不算写成功**。

## 开发中修掉的三个真缺陷（都由上述证据抳出）

1. **中央目录项漏写名字**。只在 `out` 里写了名字，`central` 里没写。结果是包开头的
   签名全对、而 EOCD 的 `cd_size` 偏小 26 字节。自家读取器在**第二项**才报签名不符，
   Python 报 `Truncated central directory`。若只有「写读自洽」的测试，这个包会一路绿。
2. **脱敏把非对象值抹成 `{}`**。`redact_settings` 早期实现对所有输入返回
   `Value::Object(...)`，于是 `default_canvas: "1920x1080"` 也变成 `{}` ——
   脱敏变成了破坏。改成非对象原样返回。
3. **长度常量断言写错**。`debug_assert_eq!(h.len(), 30 + name.len())`：头部函数
   **不含**名字（名字在头之后），这条断言本身是错的。已改为比较固定长度 30/46。

第 1 与第 3 条是同一类错误的两面：手写二进制格式时，**每个字段的宽度与顺序**
必须能被独立核对一遍，否则「看起来像 zip」和「是 zip」之间的差距不会有任何东西发现。

## 实测证据

单元测试（`crates/cli/src/diagnostic.rs`，8 用例）：

| 用例 | 断言 |
|---|---|
| `the_written_bytes_are_a_real_zip_that_an_independent_reader_can_open` | 独立读取器能解开两个条目并逐字节对上；`verify_zip` 也通过 |
| `an_empty_bundle_is_still_a_valid_zip` | 22 字节（只有 EOCD）且通过 `verify_zip` |
| `verify_zip_rejects_deliberately_broken_bundles` | **负基线**：太短/截断/条目数改大/偏移改错/签名破坏 五种都报错 |
| `incompressible_content_falls_back_to_stored_and_still_roundtrips` | 压不动时走 stored 仍能读回 |
| `a_path_traversal_entry_name_is_refused` | `../`、绝对路径、空名被拒；正常名通过 |
| `redaction_hides_sensitive_keys_but_keeps_their_presence` | 敏感键变 `⟨已脱敏⟩` 但键仍在；非敏感项原样 |
| `user_content_is_marked_so_it_can_be_reported` | `user_content_names` 只列显式附上的 |
| `an_unreadable_file_records_a_note_instead_of_failing` | 读不到的文件记说明而不是让整包失败 |

集成测试（`crates/cli/tests/diagnose.rs`，5 用例）：

| 用例 | 断言 |
|---|---|
| `a_default_bundle_is_a_valid_zip_without_any_user_content` | 默认 0 个 `input/` 条目；`environment.json` 含平台信息 |
| `settings_are_redacted_so_paths_do_not_leak` | 数据目录真实路径**不出现**在包里 |
| `input_is_included_only_when_explicitly_requested_and_is_flagged` | 不点名就不收；点名才收且报告列出 |
| `dry_run_does_not_write_the_bundle` | `--dry-run` 不落盘 |
| `an_unwritable_target_reports_a_real_error` | 写不进去时如实失败 |

独立第三方复核：把包写盘后用 Python 的 `zipfile` 打开 →
`testzip() -> None`（全部 CRC 正确），`read(namelist())` 与写入内容一致。

## 未做（诚实边界）

- **logs** 与 **crash dump** 没有收进包：22 §1 的日志文件本身还没实现（没有 `tracing`），
  而 minidump 需要一个能捕获 SEH 的宿主包装。两者都在 22 节里标注为未实现，
  而不是用空文件占位。
- **job plan 的自动定位**没做：需要 `--job-state FILE` 显式给。自动找「最近一次任务」
  要先定义「最近」跨几个进程/几台设备，而一个猜错的 plan 会让诊断走偏。
- **correlation ID**（22 §2）仍未实现，诊断包用 PID + 输出路径定位一次执行。

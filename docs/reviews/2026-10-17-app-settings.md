# 裁决记录：36 §1 AppSettings 落盘（2026-10-17）

## 需求

28 §4 把「settings/presets/logs 的落盘」列为便携包仍未实现的一项，
并注明「目前只有字体库缓存走数据目录」。36 §1 的配置分层里 AppSettings 是最底层：
ToolDefaults 与 Preset 都会从它取默认。

## 交付

- `crates/core-model/src/settings.rs`：`AppSettings` + `SettingsStore`（读/写/隔离坏文件）；
- CLI `settings show|init|set`（`crates/cli/src/run.rs` 的 `settings_cmd`）；
- 文件是 `DataDir` 里的 `settings.json`；`STK_SETTINGS` 可指定一个独立路径。

## 决定与理由

### D-SET-01：放在 core-model 而不是 CLI 或 GUI

AppSettings 是配置分层的最底层，两个前端都要读。各写一份，两份默认值迟早不一致，
而用户看到的正是「同一个设置在两个入口表现不同」这类报告。

### D-SET-02：未知字段必须保留（36 §3）

旧版程序读一份新版写的设置、原样写回时若丢掉不认识的字段，
用户在新版里的选择就静默消失了 —— 两边都不报错。

**这与 batch spec 的 `deny_unknown_fields` 是相反的策略，也有相反的理由**：
spec 拼错字段会跑错任务（必须拒绝），设置里有陌生字段只说明它来自别的版本（必须保留）。
把其中一条当成「全仓库口径」应用到另一处，两边都会做错。

实现上持有 `extra: serde_json::Map` 并在序列化时先写别人的字段、再覆盖受管字段。
顺序很重要：反过来会让 `extra` 里的同名键盖住当前值（有专门用例验这一点）。

### D-SET-03：单个字段坏不废整份

文件坏了（JSON 读不了）才退默认值；某个字段类型不对只影响那个字段。
设置是偏好，不是数据 —— 一个拼错的 `log_level` 不该让整份设置作废。

### D-SET-04：坏文件留证据再重建

解析失败时把原文件改名成 `settings.json.bad-<时间戳>`，而不是直接覆盖。
用户可能想看一眼自己丢的是什么。这是「不硬删用户数据」那条纪律在设置文件上的落法。

### D-SET-05：schema 只向前迁移

高于 `CURRENT_SCHEMA` 就**只读不写**并说明原因。新版写的设置里可能有旧版无法
表达的语义，强行写回等于降级破坏。`settings set` 在这种情况下直接拒绝（`STK-INP-0321`）。

### D-SET-06：写入原子

同目录临时文件 + 改名。半截文件会让下一次启动退回默认值，
而用户不知道自己刚设的东西为什么没了。

### D-SET-07：非法值在写入前就被拒

`settings set` 只接受受管字段名，并对取值做与读取端**同一套**校验
（帧率走 `Fps::from_label`、画布走 `WxH` 形式）。静默接受一个拼错的字段名，
用户会以为设置生效了 —— 而它其实写进了没人读的地方。

### D-SET-08：`init` 不覆盖已有文件

静默覆盖用户偏好是最容易被骂的那类缺陷。已有文件时 `init` 失败并给出
「直接编辑该文件，或用 `settings set` 修改」（`STK-INP-0320`）。

### D-SET-09：`default_canvas` 读时不要求 `Canvas` 能构造

`Canvas` 是有限枚举（09 §）。设置里可能存一个当前版本不认识的合法尺寸。
能判「形式对」就先收下，等真用它时由 `Canvas` 报「不支持这个画布」——
那时错误信息里有具体的任务上下文，比「设置文件第 5 行不合法」有用得多。

## 实测证据

单元测试（`crates/core-model/src/settings.rs`，9 用例）：

| 用例 | 断言 |
|---|---|
| `unknown_fields_survive_a_read_write_roundtrip` | 改受管字段后未知字段仍在，内容完整（不是只剩键名） |
| `a_corrupt_file_falls_back_and_keeps_the_evidence` | 退默认值 + 有问题说明 + `.bad-<ts>` 存在 + 原路径让开 |
| `one_bad_field_does_not_discard_the_others` | 坏字段退默认，好字段保留，只 1 条警告 |
| `invalid_fps_and_canvas_are_rejected_at_read_time` | 非法值在读时就被挡下，不留给「新建任务」那一刻 |
| `a_newer_schema_is_read_but_flagged_as_read_only` | 照读但报「只读不写」 |
| `a_missing_file_is_defaults_without_complaint` | 首次运行是默认值且**不报问题** |
| `a_saved_file_reads_back_identical` | 含 `ui_language: None` 的显式 null 往返 |
| `saving_leaves_no_temporary_file_behind` | 写完目录里只有 `settings.json` |
| `extra_cannot_shadow_managed_fields` | `extra` 里的同名键盖不住当前值 |

集成测试（`crates/cli/tests/settings.rs`，7 用例，每次调用都是新进程）：

| 用例 | 断言 |
|---|---|
| `a_setting_persists_into_a_later_process` | `set` 后**新进程**从 file 读到；`show` 不顺手建文件 |
| `unknown_fields_survive_a_cli_roundtrip` | 走真 CLI 往返后未知字段仍在；报告列出被保留的键名 |
| `init_refuses_to_overwrite_an_existing_file` | `STK-INP-0320`，原值原封不动 |
| `init_writes_a_default_file_when_none_exists` | 无文件时写出默认值 |
| `invalid_values_are_rejected_and_nothing_is_written` | 5 种非法取值全拒（`STK-INP-0322`）且不落文件 |
| `a_corrupt_file_degrades_instead_of_blocking` | `ok=true` + `fallback-after-error` + 证据文件在 |
| `settings_land_in_the_resolved_data_directory` | `STK_DATA_DIR` 下确实出现 `settings.json` |

## 未做（诚实边界）

- **ToolDefaults 与 Preset**（36 §1 的另外两层）仍未实现。Preset 需要 TOML 依赖获批
  （ISSUES E4），不在本次范围。
- **日志落盘**仍未实现（22 §的日志文件路径还没定，当前只有 stderr）。
- 参考实现里 AppSettings 命名空间的 `settings reset` 没做：删一个字段与
  手工编辑文件等价，加一条命令只会多一处需要维护的语义。

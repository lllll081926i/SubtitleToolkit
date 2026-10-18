# 裁决：ToolDefaults（36 §1 第二层）

日期：2026-10-17
范围：`crates/core-model/src/tool_defaults.rs`、`crates/cli/src/{args,run}.rs`、
`crates/cli/tests/tool_defaults.rs`
规格：`docs/specs/36_PRESET_CONFIG_SCHEMA.md` §1

## 1 填的是分层里的哪一格

36 §1 把配置分成五层。本次把**中间那层**从"未实现"变成"已实现"：

| 层 | 作用 | 之前 | 现在 |
| --- | --- | --- | --- |
| AppSettings | 全局偏好（日志级别、界面语言…） | 已实现 | 已实现 |
| **ToolDefaults** | **每个工具的默认参数** | **未实现** | **✅ 本模块** |
| Preset | 用户可命名、导入导出（TOML） | 未实现 | 未实现（依赖未获批） |
| JobSnapshot | 一次执行的完整冻结参数 | 已实现 | 已实现 |
| Plan | Planner 解析后的 stage DAG | 已实现 | 已实现 |

分层剩下未做的只有 **Preset**，而它卡在 36 开头的"TOML 依赖未获批
（ISSUES E4）"上 —— 那是一个**外部决策**，不是代码能解决的。

## 2 设计决定

### 2.1 只登记少数几项，不给"任意键值对"开口子

`Tool` 枚举里只列 4 个工具（`pgs encode` / `render` / `font subset` / `mkv`），
每个工具只有一个**白名单**（`Tool::keys()`），且每项都带一句说明。

不做成"把 CLI 参数全存一遍"，因为那种做法有两个后果：

1. **它会与 CLI 的默认值分叉。** 用户在设置里改过 `pgs.level` 之后，
   CLI 的 `--help` 里写的默认值就不再是真的默认值了，而没人会去同步那两处。
2. **它会把每个新增参数都变成一次设置文件的 schema 变更。**
   加一个 `--new-flag` 就要考虑"旧的设置文件里没有它怎么办"。

**没登记的工具就是没有默认值可存。**

### 2.2 求值，而不是报"文件里有什么"

`tool-defaults show` 报的是 `effective`（实际生效取值）+ `allowed_keys`（白名单）
+ `unknown_keys`（文件里有但不生效的键）。

理由：用户问的是"我设的生效了吗"，而"文件里有这一条"不回答那个问题 ——
它可能因键名拼错而不被采纳。

### 2.3 拼错的键在**写入前**就拒

`set` 会检查键是否在白名单里，不在就报 `STK-INP-0218` 并列出可用的键。

允许写进去的后果：它存得下来但**不生效**，于是用户看到"设置成功了"
却看不到任何变化 —— 而那正是最难查的一类问题。

（读的时候仍然保留不在白名单里的键并归到 `unknown_keys`：那种情况来自
**别的版本**写的文件，不是本次输入，36 §3 要求保留。）

### 2.4 不补 CLI 的默认值

设过的键才出现；没设过的不出现。调用方（CLI 的 `run.rs`）据此沿用
**CLI 自己的**默认值。

在这个模块里补一份 CLI 的默认值，就等于造出第二份默认值定义 ——
而两份定义分叉的后果正是 §2.1 里写的那个问题。

### 2.5 工具名与子命令名同源

`Tool::name()` 返回的字符串与 `args.rs` 的 `Command::name()` 同源
（`pgs encode` 而不是 `pgs_encode`）。这样"设置里写的工具名"与
"报告里写的命令名"是同一个字符串，用户能把两处对上。

有专门用例（`every_tool_name_matches_the_cli_command_name`）钉住这一点。

### 2.6 与 AppSettings 共用纪律，不共用代码

共用的是**纪律**（坏文件退回默认值、未知字段保留、原子写、坏文件留证据），
**不共用代码**：两份文件是两个独立的 schema，抽公共部分会把它们绑在一起，
而它们本来就该能各自演进。

真正复用的只有 `SettingsAction` 那个三态枚举（show/init/set）—— 两个命令的
动作集**恰好**一样，而复用它是安全的。

### 2.7 值的类型不一律当字符串

`true`/`false` 存布尔，能当整数解析的存数字，其余存字符串。

理由：数字与布尔的区别会丢，而下游每次都要重新解析一遍。
用例 `numeric_and_boolean_values_keep_their_type_on_disk` 直接读磁盘上的
原文断言 `"min_axis": 8`（不是 `"8"`）。

### 2.8 `BTreeMap` 而不是 `HashMap`

写回时的字段顺序必须稳定，否则同一份设置每次保存都会产生一个
"内容相同但字节不同"的文件，让 diff 与哈希比较失去意义。
用例 `serialization_is_stable_across_runs` 钉住。

## 3 与 AppSettings 的三条共同底线（沿用，理由见 `settings.rs`）

1. **文件坏了不能让人开不了程序**：退回默认值 + 说出失败 + 原文件改名留证据。
2. **未知字段保留**（36 §3）：旧版读新版写的文件、写回时不丢字段。
   用例 `unknown_top_level_fields_survive_a_round_trip` 读回原文断言。
3. **`schema` 高于本程序就只读不写**，并说明原因（避免降级破坏新版字段）。

## 4 抳出的问题

**一个 clippy 真问题**（在测试里）：`run(dir: &PathBuf, ...)` 应当是 `&Path`。
`cargo clippy -- -D warnings` 当场报出。已改。

**一个自己造的编译错误**：在 `USAGE` 常量里写
`tool-defaults set "pgs encode" level standard`，其中的裸引号
结束了 Rust 字符串字面量。这类错误好在编译器立刻报，不需要运行才发现。

## 5 验证记录

### 5.1 行为（真二进制，8 种输入）

```
show（文件不存在）  → 退 0, source=内置默认值, effective 全空, allowed_keys 全列出
init                → 退 0, 写出 td.json
init 再来一次       → 退 1, STK-INP-0320「已存在，init 不会覆盖它」
set pgs encode level standard → 退 0, effective={level:"standard"}
set pgs encode levle x        → 退 1, STK-INP-0218 并列出可用的键
set nope level x              → 退 2（用法错，工具名先校验）
set render min_axis 8         → 退 0, effective={min_axis:"8"}
show（改过之后）    → 退 0, source=设置文件, 两项都在
```

磁盘上的文件确实存的是 JSON 数字：

```json
{
  "schema": 1,
  "tools": {
    "pgs encode": { "level": "standard" },
    "render": { "min_axis": 8 }
  }
}
```

### 5.2 用例

- `crates/core-model/src/tool_defaults.rs`：12 个单元用例
- `crates/cli/tests/tool_defaults.rs`：8 个集成用例（走真二进制）
- `crates/cli/src/update.rs`：12 个单元用例（上一轮）
- `crates/cli/tests/check_updates.rs`：9 个集成用例（上一轮）

集成用例里刻意核的两条：**设置不漏到别的工具**、**拼错的键真的没写进去**
（不只是"报了错"，而是回查 `effective` 确认是空的）。

### 5.3 门禁

```
cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 79/79 目标，0 失败
cargo test -p core-model --test error_code_uniqueness → 4 passed
```

## 6 未完成（如实记录）

- **CLI 还没真的读这些默认值。** 本模块提供的是**存储与求值**，
  而 `pgs encode` / `render` 等命令**尚未**在"用户没给参数时"去查 ToolDefaults。
  这是关键的一步，不做的话这层就只是"能存能看"，不是"真的生效"。
  见下一节说明为什么本次不一起做。
- **Preset 未实现**，卡在 TOML 依赖审批（ISSUES E4）—— 外部决策。
- **GUI 无入口**。

### 6.1 为什么"让命令真的用上"要单独做

把 `pgs encode` 接到 ToolDefaults 需要改的是**每个命令的参数默认值取值点**，
而现在的取值点是编译期常量（`args.rs` 里 `Command::PgsEncode` 的字段在解析时
就被填成 `ValidationLevel::Standard`）。也就是说，解析阶段**看不到**
"用户到底给没给这个参数" —— 那正是"用默认值"与"用用户的值"的分界。

要做对，得让这些字段变成 `Option<T>`，再在执行阶段用
`用户值 → ToolDefaults → 编译期默认` 这个顺序求值。那是一次**跨多个命令**的
改动，会碰 `args.rs` 的每个命令定义与它们的断言，风险与本次的存储层不同量级。

**分开做**的理由不是"怕麻烦"，而是：存储层独立可验（本次已完成且证据完整），
而接入层的每一处都要单独验证"不写参数时确实变了、写了参数时确实没变"。
把两者混在一次改动里，会让"存储对了但接入漏了一个命令"这类错误难以定位。

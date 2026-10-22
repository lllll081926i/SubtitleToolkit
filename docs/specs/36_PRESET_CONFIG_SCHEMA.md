# Preset、配置与可重放任务 Schema

本文记录 TOML preset 与可重放任务的配置合同。CLI 已实现 `preset show|validate|example` 和 JSON batch spec；GUI 工作台已接通 batch 执行，但尚无 preset/设置入口。Preset 目前不能直接作为 `batch --spec` 输入。通用 Planner stage DAG 仍未实现；当前边界见[功能矩阵](03_FEATURE_MATRIX.md)。

## 1. 配置分层
- AppSettings：全局偏好；**已实现**（见下）；
- ToolDefaults：每个工具默认；**已实现（2026-10-17）**（见下）；
- Preset：用户可命名、导入导出；**已实现（2026-10-17）**（见下）；
- JobSnapshot：一次执行的完整冻结参数；**已实现**（JSON batch spec + `jobstate`）；
- Plan：Planner 生成并供执行/重放的实际 stage DAG；**未实现**。`core_model::profile::Plan` 只是无依赖边的被动阶段列表，尚未接入生产规划流程。

### Preset 的实现与边界（2026-10-17）

`core_model::preset`（`crates/core-model/src/preset.rs`）+ CLI `preset show|validate|example`。
格式是 **TOML**（规格 §2 的示例形式）。

解除 ISSUES E4：新增 `toml` 依赖。已在 crates.io 官方记录核实其许可为
**`MIT OR Apache-2.0`**（v0.9.12）—— 正是 21 §1 明确允许的宽松家族，
且与本项目自身许可相同。E4 当时写的就是“建议批准”，阻塞只是"依赖表里没有它"
这个**行政手续**，不是政策冲突。

#### “可搬运”决定了这个模块的大部分设计

因为一条 preset 会离开它被写下的那台机器。所以：

- **不存绝对 DLL 路径与哈希作为硬要求**（§4）：store provider **偏好**
  （`libass`/`csri`/`auto`），实际路径与哈希交给 JobSnapshot 记。
  把 `D:\某人的机器\x64\libass.dll` 写进 preset，收件人一定用不了。
- **未知字段保留**（§3）：旧版读新版写的 preset 再写回，不能把它读不懂的
  字段吃掉。
- **schema 高于自己时拒绝执行**（§6 第 1 条）：不猜含义。

#### 与 batch spec 的**相反**策略，以及为什么两者都对

batch spec 用 `deny_unknown_fields`：拼错字段**必须拒**，因为它会跑错任务。
preset 保留未知字段：陌生字段只说明它来自别的版本，**必须留**。

这不是不一致，而是**失败代价不同**：spec 猜错会产出错误的成品，
preset 猜错只会让下一次写回丢字段。

#### 三个动作各自回答一个问题

| 动作 | 回答 | 典型调用者 |
| --- | --- | --- |
| `show` | 里面是什么？ | 想知道这份配置会产出什么的人 |
| `validate` | 能不能用？ | CI / 提交前检查（只要退出码） |
| `example` | 我该照什么写？ | 第一次写 preset 的人 |

合一个 `show` 会让 CI 被迫解析人的输出。

#### 两处实测抳出的真问题

1. **`hdr.mode` 不是扁平键。** 第一版把 `HdrTarget` 写成 `#[serde(flatten)]`，
   而规格示例里的 `hdr.mode = "off"` 在 TOML 里是**嵌套表的语法糖**
   （等价于 `[outputs.hdr]` + `mode = "off"`），flatten 根本看不到它。
   实测报 `unknown field hdr`。另外 serde 的 `deny_unknown_fields`
   与 `flatten` 本来也不能共存。改成嵌套表后两边都对。
2. **`preset example` 存出来的文件自己读不了。**
   第一版除了样板还打了一份 JSON 信封，于是
   `preset example > my.toml` 存出来的前半是 TOML、后半是 JSON，
   再拿回来 `validate` 报“第 20 行 key 没有值”。
   而 `example` 的**全部用途**就是“原样拿走存成文件”。
   修法：给 `Envelope` 加一个显式 `raw_stdout` 标记（而不是在 `emit` 里
   按命令名特判 —— 特判会让看 `emit` 的人不知道哪几个命令是特例）。

#### 下一步（仍未做）

- **`batch --preset` 的直接接入。** §6 的合成顺序是“默认 → preset → 用户显式值”，
  而 `batch` 的 JobSnapshot 是**已冻结**的一批任务 —— 把 preset 塞进去要么
  改写已冻结的字段（破坏“可重放”），要么只说“我读过了”（等于没接）。
  所以本模块把 preset **解析成一组 `OutputTarget`**，由调用方决定怎么用。

### ToolDefaults 的实现与边界（2026-10-17）

`core_model::tool_defaults`（`crates/core-model/src/tool_defaults.rs`）
+ CLI `tool-defaults show|init|set`。文件是 `DataDir` 里的 `tool-defaults.json`；
`STK_TOOL_DEFAULTS` 可指一个独立路径。

与 AppSettings 的分工不是重要性而是**使用范围**：AppSettings 管“两个前端
共用的显示/运行偏好”，ToolDefaults 管“某个子命令不写参数时用什么值”。
日志级别全局只有一个，而 `pgs encode` 的默认 `--level` 与 `render` 的默认
`--min-axis` 各管各的。

四条边界：

- **只登记少数几项，不给“任意键值对”开口子**。`Tool` 枚举里只列 4 个工具，
  每个工具一个白名单（`Tool::keys()`）。把 CLI 参数全存一遗会有两个后果：
  它会与 CLI 的默认值**分叉**（用户改过后 `--help` 里的默认值就不真了），
  它会把每个新增参数都变成一次 schema 变更。**没登记的工具就是没有默认值可存**。
- **`show` 报的是实际生效取值（`effective`），不是“文件里有什么”**。
  用户问的是“我设的生效了吗”，而“文件里有这一条”不回答那个问题。
- **拼错的键在写入前就拒**（`STK-INP-0218`）：允许写进去会存得下来但不生效，
  用户看到“设置成功”却看不到变化。读的时候仍保留白名单外的键（那来自别的版本）。
- **不补 CLI 的默认值**：设过的键才出现，否则调用方沿用 CLI 自己的默认值。
  在这里补一份 CLI 默认值就等于造出第二份定义，而两份定义会分叉。

与 AppSettings **共用纪律不共用代码**：坏文件退回默认值并留证据、未知字段
保留、原子写、schema 高于本程序只读不写——这些都一样；但两份文件是两个
独立的 schema，抽公共部分会把它们绑在一起。

**已接入命令（2026-10-17）**：`pgs encode`（fps/canvas/level/default_font）、
`render`（fps/canvas/default_font/min_axis）、`mkv`（tools_dir）、
`font subset`（subset_dir/fallback_full）。

求值顺序是 `用户值 → ToolDefaults → 编译期默认`（`crates/cli/src/defaults.rs`）。
两个关键实现选择：

- **解析阶段只记“用户给了什么”**：`pgs encode`/`render` 的那几个字段是
  `Option<T>`。在解析阶段就填成具体默认值会让执行阶段看不出“用户到底给没给”，
  而那正是“用用户的值”与“用默认值”的分界。
- **报告新增 `resolved_from`**：逐项报出“命令行 / tool-defaults / 内置默认”。
  用户问的是“我设的生效了吗”，而只有取值不回答那个问题。

**`batch` 明确不接**：spec 里的值是已解析的确定值，spec 自己就是完整的参数快照
（36 §1 的 JobSnapshot 层）。让它查用户设置会让同一份 spec 在不同机器上跑出
不同结果，而“可重放”正是 spec 的用途。

**开关不进 ToolDefaults**（exact_colors / embed_fonts / snap_pts）：
开关的语义是“这次要不要做”，而不是“过去我偏好什么”。

详见 [接入层裁决](../reviews/2026-10-17-tool-defaults-wiring.md)。

### AppSettings 的实现与边界（2026-10-17）

`core_model::SettingsStore`（`crates/core-model/src/settings.rs`）+ CLI `settings show|init|set`。
文件是 `DataDir` 里的 `settings.json`；`STK_SETTINGS` 可指一个独立路径。

它的存在理由是 28 §4 把“settings 落盘”列为未实现项，而 AppSettings 是分层的**最底层**：
ToolDefaults 与 Preset 都会从它取默认。放到 CLI 或 GUI 里各写一份，两份默认值
迟早不一致，而用户看到的正是“同一个设置在两个入口表现不同”这类报告。

与本文其他部分的三个关键差异：

- **未知字段保留**（§3）在这里是真行为，不是目标：`settings show` 会列出
  `preserved_unknown_fields`，写回时原样合并。这与 batch spec 的 `deny_unknown_fields`
  是**相反**的策略，因为两者的失败代价不同：spec 拼错字段会跑错任务（必须拒），
  设置里有陌生字段只说明它来自别的版本（必须留）。
- **schema 只向前迁移**：高于本程序的 schema 只读不写（避免降级破坏新版字段）。
- **坏文件不自伤**：退回默认值 + 说明原因 + 原文件改名保留。设置是偏好，不是数据。

## 2. Preset 示例
```toml
schema = 1
name = "1080p ASS + Basic PGS"

[[outputs]]
name = "1080p-ASS"
format = "ass"
canvas = [1920, 1080]
fps = "24000/1001"
hdr.mode = "off"
suffix = ".1080p"

[[outputs]]
name = "1080p-Basic-SUP"
format = "pgs"
canvas = [1920, 1080]
fps = "24000/1001"
renderer = "libass"
pgs.validation = "basic"
suffix = ".1080p.basic"
```

## 3. Schema 迁移
每个配置都有 `schema`；只向前迁移，迁移前备份。未知字段保留，便于新版/旧版往返。

以上为未来 preset 的迁移策略。当前 JSON batch spec 使用 `deny_unknown_fields`，拼错字段直接报错；不能将未来的未知字段保留要求当成当前解析器行为。未来保留字段不得参与执行；无法理解 schema 或必需能力时拒绝运行并说明原因。

## 4. Renderer identity
Preset 不存绝对 DLL hash 作为硬要求；存 `renderer = special-auto` 或 provider preference。JobSnapshot 才记录实际 path/hash/version。

## 5. CLI/GUI 共用
GUI 不维护第二套参数；Preset 与 CLI schema 完全同源，避免 GUI 与核心功能漂移。

## 6. 目标解析与校验顺序

1. 解析 schema 与字段类型；不支持版本时停止，不猜测含义。
2. 按“默认 → preset → 用户显式值”合成每个 profile；视频探测只提供未手动指定字段的建议。
3. 归一化精确 FPS、画布、颜色参数与输出路径。每个执行字段保留来源。
4. 校验实际 capability；未实现的 renderer、Strict 或字体子集不可因配置中出现而启用。
5. 展开所有目标路径并检查碰撞，生成不可变 JobSnapshot 与 Plan。

Preset 存可移植的意图与 provider 偏好；执行快照记录实际路径、二进制/字体摘要、算法版本与应用版本。可重放不代表跨机器像素必然相同，重放前必须核对依赖身份。

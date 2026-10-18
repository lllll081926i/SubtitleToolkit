# Preset、配置与可重放任务 Schema

本文是目标 TOML preset 草案。当前 CLI 已有 JSON batch spec 和 `.jobstate.json` 恢复凭据，不读取或写入 TOML preset；GUI 与统一 JobSnapshot 尚未实现。草案不能作为 `batch --spec` 输入。当前可执行配置见 [CLI 规格](20_CLI_AUTOMATION.md)，实现范围见[功能矩阵](03_FEATURE_MATRIX.md)。

## 1. 配置分层
- AppSettings：全局偏好；**已实现**（见下）；
- ToolDefaults：每个工具默认；**已实现（2026-10-17）**（见下）；
- Preset：用户可命名、导入导出；**未实现**（TOML 依赖未获批，见 ISSUES E4）；
- JobSnapshot：一次执行的完整冻结参数；**已实现**（JSON batch spec + `jobstate`）；
- Plan：Planner 解析后的实际 stage DAG；**已实现**。

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

**仍未做（关键）**：CLI 命令**尚未**在“用户没给参数时”去查 ToolDefaults。
本模块提供的是存储与求值；**接入**需要把各命令的参数字段改成 `Option<T>`
再按 `用户值 → ToolDefaults → 编译期默认` 求值，那是一次跨多个命令的改动。
分开做的理由：存储层独立可验（已完成），而接入层每一处都要单独验证
“不写参数时确实变了、写了参数时确实没变”。详见
[tool-defaults 裁决](../reviews/2026-10-17-tool-defaults.md)。

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

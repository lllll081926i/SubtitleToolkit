# Preset、配置与可重放任务 Schema

本文是目标 TOML preset 草案。当前 CLI 已有 JSON batch spec 和 `.jobstate.json` 恢复凭据，不读取或写入 TOML preset；GUI 与统一 JobSnapshot 尚未实现。草案不能作为 `batch --spec` 输入。当前可执行配置见 [CLI 规格](20_CLI_AUTOMATION.md)，实现范围见[功能矩阵](03_FEATURE_MATRIX.md)。

## 1. 配置分层
- AppSettings：全局偏好；**已实现**（见下）；
- ToolDefaults：每个工具默认；**未实现**；
- Preset：用户可命名、导入导出；**未实现**（TOML 依赖未获批，见 ISSUES E4）；
- JobSnapshot：一次执行的完整冻结参数；**已实现**（JSON batch spec + `jobstate`）；
- Plan：Planner 解析后的实际 stage DAG；**已实现**。

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

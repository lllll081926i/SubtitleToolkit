# Preset、配置与可重放任务 Schema

本文是目标 TOML preset 草案。当前 CLI 已有 JSON batch spec 和 `.jobstate.json` 恢复凭据，不读取或写入 TOML preset；GUI 与统一 JobSnapshot 尚未实现。草案不能作为 `batch --spec` 输入。当前可执行配置见 [CLI 规格](20_CLI_AUTOMATION.md)，实现范围见[功能矩阵](03_FEATURE_MATRIX.md)。

## 1. 配置分层
- AppSettings：全局偏好；
- ToolDefaults：每个工具默认；
- Preset：用户可命名、导入导出；
- JobSnapshot：一次执行的完整冻结参数；
- Plan：Planner 解析后的实际 stage DAG。

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

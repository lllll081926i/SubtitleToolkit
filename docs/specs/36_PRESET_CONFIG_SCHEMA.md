# Preset、配置与可重放任务 Schema

本文是目标 schema 草案。当前 CLI 不读取或写入 preset，GUI 与任务持久化也未实现。示例里的 `special-auto` renderer 和 `strict` PGS 校验表示未来配置场景，不表示当前能执行；当前能力见[功能矩阵](03_FEATURE_MATRIX.md)。

## 1. 配置分层
- AppSettings：全局偏好；
- ToolDefaults：每个工具默认；
- Preset：用户可命名、导入导出；
- JobSnapshot：一次执行的完整冻结参数；
- Plan：Planner 解析后的实际 stage DAG。

## 2. Preset 示例
```toml
schema = 1
name = "Anime UHD + BD"

[[outputs]]
name = "4K-HDR-ASS"
format = "ass"
canvas = [3840, 2160]
fps = "24000/1001"
hdr.mode = "reference-white"
hdr.nits = 203
suffix = ".2160p.HDR"

[[outputs]]
name = "UHD-BD-SUP"
format = "pgs"
canvas = [1920, 1080]
fps = "24000/1001"
renderer = "special-auto"
pgs.validation = "strict"
suffix = ".UHD-BD"
```

## 3. Schema 迁移
每个配置都有 `schema`；只向前迁移，迁移前备份。未知字段保留，便于新版/旧版往返。

## 4. Renderer identity
Preset 不存绝对 DLL hash 作为硬要求；存 `renderer = special-auto` 或 provider preference。JobSnapshot 才记录实际 path/hash/version。

## 5. CLI/GUI 共用
GUI 不维护第二套参数；Preset 与 CLI schema 完全同源，避免 GUI 与核心功能漂移。

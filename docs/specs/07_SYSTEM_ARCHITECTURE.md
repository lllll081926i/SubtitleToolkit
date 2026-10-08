# 系统架构

文档修订：2026-10-08。当前结构以 `Cargo.toml` 与源码为准；本页将现有结构与目标契约分开。相关规格：[数据模型](08_FORMATS_AND_IR.md)、[画布与共享边界](09_CANVAS_RESOLUTION_MULTI_OUTPUT.md)、[任务引擎](15_BATCH_TASK_ENGINE.md)、[配置模型](36_PRESET_CONFIG_SCHEMA.md)。

## 1. 进程拓扑

```
subtitletool-cli.exe
 ├─ core crates (parse/transform/plan/cache/verify)
 ├─ subtitletool-render-host.exe  <-- 当前 libass 动态库唯一加载位置
 └─ ffprobe process adapter

SubtitleToolkit.exe (Iced, planned)
 └─ reuses the same core crates and job/report schema
```

CLI 的 render/pgs encode 通过 renderer-host 渲染。GUI 尚未实现。CSRI、VSFilterMod、x86 host、MKVToolNix adapter 目前都未交付，不能按上图中的目标结构当成现有能力。

## 2. 当前 Workspace

| 职责 | 已存在的 crate |
|---|---|
| 共享模型与探测 | `core-model`、`format-detect` |
| 格式编解码 | `codec-srt`、`codec-ass`、`codec-bdn`、`codec-pgs` |
| 字幕变换 | `transform-time`、`transform-hdr`、`transform-resolution` |
| 渲染与进程隔离 | `render-api`、`render-libass`、`renderer-host`、`win-job` |
| 图像与 PGS 作者链 | `image-core`、`pgs-author`、`validator` |
| 调度与辅助 | `task-engine`、`media-probe`、`font-service` |
| 用户入口 | `cli` |

共 20 个成员。`app`、`transform-text`、`container-adapter` 属于旧规划的候选拆分，当前没有这些 crate。宿主协议位于 `renderer-host` 内，不存在独立的 `render-host-protocol` crate。新增模块应根据实际职责决定是否拆分，不能以旧目录清单作为构建前置条件。

## 3. 核心接口

### Codec
`probe/read/write/capabilities/loss_report`。

### Transform
纯函数优先：`input IR + settings -> output IR + report`。

### Renderer
目标接口为 `open(script, canvas, fps, font_context)`、`render(timestamp)` 和 `capabilities()`。当前 CLI 的 libass 实现由独立 renderer-host 承载，host 使用版本化协议加载字体、加载脚本并返回分块 RGBA 帧。

### Validator
`validate(bytes/IR, level) -> issues + metrics + verified_level`。

## 4. Planner
目标流程：用户选 profile 后生成不可变 `Plan`：
- decode；
- transforms；
- renderer；
- image path；
- writer；
- validator；
- output transaction。

`core-model` 已有 Plan 的 JSON 序列化模型。完整 GUI/preset 到执行 DAG 的共用规划器仍是目标；当前 `batch` 使用自己的 JSON 规格并调用单命令执行链，不能据此声称目标 Planner 全部接通。

## 5. 多输出 DAG
目标 DAG 对每个 profile 先确定时间与脚本变换，再按 [RenderGroupKey 契约](09_CANVAS_RESOLUTION_MULTI_OUTPUT.md#6-rendergroupkey-与时间采样)分组。共享规则如下：

- 原始输入和未修改 AST 可共享；修改后的脚本按变换结果分支。
- 仅同一渲染组可合并采样并复用 RGBA；同一时间戳不足以证明像素相同。
- 同组 BDN/SUP 共享渲染结果，各自维护事件、调色板和 PGS 状态。
- 索引图像只有量化算法、参数和输入像素相同才可复用。

当前有 `CacheKey`、采样规划和逐帧事件规划模块。完整渲染分组与跨输出复用验收仍需实现调用链与实际计数证据，不能用这些模块的存在证明一次批处理只渲染一次。

## 6. 任务线程
以下是 GUI 与扩展调度的目标要求：
- GUI 主线程只处理 UI；
- CPU task 进 Rayon/custom pool；
- process IO 用 async task；
- renderer-host 每实例串行或限定并发，按 provider 能力决定；
- cancellation token 贯穿所有 stage。

## 7. 输出事务
`output.tmp.<uuid>` → write → fsync/close → reopen verify → rename final。覆盖替换先备份旧目标；安装新文件失败时恢复旧目标。恢复也失败时，错误必须指出备份路径和临时文件路径。

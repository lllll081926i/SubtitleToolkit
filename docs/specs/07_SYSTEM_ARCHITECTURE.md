# 系统架构

## 1. 进程拓扑

```
subtitletool-cli.exe
 ├─ core crates (parse/transform/plan/cache/verify)
 ├─ renderer-host.exe  <-- 当前 libass 动态库唯一加载位置
 └─ ffprobe process adapter

SubtitleToolkit.exe (Iced, planned)
 └─ reuses the same core crates and job/report schema
```

CLI 的 render/pgs encode 通过 renderer-host 渲染。GUI 尚未实现。CSRI、VSFilterMod、x86 host、MKVToolNix adapter 目前都未交付，不能按上图中的目标结构当成现有能力。

## 2. Workspace
- `app`：Iced shell + view-model。
- `core-model`：时间、颜色、画布、错误、Job/Profile。
- `format-detect`。
- `codec-srt`。
- `codec-ass`。
- `codec-bdn`。
- `codec-pgs`。
- `transform-time`。
- `transform-hdr`。
- `transform-resolution`。
- `transform-text`。
- `render-api`。
- `render-libass`。
- `render-host-protocol`。
- `image-core`。
- `pgs-author`。
- `validator`。
- `task-engine`。
- `media-probe`。
- `font-service`。
- `container-adapter`。
- `cli`。

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
用户选 profile 后先生成不可变 `Plan`：
- decode；
- transforms；
- renderer；
- image path；
- writer；
- validator；
- output transaction。

Plan 必须能导出 JSON，用于 bug report 和 CLI 重放。

## 5. 多输出 DAG
同一个 ASS：
- Parse AST 共享；
- HDR/Resolution transform 按 profile 分支；
- 若多个 profile renderer/canvas/fps 相同，render frame cache 共享；
- BDN 和 SUP 可共享 rendered bitmaps；
- 量化参数相同则 indexed bitmap/palette 共享。

## 6. 任务线程
- GUI 主线程只处理 UI；
- CPU task 进 Rayon/custom pool；
- process IO 用 async task；
- renderer-host 每实例串行或限定并发，按 provider 能力决定；
- cancellation token 贯穿所有 stage。

## 7. 输出事务
`output.tmp.<uuid>` → write → fsync/close → reopen verify → rename final。覆盖替换先备份旧目标；安装新文件失败时恢复旧目标。恢复也失败时，错误必须指出备份路径和临时文件路径。

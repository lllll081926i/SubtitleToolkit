# 系统架构

## 1. 进程拓扑

```
SubtitleToolkit.exe (Iced)
 ├─ core crates (parse/transform/plan/cache/verify)
 ├─ libass adapter (可信 native dependency)
 ├─ process adapters: ffprobe / mkvmerge / mkvextract
 ├─ renderer-host-x64.exe  <-- CSRI/VSFilterMod, legacy VS/AviSynth
 └─ renderer-host-x86.exe  <-- 仅旧 32-bit provider
```

外部字幕 renderer DLL 永远不直接 LoadLibrary 到 GUI 进程。

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
`open(script, canvas, fps, font_context)`；`render(timestamp)`；`capabilities()`。

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
`output.tmp.<uuid>` → write → fsync/close → reopen verify → rename final。  
默认不覆盖；覆盖时先生成 backup/replace transaction。

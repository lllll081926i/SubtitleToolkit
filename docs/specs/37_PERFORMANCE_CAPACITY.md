# 性能、缓存与容量规划

本文区分现有内存路径与性能目标。缓存配额、LRU、benchmark 阈值和完整跨 profile 共享尚未全部交付，不以目标值描述当前测量结果。

## 1. 性能目标
- 文本转换：1000 个普通字幕文件的 parse/transform 不应被 GUI 阻塞；
- 1080p ASS→BDN：瓶颈应主要在 renderer/quantizer，不应出现旧项目的每 PNG 约未压缩整帧写盘；
- multi-output：仅同 RenderGroupKey 的 BDN+SUP 共享 bitmap；按实际 renderer 调用次数评估收益，不预设固定 50% 加速；
- UI 主线程任何单次同步工作建议 < 8ms，绝不直接执行文件遍历/renderer。

## 2. Render Cache
两级：
- Memory LRU：最近 frames/indexed images；
- Disk cache：批量/多输出中间产物。

Disk cache 默认放 `%LOCALAPPDATA%\SubtitleToolkit\cache`，设置容量上限和自动清理。

此目录是未来应用运行时缓存候选，不是构建目录。开发会话必须先运行 `tools/set_build_cache.ps1`；Cargo/native/download/tmp 缓存均使用非 C 盘。构建指南不能将上面的运行时目录当成构建缓存。

完整缓存的键见[渲染分组契约](09_CANVAS_RESOLUTION_MULTI_OUTPUT.md#6-rendergroupkey-与时间采样)。容量达到上限时先淘汰未引用产物；正在使用的产物不得删除。配额、峰值内存与输出大小必须有记录后再冻结默认值。

## 3. 不缓存的内容
- 不可信/失败 parser 结果；
- renderer crash 前最后一个未确认 frame；
- 外部字体环境无法 hash 时的 render；
- Strict validator 未完成的“verified result”。

## 4. Memory
全画布 3840×2160 RGBA 约 31.6 MiB/帧。禁止把完整采样帧按时间轴保存在内存。当前规划器逐帧计算可见内容指纹，只保留活动 run 的 tight patch 和最终事件补丁；renderer-host 通过分块管道传输一帧，调用侧仍需为当前帧分配 RGBA 缓冲。

最终事件补丁随不同视觉事件数量增长，这是输出本身所需的数据；长时间不变的帧会合并为一个事件。若长片含大量不同画面，仍需评估事件补丁总量和输出磁盘容量。完整帧的瞬时峰值还包括 host 当前帧、IPC 块和调用侧帧，不应把“逐帧”误写成固定常数内存。

## 5. 并发默认
根据逻辑核与内存估算：
- text jobs 可高并发；
- 4K render/quantize 默认低并发；
- external renderer provider 初始设 1 session/host，确认 thread safety 后才提高。

## 6. Benchmark
criterion + end-to-end benchmark corpus，记录版本趋势；性能回退超过阈值时 CI warning。

这是后续 benchmark 方案，当前不宣称已有 criterion/CI 基准。记录输入摘要、画布、FPS、provider/字体身份、并发、渲染次数、进程峰值内存、临时磁盘与总耗时；相同组复用与不同组隔离分别验收。

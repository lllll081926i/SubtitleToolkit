# 性能、缓存与容量规划

## 1. 性能目标
- 文本转换：1000 个普通字幕文件的 parse/transform 不应被 GUI 阻塞；
- 1080p ASS→BDN：瓶颈应主要在 renderer/quantizer，不应出现旧项目的每 PNG 约未压缩整帧写盘；
- multi-output：相同 render profile 的 BDN+SUP 共享 bitmap，目标减少接近 50% 的重复渲染；
- UI 主线程任何单次同步工作建议 < 8ms，绝不直接执行文件遍历/renderer。

## 2. Render Cache
两级：
- Memory LRU：最近 frames/indexed images；
- Disk cache：批量/多输出中间产物。

Disk cache 默认放 `%LOCALAPPDATA%\SubtitleToolkit\cache`，设置容量上限和自动清理。

## 3. 不缓存的内容
- 不可信/失败 parser 结果；
- renderer crash 前最后一个未确认 frame；
- 外部字体环境无法 hash 时的 render；
- Strict validator 未完成的“verified result”。

## 4. Memory
全画布 3840×2160 RGBA ~33MiB/帧，禁止无界 per-frame 缓存。动态特效必须流式 render→bbox→quantize→writer/cached compressed artifact。

## 5. 并发默认
根据逻辑核与内存估算：
- text jobs 可高并发；
- 4K render/quantize 默认低并发；
- external renderer provider 初始设 1 session/host，确认 thread safety 后才提高。

## 6. Benchmark
criterion + end-to-end benchmark corpus，记录版本趋势；性能回退超过阈值时 CI warning。

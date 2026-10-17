# 性能、缓存与容量规划

本文区分现有内存路径与性能目标。**Render Cache 两级与配额（§2）已落地**；benchmark 阈值（§6）与完整跨 profile 共享仍未交付，不以目标值描述当前测量结果。

## 1. 性能目标
- 文本转换：1000 个普通字幕文件的 parse/transform 不应被 GUI 阻塞；
- 1080p ASS→BDN：瓶颈应主要在 renderer/quantizer，不应出现旧项目的每 PNG 约未压缩整帧写盘；
- multi-output：仅同 RenderGroupKey 的 BDN+SUP 共享 bitmap；按实际 renderer 调用次数评估收益，不预设固定 50% 加速；
- UI 主线程任何单次同步工作建议 < 8ms，绝不直接执行文件遍历/renderer。

## 2. Render Cache
两级：
- **内存 LRU**：最近 frames/indexed images；
- **磁盘 cache**：批量/多输出中间产物。

Disk cache 默认放 `%LOCALAPPDATA%\SubtitleToolkit\cache`，设置容量上限和自动清理。

**实现状态（2026-10-17）：内存共享（`front::RenderMemo`）与磁盘这级均已落地。**

### 磁盘这级的实测事实

- 命令：`--no-render-cache` 关，`cache stat` / `cache clear` 看与清；
  环境变量 `STK_RENDERCACHE=0` 关掉、`STK_RENDERCACHE_DIR` 改目录、
  `STK_RENDERCACHE_MAX_BYTES` 改配额（默认 512 MiB）；
- 目录走 [`DataDir`](file:///D:/Code/SubtitleToolkit/crates/core-model/src/datadir.rs)：
  便携包内是包内 `data/rendercache/`，非便携是 `%LOCALAPPDATA%\subtitletool\rendercache`；
- 缓存 key 与内存共享**同一口径**：输入内容哈希 + 画布 + 帧率 + 默认字体
  + renderer 二进制哈希 + 注入字体哈希。后半链参数（`exact_colors`/`level`/`min_axis`）
  不进 key——它们不改变前半链产物，这正是跨运行复用成立的原因；
- 配额执行：写入后按 mtime 从旧到新淘汰到 90%，避免"刚清完又满"的抖动。
  实测：上限 20 KiB 时 5 份 17,786 字节的记录只留 1 份；上限 60 KiB 时留 3 份 / 53,062 字节；
- 读回时逐字段校验（魔数、key、长度、patch 的 RGBA 与宽高自洽）。
  不符就当作**未命中并删掉**，而不是把别人的字节当自己的结果用。
  实测：截断一半、改魔数、换 key 读同一份字节，三种情况都计 `corrupt` 并重渲染，产物不变；
- 报告里 `render_cache.hit` / `counters` 是显式事实，不是内部细节：
  同一命令两次运行的耗时差异必须可解释。

等价性证据（真 libass 渲染，见 `crates/cli/tests/render_cache.rs`）：

1. 第 1 次 `hit=false`、第 2 次 `hit=true`，两次的 XML 与每个 PNG **逐字节相同**；
2. `--no-render-cache` 与命中给出**同一份**产物（缓存是纯优化，不改变输出）；
3. 改文案后 `hit=false` 且位图确实变化；改回原样重新命中（判据是内容哈希）；
4. `cache stat/clear` 报的条数与字节数与磁盘实际一致。

未做的：没有把渲染层做成"LRU 内存缓 + 磁盘二级"的自动分级
（当前内存那级是批量内的同 key 共享 + 引用计数式的 slot，不是容量受限的 LRU）。
本机不测"更快"：那需要基准与噪声控制，而 `hit` 这个显式标志才是可核对的事实。

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

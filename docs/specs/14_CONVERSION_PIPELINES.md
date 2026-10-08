# 转换流水线规范

## ASS→SRT
Probe → ASS AST → select Dialogue → remove/flatten tags → preserve text/line break policy → SRT writer → loss report → reopen verify。

## SRT→ASS
Probe encoding → Text IR → default ASS template → time/style mapping → writer → reopen parser verify。

## ASS→HDR ASS
ASS AST → color transform targeted nodes/tokens → optional resolution transform → lossless writer → round-trip compare non-target fields。

## ASS→BDN
ASS AST → optional time/HDR/resolution transforms → renderer-host → frame change detector → streaming RGBA sample → incremental event planner/tight crop → PNG writer → BDN writer → XML+PNG validator。PNG 是否采用索引色由消费者要求决定，不因 PGS 调色板限制强制所有 BDN 量化。完整画布帧在规划后立即释放；规划器只保留当前视觉 run 和最终事件补丁。

## ASS→SUP
同 BDN 前半 → indexed image/quantization → PGS author → validator。目标多输出只在同 RenderGroupKey 内共享 host 会话、逐帧渲染和事件规划；不同组分别渲染，不缓存整段全画布 RGBA。当前跨 profile 的实际共享计数仍待 AT-MULTI-002 验收。

## SUP fast adjust
PGS parser → mutate timestamps/PCS/WDS/PDS → binary writer preserving ODS → reopen parser → compare ODS hash unchanged。

## SUP resize
PGS decode → full-canvas composition → resize bitmap/object geometry → quantize → re-author → validate。

## Multi-ASS merge
避免旧工具固定字符偏移法：按动态 Format 字段解析 Style 名和 Event Style 引用；对冲突 style 做 deterministic rename；Script Info 由 policy 选择主文件/merge white-list。

目标合并还须重写内联 `\rStyle` 引用并检查附件与脚本属性冲突。无法证明兼容时拒绝“无损合并”或报告具体损失，不把重命名 Style 表视为完整引用修复。

## Multi-output
一个 input 生成 profile DAG；输出相互独立，单个失败不回滚其他已验证成功输出，但任务最终显示 partial success。

## MKV helper
mkv identify → select subtitle track → extract → run normal pipeline → prepare fonts → mkvmerge new file → identify/verify。

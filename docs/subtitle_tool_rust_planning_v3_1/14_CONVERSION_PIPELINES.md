# 转换流水线规范

## ASS→SRT
Probe → ASS AST → select Dialogue → remove/flatten tags → preserve text/line break policy → SRT writer → loss report → reopen verify。

## SRT→ASS
Probe encoding → Text IR → default ASS template → time/style mapping → writer → reopen parser verify。

## ASS→HDR ASS
ASS AST → color transform targeted nodes/tokens → optional resolution transform → lossless writer → round-trip compare non-target fields。

## ASS→BDN
ASS AST → optional time/HDR/resolution transforms → Renderer → frame change detector → full-canvas RGBA → transparent bbox object crop → PNG quantization → BDN writer → XML+PNG validator。

## ASS→SUP
同 BDN 前半 → PGS author → validator。若同时输出 BDN+SUP，共享 render/cache。

## SUP fast adjust
PGS parser → mutate timestamps/PCS/WDS/PDS → binary writer preserving ODS → reopen parser → compare ODS hash unchanged。

## SUP resize
PGS decode → full-canvas composition → resize bitmap/object geometry → quantize → re-author → validate。

## Multi-ASS merge
避免旧工具固定字符偏移法：按动态 Format 字段解析 Style 名和 Event Style 引用；对冲突 style 做 deterministic rename；Script Info 由 policy 选择主文件/merge white-list。

## Multi-output
一个 input 生成 profile DAG；输出相互独立，单个失败不回滚其他已验证成功输出，但任务最终显示 partial success。

## MKV helper
mkv identify → select subtitle track → extract → run normal pipeline → prepare fonts → mkvmerge new file → identify/verify。

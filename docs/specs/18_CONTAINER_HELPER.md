# MKV 字幕与附件助手

## 1. 原则
调用 MKVToolNix，不自己重写 Matroska muxer。

## 2. 功能
- identify tracks/attachments；
- extract ASS/SRT/PGS/fonts；
- 处理后插回；
- language/name/default/forced；
- attach subset fonts；
- 输出新 MKV。

## 3. 不做
- 视频/音频转码；
- chapter editor；
- 全量 mkvmerge GUI 参数。

## 4. 任务事务
绝不原地修改 MKV。输出新文件，identify 成功后才显示完成。

## 5. 实现状态（2026-10-17）

已实现：`mkv identify|extract|remux`（crate `container-adapter`）。

- `identify` 读 `mkvmerge --identify -J` 的 JSON，透出 mkvmerge 自身的 errors/warnings。
- `extract` 用 `mkvextract tracks` / `mkvextract attachments`。
- `remux` 用 `mkvmerge --output`，固定 `--no-date` 与 `--disable-track-statistics-tags`（确定性产物）；
  先写 `.stk-tmp.mkv` 再原子改名；写完重新 identify 并逐轨核对元数据（类型/编码/语言/名称/default/forced、
  附件数、章节数），对不上就删半成品并失败（AUDIT-A023）。
- 附件按「文件名 + 字节数」去重，跳过项在报告里列出。

唯一写盘路径是 `remux`，且解析层与适配器层各有一道"输出不等于输入"的检查。

验收与未覆盖项见[AT-MKV-001 执行记录](../reviews/2026-10-17-mkv-container-helper.md)。

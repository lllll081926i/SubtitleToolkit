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

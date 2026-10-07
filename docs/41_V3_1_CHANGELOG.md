# V3.1 Source-Audited 变更记录

- 增加 `40_SOURCE_CODE_AUDIT.md`。
- ASS 分辨率模型从单一 PlayRes 改为 OutputCanvas / ScriptCoordinateSpace / LayoutRes / StorageSize 分层。
- libass 改为 memory-input + font-bytes injection 设计。
- 新增 RenderTimelinePlanner 与 visible-frame fingerprint。
- CSRI provider 与 VSFilterMod legacy adapter 解耦。
- PGS Fast Path 增加 ODS hash invariant 和 DTS/decode schedule gate。
- PGS encoder 增加 stateful decoder、ODS fragmentation、erase-display-set 要求。
- Peak bitrate 检测与 stream shaping 分离。
- 字体 subset 增加复杂 shaping/render equivalence gate。
- 补全 SupMover AGPL / ALASS GPL 等边界。

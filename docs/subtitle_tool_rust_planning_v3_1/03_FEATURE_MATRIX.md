# 功能矩阵、阶段与依赖

| 功能 | 优先级 | 首发 | 主要依赖 | 备注 |
|---|---|---|---|---|
| ASS/SSA parse + lossless rewrite | P0 | 是 | Rust parser bake-off | 未知 tag 必须保留 |
| SRT parse/write | P0 | 是 | internal | UTF-8 默认 |
| ASS→SRT | P0 | 是 | ASS AST | 有损报告 |
| SRT→ASS | P0 | 是 | template | 不伪造复杂样式 |
| FPS/Duration scale | P0 | 是 | Rational time | 24000/1001 |
| Offset / two-point fit | P0 | 是 | time core | |
| ASS HDR dim | P0 | 是 | color transform | 保留 alpha |
| 16:9 resolution scale | P0 | 是 | ASS AST | 完整画布硬规则 |
| Multi-output profiles | P0 | 是 | planner/cache | 一次多产物 |
| Batch queue | P0 | 是 | task engine | |
| libass render | P0 | 是 | libass 0.17.x | default |
| BDN XML+PNG | P0/P1 | 是 | render/image | |
| PGS parser/decoder | P1 | 是 | oxideav or internal | differential test |
| PGS Fast Path | P1 | 是 | parser | 无图像重编码 |
| PGS basic encoder | P1 | 是 | quant/RLE | 不冒充 strict |
| CSRI VSFilterMod host | P1 | 是 | Windows helper | 特效关键 |
| VapourSynth adapter | P1/P2 | 可选 | R80/Python | legacy |
| AviSynth+ adapter | P1/P2 | 可选 | 3.7.5+ | legacy |
| Font health/subset | P1 | 是 | assfonts ideas | Windows font index |
| MKV helper | P1 | 是 | mkvmerge/mkvextract adapter | 不重造 muxer |
| Smart media probe | P1 | 是 | ffprobe | 辅助，不主导 |
| Smart sync | P2 | 可选 | ALASS external | GPL 隔离 |
| Advanced PGS author | P2 | 后续 | decoder model | SUPer research |
| Strict Blu-ray validator | P2 | 后续 | corpus | |
| OCR SUP→SRT | P2/P3 | 后续 | Paddle/Tesseract | 不做编辑器 |
| OpenCC/zhhz | P2 | 可选 | pure Rust | 中文转换 |

## 明确不做的功能矩阵

| 功能 | 决策 |
|---|---|
| 波形/视频手工打轴 | 不做 |
| 逐句字幕编辑 | 不做 |
| Style Manager | 不做 |
| Visual Typesetting | 不做 |
| ASS Draw | 不做 |
| ASR/TTS/翻译 | 不做 |
| 视频转码/烧录 | 不做 |
| 完整蓝光 Authoring | 不做 |

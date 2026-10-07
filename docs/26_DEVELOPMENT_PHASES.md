# 开发阶段与里程碑

## Phase 0：技术验证（必须先做）
- Iced 0.14 shell + drag/drop + native file dialog；
- ASS parser bake-off；
- libass x64 build/FFI；
- CSRI provider proof-of-concept + x86/x64 host；
- PGS parser candidate round-trip；
- Windows font enumeration；
- ffprobe/MKVToolNix adapters；
- 许可证表。
Gate：每项有 executable test，不接受 README 口头“可行”。

## Phase 1：文本/时间/HDR MVP
ASS/SRT、format detect、encoding、time rational、FPS/offset、HDR、16:9 resolution transform、CLI、基础 Iced 页面。

## Phase 2：批量与多输出
Task engine、profiles、cache、transaction、manual-first UI 完整、video probe 辅助。

## Phase 3：libass + BDN
全画布 renderer、动态事件采样、PNG/BDN、golden tests。

## Phase 4：PGS parser/Fast Path/Basic Encoder
先解析和无损调整，再做 encoder；validator Level 1/2。

## Phase 5：特效 Renderer
CSRI/VSFilterMod production host；x86 helper；VapourSynth/AviSynth legacy adapters；pixel diff。

## Phase 6：字体/MKV
Font Health/subset/attachments；container helper。

## Phase 7：Advanced PGS / Strict Blu-ray
buffer/epoch/object reuse/ahead decode；strict validator；硬件/软件播放器 corpus。

## Phase 8：OCR/Smart Sync 可选
不阻塞主产品。

## Phase 9：稳定与发布
Installer/portable、签名、更新、性能、fuzz、diagnostics、文档。

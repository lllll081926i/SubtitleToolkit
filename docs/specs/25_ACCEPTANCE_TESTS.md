# 正式验收用例

本表定义验收条件，不代表用例已经通过。需要真实 DLL、播放器、真盘 SUP 或 GUI 的用例，必须明确记录依赖与样本；模拟测试不能代替互操作结果。当前交付状态见 [`../STATUS.md`](../STATUS.md)。

- AT-ASS-001：parse→write 未修改文件语义等价，未知 tag 不丢。
- AT-ASS-002：HDR transform 只改目标颜色字段，时间/文本/几何 hash 不变。
- AT-CANVAS-001：1080→2160 特效字幕保持完整16:9坐标，关键定位×2。
- AT-CANVAS-002：2.39:1 视频辅助不得把 ASS 自动改为非16:9逻辑画布。
- AT-MULTI-001：单 ASS 一次生成 1080 ASS + 2160 ASS + 1080 SUP + SRT，所有 output 独立 verify。
- AT-MULTI-002：BDN+SUP 相同 render profile 只渲染一次（cache metrics 证明）。
- AT-TIME-001：24000/1001→25 speed conversion 长片无浮点累积漂移。
- AT-TIME-002：25→50 frame snap 不改变总时长。
- AT-PROBE-001：目标视频只填建议，不自动应用 speed conversion。
- AT-RENDER-001：设置 `STK_LIBASS_DLL` 后，真实 libass 经 renderer-host 输出正确 RGBA；故障只影响 host。
- AT-RENDER-002：取得可用 VSFilterMod provider 后，证明其崩溃只终止 renderer-host，GUI 可重试；自伤 `Poison` 测试不能代替该 provider 测试。
- AT-RENDER-003：仅在 x86 host 产物加入支持矩阵后，验证 x86 DLL 路由；当前缺少该产物，不验收自动路由。
- AT-PGS-001：对可合法使用的第三方 SUP 执行 +1000ms，ODS payload hash 完全一致；自制流往返不能代替外部样本。
- AT-PGS-002：Forced-only Fast Path 不重编码。
- AT-PGS-003：palette dim 只改 PDS，结构验证通过。
- AT-PGS-004：Basic Encoder segment length/RLE/object id/palette 正确。
- AT-BD-001：严格 Blu-ray profile 禁止 2160p PGS；UI 明确解释。
- AT-BDN-001：XML graphic width/height 与 PNG 一致；timecode frame 合法。
- AT-FONT-001：中文字体缺字被报告。
- AT-FONT-002：font environment 改变导致 render cache miss。
- AT-MKV-001：提取→处理→新MKV，原文件不改，字体附件存在。
- AT-BATCH-001：100 文件×3 profiles，单文件失败不阻塞其他。
- AT-UI-001：重任务期间 UI 60s 内持续可响应、取消可点击。
- AT-PATH-001：中文/emoji/260+长路径正确。
- AT-SEC-001：恶意 PGS length/RLE 不崩溃、不超额分配。
- AT-OUT-001：写出后 verify 失败时 final 文件不替换旧文件。

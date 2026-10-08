# OCR（后期可选）

## 1. 为什么延后
SUP→SRT 是有价值的，但 OCR 会引入模型/语言/校对问题；产品又明确不做字幕编辑器，因此必须提供“批量 OCR + 报告”，而不是把大量编辑 UI 带回来。

## 2. 候选
- PaddleOCR（Windows）；
- Tesseract；
- 可选本地视觉模型 adapter。

## 3. 前处理
transparent crop、border remove、color isolation、binarization、scale。

## 4. 输出
SRT/ASS + confidence sidecar；低置信度行导出报告，用户去外部编辑器修改。

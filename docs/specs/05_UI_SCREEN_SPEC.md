# UI 页面级详细规格

## 1. 通用控件规则
- 所有数值输入同时支持下拉 preset + 手输。
- 参数旁有 `?` tooltip，解释“会不会改时长/是否有损”。
- 每个输出卡都显示：Format / Canvas / FPS / HDR / Renderer / Validation。
- 有损动作统一黄色 `Lossy` chip；non-standard 输出橙色 `Custom` chip；严格验证通过绿色 `Verified` chip。
- 任何自动建议都必须显示“建议来源”。

## 2. 转换页
### 输入区
- 文件列表：名称、探测格式、编码、事件数、时长、PlayRes。
- 批量时支持选择/取消某行。

### 目标区
- Format combobox。
- Encoding（文本格式）。
- Loss report 卡片。
- Normalize 开关（默认仅安全项）。

### 输出区
- 文件夹；命名模板；覆盖策略；保留目录结构。

## 3. ASS/HDR 页
- HDR Mode：Off / Compatibility / Percentage / Reference White。
- Percentage：0–100；常用 preset 33.3/40/50/60。
- Reference White：100/120/150/203/custom nit。
- 作用范围：Style colors / inline colors / VSFilterMod gradient colors。
- Alpha：锁定不改。
- Resolution Adapt：保持 / 720 / 1080 / 2160 / custom 16:9。
- “同时输出”按钮：添加另一个 profile。

## 4. 时间/FPS 页
- Source FPS：23.976(24000/1001)、24、25、29.97、30、50、59.94、60、custom rational。
- Target FPS 同上。
- Mode：Speed scale / Frame snap / Offset / Two-point。
- Scale summary：`x 0.95904` 等。
- Preview：首条、1h、末尾预计时间差。
- 不把 25→50 自动当 speed change；用户必须显式选择。

## 5. ASS→BDN/SUP 页
### Renderer
- libass；
- External Special Renderer（自动探测 CSRI provider/VSFilterMod）；
- Legacy VapourSynth；
- Legacy AviSynth+。

### Canvas
- 720p/1080p/2160p/custom 16:9。
- Strict Blu-ray SUP 时 2160p 选项禁用并解释原因。

### Render precision
- Event-adaptive；
- Per-frame for animated tags；
- HFR downsample policy。

### Outputs
- BDN XML+PNG；SUP；两者同时。
- 可一次定义多个 profile。

## 6. SUP 工具页
Tab 1 `Fast Adjust`：Offset、scale time、X/Y、Forced、palette brightness。  
Tab 2 `Re-encode`：resolution scale、crop、palette/quantization。  
Tab 3 `Validate`：segment structure、timeline、palette/object/window、buffer model。  
Fast Adjust 顶部必须显示“此操作是否重编码 bitmap”。

## 7. 批量/多输出页
左侧 Inputs；中间 Pipeline Steps；右侧 Output Profiles。

Profile 行：
`[✓] Name | Format | Canvas | FPS | HDR | Renderer | Validation | Suffix`

支持 duplicate profile，例如复制 1080p profile 后改 2160p。

## 8. 字体/MKV 页
### Font Health
- Required family/style；
- Found/Not found；
- missing glyph count；
- source path；
- subset size estimate。

### MKV Helper
- tracks table；
- attachments table；
- extract/reinsert；
- language/default/forced；
- 不提供视频/音频转码参数。

## 9. 同步助手
- Reference：video/audio 或正确字幕；
- Engine；
- Result：offset、ratio、segments、confidence；
- `Apply to copy` 主按钮，禁止默认覆盖源文件。

## 10. 诊断页
- App version / git hash；
- Windows build；
- CPU/RAM；
- libass version；
- external renderer provider name/version/arch；
- ffprobe/mkvtoolnix available；
- render self-test；
- PGS self-test；
- Export diagnostic zip。

## 11. 设置页
- General；Paths；Renderer；Performance；Output defaults；Advanced；Updates；About/Licenses。
- 设置搜索框。
- 所有第三方 DLL 路径都显示 hash/arch/version 检测结果。

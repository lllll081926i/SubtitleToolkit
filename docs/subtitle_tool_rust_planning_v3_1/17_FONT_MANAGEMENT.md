# ASS 字体管理、字体子集与附件

## 1. 目标
不做字体编辑器，做“字幕可复现性”工具：找到作者真正引用的 face，确认字符覆盖，安全 subset，并能让 libass / VSFilterMod / MKV 在目标机器重现。

## 2. Windows Font Resolver 以 DirectWrite 为权威
数据库至少记录：
- localized family names；
- typographic family / Win32 family；
- face / full name / PostScript name；
- weight / style / stretch；
- variable axes；
- file path / collection index；
- TTC/OTC face index；
- glyph coverage；
- SHA-256 / mtime。

不能只扫描 `C:\\Windows\\Fonts` 文件名，因为字体注册、localized family、TTC、用户字体和 variable font 都会让“文件名=Fontname”失效。

## 3. ASS Font Usage Scanner
必须支持动态 `Format:` 字段，不依赖固定列号。
至少跟踪：
- Style Fontname / Bold / Italic；
- inline `\\fn`；
- `\\b`；
- `\\i`；
- `\\r` / `\\rStyle` reset；
- dialogue 实际字符集合。

kotorimiku/assfonts 当前 Rust 源码正是围绕这四类 override 做 state tracking，这部分可作为行为参考；它不是通用 ASS parser，不应代替我们的 ASS AST。

## 4. Matching
候选评分不能只看 family：
1. exact family/face；
2. typographic / localized aliases；
3. weight distance；
4. italic mismatch penalty；
5. stretch；
6. glyph coverage；
7. variable axes；
8. renderer 实际解析结果。

最终 report：Exact / Alias / Substituted / Missing / Missing Glyph / Ambiguous。

## 5. Subset：源码审计后的风险修正
assfonts Rust 版本当前实现从 cmap 把 `required_chars` 映射为 glyph IDs，再调用 allsorts `SubsetProfile::Minimal`。这个方案非常适合作为候选，但不能未经验证就称为“成熟且一定安全”。

必须建立 subset equivalence corpus：
- Latin ligatures：fi/fl；
- CJK + IVS；
- Arabic joining；
- Devanagari/Indic GSUB/GPOS；
- combining marks；
- vertical glyphs / `@Font`；
- color fonts（COLR/CPAL/CBDT/SVG）若支持；
- variable fonts；
- CFF/CFF2；
- TTC/OTC；
- composite glyphs。

判定方式：原字体 vs subset 字体，在相同 renderer/timestamp/canvas 上的 glyph shaping + rendered RGBA 必须达到 golden 条件。任何复杂脚本失败就回退“复制原字体，不 subset”。

## 6. 输出
- report only；
- copy full fonts；
- copy subset fonts；
- embed ASS `[Fonts]`；
- prepare MKV attachments；
- manifest（原 font hash → subset hash → ASS request）。

## 7. libass 字体注入
Windows 路径为了稳健：
- Rust/Win32 读 font bytes；
- `ass_add_font()` 注入 renderer library；
- `[Fonts]` 可由 libass extraction 或我们自己的 attachment parser 处理；
- render cache key 包含所有实际 font bytes hash + resolver policy + renderer version。

## 8. 安全策略
- subset 是可回退优化，不是必需条件；
- 永远保留原字体来源信息；
- 不覆盖用户原字体；
- 对字体 parser/subsetter 做 fuzz；
- 限制单字体、总字体、table size，防止恶意字体内存/CPU DoS。

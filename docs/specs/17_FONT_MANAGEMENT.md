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

### 实现状态（2026-10-17）

**六档已全部交付**：

| 档位 | 命令 |
| --- | --- |
| report only | `font scan` |
| copy full fonts + manifest | `font scan --copy-fonts DIR` |
| copy subset fonts + manifest | `font subset` |
| embed ASS `[Fonts]` | `font subset --embed-fonts --out X.ass` |
| prepare MKV attachments | `font subset --mkv-attachments` |
| manifest（原 hash → subset hash → 请求）| 每档都带 |

`font subset` 的行为：

- 字符集来自同一套 ASS 用量扫描（`usage::scan_text`），与 `font scan` 不漂移；
- 复杂脚本（Arabic/Indic/combining/vertical/IVS/emoji）**一律回退整份**（§5 的硬性要求）；
- 其余做 cmap 级等价检查（字符解出的字形存在性与水平 advance 必须一致），不过就回退；
- 产物写到 `--subset-dir`（默认源字体目录下的 `.subtitletool-subset/`），**不覆盖原字体**；
- manifest 同时记原字体 hash 与子集 hash，任何时刻都能回到原字体。

两档新输出的纪律：

- **embed**：先剥掉原有的 `[Fonts]` 段再嵌（不去重就有两份而解析器只认第一份）；
  写完重新解一遍，逐份核对名字与 SHA-256；**`[Fonts]` 必须能被解析器读回**（
  `font_service::attachments` 的编码器与解码器是严格互逆的一对，由往返用例守护）。
- **MKV 附件**：只准备目录 + 给出可照抄的 `mkv remux --attach-font …` 命令行，
  自己**不调 mkvmerge**（容器层的事留给 18 §2 的 `mkv remux`）；附件名做安全化
  （去掉路径分隔符与控制字符），否则 `--attach-file` 会把它当子目录。

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

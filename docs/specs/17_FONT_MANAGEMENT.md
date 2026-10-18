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

### 实现状态（2026-10-17）

上面的清单**已全部交付**（`db::FaceRecord` 逐项对应），数据来源是
“文件系统 + 注册表 + 读 `name`/`OS2`/`cmap`/`fvar` 表”。

**权威性部分交付**：新增 **GDI 交叉核对**（`font-service::gdi`），
`font scan --json` 的 `cross_check` 段给出本机实测：

```
gdi_faces_raw   6020   // GDI 枚举到的原始 face 数
 gdi_faces       3233   // 过滤后（只数真曲线字体、不含 @ 竖排变体）
 db_faces        1376   // 我们的库
 missing_from_db    2   // GDI 看得到、我们一个 face 都匹配不上的家族
 extra_in_db     1801   // 我们有、GDI 没枚举到的家族（信息，不是缺陷）
```

为什么是 GDI 而不是 DirectWrite：后者的 `IDWriteFactory` 是 **COM**，手写 vtable
布局一个字段错就崩，而那类崩溃无法用测试穷尽 —— 与本仓库“不自己实现安全边界”的
取向相同（同 23 §XML、PNG deflate、真 minidump）。GDI 的 `EnumFontFamiliesExW`
是**扁平 C API**（只需两个 `extern "system"` 声明），枚举的是同一套系统字体注册表。

它给的不是“权威替换”而是**权威交叉核对**：漏掉的 face 会让“找到作者引用的字体”
静默失败，而那正是 §1 的全部目的。价值在于那些名字是**可执行的线索**
（去查为什么没扫到），而“未实现”什么都不是。

**仍未做**：DirectWrite 的 **fallback 链**（某个码位找不到时系统会回退到哪个 face）
—— 那需要 `IDWriteFontFallback`，是 COM（ISSUES AG1）。

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

### corpus 已建立（2026-10-17）

`crates/font-service/tests/subset_corpus.rs`（9 个用例）逐项回答上列 11 项**被怎么处理**。
关键认识：那 11 项写成一张清单，但**风险来源分两类**，判据也因此不同：

| 类别 | 项 | 判据 |
| --- | --- | --- |
| **字符级**（危险来自“请求了哪些字”） | Arabic、Indic、combining、vertical、CJK+IVS、emoji | **必回退** |
| **字体级**（危险来自字体本身的结构） | TTC/OTC、variable、CFF/CFF2、composite | 能做就做且等价，不能就回退 |
| **单独一类** | Latin ligatures（fi/fl） | **必须真的做子集** |

混用同一个判据会出错：要求 Latin ligatures 也回退等于**对英文关掉子集**。
所以另有一条**反向**用例断言普通拉丁/汉字/日文/韩文**都不回退** ——
只断言“该回退的回退了”会漏掉“全都回退”这种也满足的实现。

本机实测：CFF（`MiSans-Regular.otf`）与 variable（`bahnschrift.ttf`）
**都能正确子集**（等价检查已过）；TTC 走回退分支并带原因；复合字形子集正常。

**仍缺 golden 图像比较**。规格要的 “rendered RGBA 达到 golden 条件” 需要
固定的语料字体 + 参考图，而选语料先把问题带到许可审计上（21 §6）。
当前用 shaping 等价（字形数 + advance）作替代：比不查强，但**比规格要求的弱**。
variable font 的**轴保真**也未证明（等价比较不覆盖轴实例化后的轮廓差异）。
详见 [subset corpus 裁决](../reviews/2026-10-17-subset-corpus.md)。

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

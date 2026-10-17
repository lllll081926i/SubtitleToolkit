# 字体子集实现裁决（17 §5/§6，AT-FONT-003）

日期：2026-10-17
范围：`WBS-F009/F010`（子集与等价 corpus）、`AT-FONT-003`、`ISSUES AG2/E4`

## 1. 依赖裁决（ISSUES E4）

选了 `allsorts` 0.17.0（**Apache-2.0**），符合 21 §1 的宽松许可政策。
许可门禁：`NOTICES OK：493 个第三方包 … 0 个问题`，未引入任何强制 copyleft。

关闭默认特性 `flate2_zlib`（链 C zlib），改用 `flate2_rust`：纯 Rust 后端，
少一个 native 依赖，代价是 WOFF2 解压稍慢——而本功能只处理 TTF/OTF，
这条代价在实际上不存在。

候选对比（都不是强制 copyleft，规格点名 allsorts）：

| 库 | 许可 | 为什么没选 |
| --- | --- | --- |
| `allsorts` | Apache-2.0 | **选中**：规格点名的候选；自带独立 shaping/解析实现，可交叉验证 |
| `subsetter`（typst） | MIT OR Apache-2.0 | 与现有 fontations 栈同源，交叉验证价值低 |
| `fontcull` | MIT | 依赖 `fontcull-skrifa` 等**改名分叉**，来源核对成本高 |

## 2. 用真实依赖执行的验收

### 2.1 cmap 级等价（`crates/font-service/tests/subset_real.rs`）

`C:\Windows\Fonts\AGENCYB.TTF`，请求 `"Hello World 123"`：
60,656 → 5,916 字节（省 90%），等价检查 11 次探查全过，产物仍可被解析。

### 2.2 渲染级等价（AT-FONT-003，`crates/cli/tests/subset_render.rs`）

同一份 ASS、同一 canvas（1920×1080）、同一 fps（24），
把**整份 Arial** 与**子集**分别经 `--inject-fonts`（17 §7 的 `ass_add_font` 通路）
注入真 libass 渲染，逐帧比较 PNG 的 SHA-256：

```
子集：1045720 -> 29240 字节（省 1016480）
2 帧逐像素一致
```

**结论：拉丁文字场景下，子集与整份渲染结果逐像素一致。** 这是 17 §5 要求的
"same renderer/timestamp/canvas + rendered RGBA" 那一半，用真实 renderer 回答了。

## 3. 过程中发现的真实现缺陷（已修）

### 3.1 `allsorts` 在请求列表不含 glyph 0 时会丢掉空格映射

本机实测复现矩阵（字体 `AGENCYB.TTF`，空格原 gid=3）：

| 传给 subsetter 的字形列表 | 子集里 U+0020 的 gid |
| --- | --- |
| `[3]` | 0（丢） |
| `[3, 43, 72]` | 0（丢） |
| `[0, 3, 43]` | 1（保住） |
| Arial 的 `[3, 1, 2, 3]` | 3（保住） |

规律是 subsetter 重编号字形时 `.notdef` 的处理不一致 —— 子集保留了空格的**字形数据**，
却把 cmap 里 `U+0020 → gid` 的那条映射写成了 0。空格会渲染成 `.notdef`。

修正：请求列表**恒带 glyph 0（.notdef）**。这避开了已知的这一类，
**但不等于解决了问题**——真正的兜底是等价检查，它不为任何未验证的行为背书。
这个缺陷恰好证明了门禁的价值：第一版没有等价检查时，它会静默交付一个空格坏掉的字体。

### 3.2 `MatchingPresentation::Keep` 与 `NotRequired` 语义不同

`Keep` 在查不到呈现形式时返回 `.notdef`，`NotRequired` 退回基础映射。
用 `Keep` 会把大量正常字符误判成"没有字形"，于是整批回退。
改用 `NotRequired`（本模块不做呈现形式选择，那是 renderer 的事）。

### 3.3 `allsorts` 的 API 形状

`Font::from_index` / `file::FileTableProvider` / `allsorts::Glyph` 都不存在。
正确路径是 `FontData::read` → `table_provider(index)` → `Font::new(provider)`，
它顺带把 TTC/OTC 成员选择与 WOFF/WOFF2 分派一起解决了。

## 4. 决策记录

### D-FNT-01 等价检查比 advance，不比字形 id

子集必然重编号字形，拿 id 比对会大面积误报。真正的不变式是
"这个字符解出的字形还在不在 + 它的水平 advance 是多少"。

**已知盲区**：合字（如 `fi`）在逐字符检查里看不出来。这一点写在报告 `scope` 里，
且在 17 §5 的完整 corpus 语料到位后要补上——不假装它已经被覆盖。

### D-FNT-02 复杂脚本一律回退，不做"试试看"

17 §5 最后一句是硬性要求。判定按**码位区间**（Arabic / Indic / combining /
vertical forms / IVS / emoji），不按"这个字体有没有对应表"：
在没有真 renderer 与授权语料之前，我们无法证明复杂脚本一定成功，
所以宁可回退（体积大一点）也不冒"渲染变了却没发现"的风险。

### D-FNT-03 默认输出到源字体目录下的 `.subtitletool-subset/`

不用系统临时目录：子集产物是要交付给 MKV 附件或 ASS `[Fonts]` 的，
放在临时目录里会被清理掉，用户只会看到"交付了但找不到"。

文件名带源字体哈希前 8 位（`<stem>.<sha8>.subset.<ext>`）：
同一份字体的不同子集不会互相覆盖，且从文件名就能对上是哪一份源（17 §8）。

### D-FNT-04 `--fallback-full` 存在，但默认关

默认"能子集就子集、不能就整份交付"，符合 17 §8 第一条（子集是优化不是必需）。
给流水线一个"要么全是子集、要么别交付"的开关，是因为那种场景需要它，
而不是让调用方去解析 JSON 才知道发生了什么。

### D-FNT-05 不覆盖用户原字体

17 §8 第三条。产物只写 `--subset-dir`，源字体全程只读。

## 5. 未覆盖项（不宣称通过）

- **17 §5 的完整等价 corpus 尚未建立**：CJK+IVS、Arabic joining、Indic GSUB/GPOS、
  combining marks、vertical/`@Font`、color font（COLR/CPAL/CBDT/SVG）、variable font、
  CFF/CFF2、TTC/OTC、composite glyph。当前用码位区间**规避**这些情形（回退），
  而不是**覆盖**它们——这两件事不能混为一谈。
- **TTC/OTC 成员与 variable font 未覆盖**：`subset_once` 遇到即回退。
- **合字未做逐序列检查**（见 D-FNT-01）。
- **RGBA 对照只覆盖了拉丁文字**：AT-FONT-003 用的是 Arial + 英文；
  中文、日文、韩文的渲染等价没跑（缺少授权语料与对应断言）。
- **未做字体 parser/subsetter 的 fuzz**：17 §8 第四条要求，
  当前只有"错误路径不 panic"的常规测试（`subset_bytes` 对垃圾字节回退而不崩）。
- **未限制单字体/总字体/table size**：17 §8 第五条的内存与 CPU DoS 防护未做。

## 6. 仍未交付的输出档位（ISSUES AG8）

| §6 档位 | 状态 |
| --- | --- |
| report only | 已交付（`font scan`） |
| copy full fonts + manifest | 已交付（`font scan --copy-fonts`） |
| copy subset fonts + manifest | 已交付（`font subset`） |
| embed ASS `[Fonts]` | **已交付**（`font subset --embed-fonts`） |
| prepare MKV attachments | **已交付**（`font subset --mkv-attachments`，输出可照抄的 `mkv remux` 命令行） |

## 7. 第四/五档的实现裁决（AT-FONT-004 候选）

### 7.1 编码器与解码器必须严格互逆

`font_service::attachments` 新增 `encode_base64` / `encode_fonts_section` /
`insert_fonts_section` / `strip_fonts_section`。回程用例把**整份真字体字节**
（1,045,720 字节）走一遍 `encode → parse`，逐字节比较才算过。

带出两个真实缺陷（已修）：

- **BOM 会让整个 `[Fonts]` 段隐形**：BOM 在文件开头时，第一行是
  `\u{feff}[Fonts]`，而它不是一个段头，解析器会整段跳过去。修法是在解析的第一行剥 BOM。
- **不去重会写出两份 `[Fonts]`**：解析器只认第一份，第二份字节会静默留在文件里。
  `strip_fonts_section` 负责先剥掉；没删过东西时它一个字节都不动（有断言守护）。

base64 行宽取 80（4 的倍数）：拆散编码单元就拼不回去了。

### 7.2 什么被证明了、什么没有

原本想拿“字体库置空的目录里渲染内嵌产物”当判据。实测发现**这条路证明不了任何事**：
`STK_FONTS_DIR` 只换我们自己的扫描根（报告 `roots` 可见它只在扫描侧生效），
libass 仍会走自己的系统字体回退链，于是“空目录也能渲染成功”与内嵌无关。
把这个结果写成“内嵌验证通过”就是拿一个自欺的断言充证据。

所以 `crates/cli/tests/embed_fonts.rs` 只声明能判定的东西：

1. 内嵌段能被解析器读回，**SHA-256 与报告里声明交付的那份相等**；
2. 产物里只有一个 `[Fonts]`；
3. 带内嵌段的 ASS 能渲染，且与不带它时**逐像素一致**
   （只证明“加了这段没弄坏渲染”，**不证明** libass 用了它）；
4. MKV 附件目录里的字节与报告里的哈希一致，且仍是可解析的字体。

“libass 到底用没用内嵌字节”需要一条能屏蔽系统字体的通路，
那由 `--inject-fonts` 回答（见 `crates/cli/tests/subset_render.rs` 的逐像素对照）。

### 7.3 MKV 附件不自己调 mkvmerge

字体服务只落目录 + 给命令行，容器层的事留给 `mkv remux`（18 §2）。
这样做的好处是两边可以独立验收：附件目录的判据是“字节对不对”，
MKV 那侧的判据是 AT-MKV-001 的逐轨元数据核对。

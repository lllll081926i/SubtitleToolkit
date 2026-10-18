# 裁决：subset equivalence corpus（17 §5）

日期：2026-10-17
范围：`crates/font-service/tests/subset_corpus.rs`（新增）
规格：`docs/specs/17_FONT_MANAGEMENT.md` §5

## 1 规格要什么

17 §5 列了 11 项必须覆盖的情形，并给出判定方式与一句很要紧的话：

> 这个方案非常适合作为候选，但**不能未经验证就称为"成熟且一定安全"**。

> 判定方式：原字体 vs subset 字体，在相同 renderer/timestamp/canvas 上的
> glyph shaping + rendered RGBA 必须达到 golden 条件。
> **任何复杂脚本失败就回退"复制原字体，不 subset"。**

所以本文件**不声称"子集是安全的"**，而是逐项回答每一类情形被怎么处理。

## 2 11 项实际分成两类（这是本裁决的关键认识）

规格把它们列在一张清单里，但它们的风险来源不同，判据也因此不同：

| 类别 | 项 | 风险来自 | 判据 |
| --- | --- | --- | --- |
| **字符级** | Arabic joining、Indic GSUB/GPOS、combining marks、vertical forms、CJK+IVS、emoji/color | "请求了哪些字" | **必回退** |
| **字体级** | TTC/OTC、variable、CFF/CFF2、composite glyphs | "字体本身的结构" | 由解析层决定：能做就做且等价，不能就回退 |
| **单独一类** | Latin ligatures | —— | **必须真的做子集** |

把三类混在一起用同一个判据会出错：

- 若把 Latin ligatures 也要求"必回退"，等于**对英文关掉子集功能** ——
  因为 "fi"/"fl" 在英文字幕里满地都是。那不是保守，是功能失效。
- 若把字体级那几项也要求"必回退"，就把 `allsorts` **已经能正确处理的**
  CFF 与 variable 一并否掉，而那与实测不符（见 §4）。

## 3 字符级：断言"必回退"，钉住策略本身

`the_complex_script_corpus_all_falls_back` 覆盖 9 个样本，逐项断言命中了哪条规则
（Arabic / Indic / combining / vertical / variation / emoji）。

为什么断言"必回退"而不是"必须正确"：**因为规格说的就是回退**。钉住策略之后，
若哪天有人为了"少回退"而放宽某一条，用例会红，审查者能看到"这一条被放开了"
这个事实 —— 而不是在某个用户的阿拉伯文渲染错了之后才发现。

**配套的反向用例同等重要**：`plain_latin_and_cjk_do_not_trigger_fallback` 断言
普通拉丁、常用汉字、日文、韩文、带变音符的拉丁文**都不回退**。

只断言"该回退的回退了"会漏掉"全都回退"这种也满足的实现。

## 4 字体级：实测结果（本机真实字体）

| 项 | 实测 | 说明 |
| --- | --- | --- |
| **CFF/CFF2** | `MiSans-Regular.otf` **子集成功**，等价检查通过 | `allsorts` 能正确处理 CFF 轮廓 |
| **variable** | `bahnschrift.ttf` **子集成功**，等价检查通过 | 见下面的保留意见 |
| **TTC/OTC** | 走回退分支（带原因） | 集合成员选择不支持 |
| **composite glyphs** | 子集成功，等价检查通过 | À/é/ü 这类基字符+重音的复合字形没被弄坏 |
| **CJK 普通汉字** | 子集成功 | 且验证了"不带 IVS 就不被连累回退" |

### 一个**如实保留**的意见

variable font 那条用例的通过信息里打印了：

> 注意：可变字体 bahnschrift.ttf 被做了子集（等价检查已过）——
> 若产物丢掉了 fvar 轴，默认实例仍应正确；**这一点未由本用例证明**。

为什么不能证明：本项目的等价检查比的是"同一批字符给出相同的字形数与 advance"，
而**轴实例化后的轮廓差异不在那个比较的范围内**。要真正证明需要逐轴实例化再比
渲染像素，那需要一套 golden 图像，本项目当前没有。

所以这条用例的判据刻意是"**绝不静默做错**"而不是"一定正确"：
做了子集就必须过等价检查，否则必须带原因回退。

## 5 等价检查的判据是 `equivalence_probes > 0`

写第一版时我猜了字段名 `equivalence_ok`（不存在），编译器当场报出。
真实字段是 `equivalence_probes`（做了几次 shaping 比较），
因为 `subset_bytes` 的契约是"等价不过就回退"，所以**能走到 `Outcome::Subset`
就意味着等价已过**；能额外核的是"它到底比了没有"。

用例里的文案据此改成"做了子集却**一次等价检查都没做**" ——
比原来的"等价检查没过"更准确：它描述的是实际能被检查的那件事。

## 6 "账本"用例

`every_corpus_item_is_accounted_for` 把规格里的 11 项列成一张可执行的账本，
逐项写明由哪条用例覆盖。

加它的理由：**规格给了清单，而清单最容易被"做了一部分、剩下的忘了"**。
写成可执行账本之后，漏掉一项会在这里显形（改规格时也要改这张表）。

## 7 验证记录

```
cargo test -p font-service --test subset_corpus
→ 9 passed; 0 failed

cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 80/80 目标，0 失败
```

实测输出（`--nocapture`）：

```
CFF 字体 MiSans-Regular.otf 子集成功
注意：可变字体 bahnschrift.ttf 被做了子集（等价检查已过）—— …
```

TTC 那条走了回退分支且带原因（用例不打印回退信息，因为它不是异常）。

## 8 未完成（如实记录）

- **缺 golden 图像比较**。规格要求的 "rendered RGBA 达到 golden 条件" 需要
  一套固定的字体 + 固定的 renderer + 存好的参考图。本项目有真 libass，
  但没有把人眼可判的参考图固化下来。当前用 shaping 等价（字形数 + advance）
  作为替代，这比"什么都不查"强，但**比规格要求的弱**。
  要做它需要：选定语料字体（含许可）、渲染、存参考、写比较器 —— 而第一条
  就把问题带到许可审计上（21 §6）。
- **各脚本类别的"具体错法"没有覆盖**。用例证明的是"这些字符集会被回退"，
  没有证明"假如不回退，哪里会错"。后者需要一个故意放宽策略的对照实现。
- **`@Font` 前缀的竖排族名**由 `font scan` 侧处理，不在本 corpus 内
  （账本里已注明）。
- **variable font 的轴保真未证明**，见 §4。

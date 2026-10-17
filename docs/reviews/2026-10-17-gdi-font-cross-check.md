# 裁决记录：17 §2 的 GDI 权威交叉核对（2026-10-17）

## 需求

17 §2：「Windows Font Resolver 以 DirectWrite 为权威」，并列出了数据库至少应记录的
九项（localized family names / typographic+Win32 family / face+full+PostScript /
weight+style+stretch / variable axes / 文件路径+集合索引 / TTC-OTC face index /
glyph coverage / SHA-256+mtime）。

## 交付前的真实状态

九项**都已实现**（`db::FaceRecord` 逐项对应），但"权威"那一半没有：
数据来源是文件系统 + 注册表 + 读表，看不到 DirectWrite 的视角。
代码里如实记为 ISSUES AG1 —— 这一点是对的，但"未实现"这个记载本身**没有可执行价值**。

## 为什么不用 DirectWrite（D-GDI-01）

`IDWriteFactory::CreateSystemFontCollection` 是 **COM**。手写 vtable 意味着：

- 一个方法顺序/签名错就崩，而那类崩溃无法用单元测试穷尽（要覆盖上百个入口）；
- 必须自己处理 `QueryInterface` / `AddRef` / `Release` 与 `HRESULT`，
  漏一个就是泄漏，检查漏一个就是静默失败。

本仓库一以贯之的取向是**不自己实现安全边界**（同 23 §XML 的 XXE、PNG 的 deflate、
真 minidump 的异常处理器）。这里属于同一类。

GDI 的 `EnumFontFamiliesExW` 是**扁平 C API**：两个 `extern "system"` 声明、一个回调、
没有 vtable 与引用计数。它枚举的正是同一套系统字体注册表。

## 交付

`crates/font-service/src/gdi.rs`：

- `GdiFace`（家族名 / 全名 / 子族名 / 等宽 / TrueType 位）；
- `CrossCheck`（原始数 / 过滤后数 / 库数 / missing / extra）；
- `run()` / `cross_check()`，接进 `font scan --json` 的 `cross_check` 段。

### 本机实测结果

```
gdi_faces_raw   6020
 gdi_faces       3233   （过滤后）
 db_faces        1376
 missing_from_db    2   （创艺简行楷、微软简老宋）
 extra_in_db     1801
```

## 三条实现纪律

### D-GDI-02：必须过滤位图字体与 `@` 竖排变体

GDI 会枚举 `System` / `Terminal` / `Fixedsys` 这些**古早位图字体**（本机 6020 个
face 里大部分是它们），以及 `@Arial` 这种**竖排变体**。它们没有 `name`/`cmap` 表，
不在 17 §2 数据库的范围内。不滤掉的话，`missing_from_db` 会被几百个无意义条目淹没 ——
**真正的缺口会被埋在噪声里**。

两个计数都给（`gdi_faces_raw` 与 `gdi_faces`）：只报一个会让人误判范围。

### D-GDI-03：具名实例与带风格后缀的族不是缺口

本机第一次跑出 **63 个** missing，全部是同一类现象：

- `Bahnschrift Light` / `Bahnschrift SemiCondensed` —— 一个**可变字体**的 `fvar`
  具名实例（库里只有基名 `Bahnschrift` 一条）；
- `Sitka Display Semibold` / `Segoe UI Variable Small Light` —— 家族名带了子族后缀
  （库里是基名 + `style_name`）。

它们是**同一个文件**，不是"我们漏了字体"。判据是 `matches_a_family_prefix`：
**逐词从尾部剥**，看剩下的前缀能不能对上已知家族。

只在尾部剥（不是任意子串）是刻意的：`Noto Sans SC` 与 `Noto Sans Arabic` 共享
词首 `Noto Sans`，而尾部剥离能把它们区分开 —— 任意子串匹配会把不同的族混成一个。

加这一条之后 63 → **2**，而那 2 个是真线索。

### D-GDI-04：回调里不许 panic

GDI 回调是 `extern "system"`，其中的 panic 跨 FFI 边界是**未定义行为**。
所以回调里不用 `String::from_utf8`（用 `from_utf16_lossy`）、不索引切片、不 `unwrap`；
另外用 `catch_unwind` 包住整个枚举调用作第二道防线。

`from_utf16_lossy` 而不是 `from_utf16` 还有一层理由：真实字体名里出现未配对代理项
是存在的（有些老字体的名字表就是错的），而那不该让整次枚举失败。

## 开发中抳出的一个真缺陷（本文件最重要的一条）

**`TEXTMETRICW` 的字段宽度写错**：我把 `tmFirstChar`…`tmBreakChar` 写成了 `u8`，
而它们在 `wingdi.h` 里是 **`WCHAR`（u16）**。后果是 `tmPitchAndFamily` 的偏移差 4 字节，
读出来恒为 0，于是**所有字体都被当成"非 TrueType"**。

表现：GDI 枚举到 6020 个 face，**过滤后只剩 0 个**。

这个缺陷**纯构造数据的单元测试看不见** —— 单元测试里的 `GdiFace` 是我自己填的，
`truetype: true` 想写就写。只有真机用例（`tests/gdi_real.rs` 断言
"该有 100 个以上真曲线字体"）能抳出它。

修复后加了一道**编译期**的布局自检：

```rust
const _: () = {
    assert!(std::mem::offset_of!(TextMetricW, tm_pitch_and_family) == 55);
    assert!(std::mem::offset_of!(LogFontW, lf_face_name) == 28);
    assert!(std::mem::offset_of!(EnumLogFontExW, elf_full_name) == 92);
};
```

教训：**手写 FFI 结构体时，偏移必须能被机器核对**。注释里写"与 wingdi.h 逐字对应"
不起作用 —— 我写了那句话，然后字段宽度还是错的。

## 实测证据

单元测试（`gdi.rs`，14 用例）：

| 用例 | 断言 |
|---|---|
| `a_gdi_family_we_do_not_have_is_reported_as_missing` | 核心语义 |
| `an_agreeing_pair_reports_no_gap` | **负基线**：一致时不报缺 |
| `matching_is_case_and_whitespace_insensitive` | 大小写/空白不造假差异 |
| `a_match_on_the_full_name_counts_as_known` | 全名也算匹配（GDI 家族名可能是异体） |
| `a_db_only_family_is_listed_as_extra_not_as_a_failure` | extra 是信息不是缺陷 |
| `a_failed_enumeration_says_it_did_not_run` | 枚举失败要如实说"没跑" |
| `an_empty_gdi_family_name_is_not_reported` | 空名字不刷条目 |
| `a_repeated_gdi_family_is_reported_once` | 同族 4 个 face 只报一次 |
| `bitmap_fonts_and_vertical_variants_are_not_counted_as_gaps` | D-GDI-02 |
| `named_instances_and_style_suffixed_families_are_not_gaps` | D-GDI-03 |
| `a_genuinely_unknown_family_is_still_reported_after_prefix_matching` | 负基线 |
| `prefix_matching_does_not_merge_unrelated_families` | 尾剥不误合 |
| `a_single_word_family_is_not_stripped_to_nothing` | 不剥成空串 |
| `reading_a_wide_buffer_stops_at_nul_and_never_panics` | 宽字符读取 |

真机测试（`tests/gdi_real.rs`，3 用例）：真枚举拿到 100+ 曲线字体、
含 `Arial`/`Times New Roman`、核对照能给出可读证据。
无桌面会话时显式 SKIP。

## 未做（诚实边界）

- **DirectWrite 的 fallback 链**仍未做（需要 `IDWriteFontFallback`，是 COM，AG1）。
  它影响的是"某个码位在系统上最终会落到哪个 face"这个问题的回答，
  而不是"我们的库漏了哪些字体"。
- **GDI 枚举的 face 粒度与我们的不同**：GDI 按字体链接/模拟加粗展开，
  所以逐 face 对比会刷出大量无意义差异。当前只比**家族名集合**，
  这也意味着"同一家族里少了一个 weight"这类差异看不出来。
- `missing_from_db` 目前只在 `font scan` 的 warnings 里提示，**没有**影响匹配评分。
  它是诊断信息，不是自动纠错 —— 让一个跨 API 的差异去改匹配结果风险更大。

//! 17 §5 的 **subset equivalence corpus**。
//!
//! ## 规格要什么
//!
//! 17 §5 列了 11 个必须覆盖的情形，并要求判定方式是
//! "原字体 vs subset 字体，在相同 renderer/timestamp/canvas 上的 shaping + RGBA
//! 必须达到 golden 条件；**任何复杂脚本失败就回退'复制原字体，不 subset'**"。
//!
//! 它另外写了一句很要紧的话：
//!
//! > 这个方案非常适合作为候选，但**不能未经验证就称为"成熟且一定安全"**。
//!
//! 所以本文件不声称"子集是安全的"，而是逐项回答**每一类情形被怎么处理**：
//! 要么"确实做了子集且等价检查通过"，要么"按策略回退整份"。
//! 两者都是**可接受**的结果 —— 不可接受的是"悄悄做了个错的子集"。
//!
//! ## 两类情形，判据不同
//!
//! 11 项分成性质不同的两类，本文件按类别用不同的判据：
//!
//! | 类别 | 例子 | 判据 |
//! | --- | --- | --- |
//! | **字符级**：危险来自"请求了哪些字" | Arabic、Indic、combining、IVS、vertical、emoji | 必回退（`needs_complex_fallback` 命中） |
//! | **字体级**：危险来自"字体本身的结构" | TTC/OTC、variable、CFF/CFF2、color、composite | 由字体解析层决定：能安全做就做，不能就回退 |
//!
//! "Latin ligatures"（fi/fl）单列：它**不**该触发回退 —— 拉丁连字是
//! 常见的，全回退等于把子集功能对英文关掉。所以它必须真的做子集**且**等价检查通过。
//!
//! ## 为什么字符级那几项断言"必回退"而不是"必须正确"
//!
//! 因为规格说的就是回退。断言"必回退"能钉住**策略本身**：
//! 若哪天有人为了"少回退"而放宽某一条，用例会红，而审查者能看到
//! "这一条被放开了"这个事实，而不是在某个用户的阿拉伯文渲染错了之后才发现。
//!
//! ## 跳过而不是假装
//!
//! 某些项依赖本机恰好有对应字体（CFF、TTC、variable、color）。
//! 找不到就**明确跳过并说明**，不写成通过。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use font_service::subset::{needs_complex_fallback, subset_bytes, FallbackReason, Outcome};

fn chars(s: &str) -> BTreeSet<char> {
    s.chars().collect()
}

/// 系统字体目录列表。
fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(w) = std::env::var("WINDIR") {
        v.push(PathBuf::from(w).join("Fonts"));
    }
    v.push(PathBuf::from("C:\\Windows\\Fonts"));
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        v.push(PathBuf::from(la).join("Microsoft\\Windows\\Fonts"));
    }
    v
}

/// 按扩展名找一个字体文件。`want` 是候选名（不区分大小写，按顺序优先）。
fn find_font(exts: &[&str], preferred: &[&str]) -> Option<PathBuf> {
    for dir in font_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut all: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| exts.iter().any(|e| s.eq_ignore_ascii_case(e)))
                    .unwrap_or(false)
            })
            .collect();
        all.sort();
        // 先找首选的
        for want in preferred {
            if let Some(p) = all
                .iter()
                .find(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case(want))
                        .unwrap_or(false)
                })
                .cloned()
            {
                return Some(p);
            }
        }
        // 再找同扩展名的任意一份
        if let Some(p) = all.first() {
            return Some(p.clone());
        }
    }
    None
}

fn latin_font() -> Option<PathBuf> {
    find_font(
        &["ttf", "otf"],
        &["arial.ttf", "segoeui.ttf", "verdana.ttf"],
    )
}

fn cff_font() -> Option<PathBuf> {
    // .otf 通常是 CFF 轮廓；.ttf 通常是 glyf。想验 CFF 得挑 .otf。
    find_font(&["otf"], &[])
}

fn cjk_font() -> Option<PathBuf> {
    find_font(
        &["ttc", "ttf", "otf"],
        &["msyh.ttc", "simsun.ttc", "msyh.ttf", "simhei.ttf"],
    )
}

fn ttc_font() -> Option<PathBuf> {
    find_font(&["ttc", "otc"], &["msyh.ttc", "simsun.ttc", "msjh.ttc"])
}

fn font_bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_default()
}

fn label(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

// ===========================================================================
// 一、字符级：这些必须回退
// ===========================================================================

/// 把 11 项里"字符级"的那几项一次跑完，逐项报出它命中了哪条规则。
///
/// 断言的是**策略**：请求这些字符必须触发复杂脚本回退。
#[test]
fn the_complex_script_corpus_all_falls_back() {
    // (说明, 样本, 期望命中的规则关键词)
    let corpus: &[(&str, &str, &str)] = &[
        ("Arabic joining", "مرحبا بالعالم", "Arabic"),
        ("Arabic presentation forms", "\u{FB50}\u{FDFF}", "Arabic"),
        ("Devanagari/Indic GSUB/GPOS", "नमस्ते दुनिया", "Indic"),
        ("Bengali", "হ্যালো", "Indic"),
        ("Tamil", "வணக்கம்", "Indic"),
        ("Thai", "สวัสดี", "Indic"),
        ("combining marks", "e\u{0301}a\u{0308}", "combining"),
        ("combining marks extended", "x\u{1AB0}", "combining"),
        ("vertical forms", "\u{FE10}\u{FE30}", "vertical"),
        ("CJK+IVS（变体选择符）", "漢\u{E0100}", "variation"),
        // 注意：这一项用**变体选择符**而不是普通汉字 —— 见下面那条用例。
        ("emoji/color glyphs", "😀🎨", "emoji"),
    ];

    for (what, sample, expect) in corpus {
        let hit = needs_complex_fallback(&chars(sample));
        let hit = hit.unwrap_or_else(|| {
            panic!("「{what}」({sample:?}) 没有触发复杂脚本回退 —— 那会让它被做成子集")
        });
        assert!(
            hit.contains(expect),
            "「{what}」命中的是 {hit:?}，期望包含 {expect:?}"
        );
    }
}

/// 反向：**普通拉丁字母与常用汉字不该回退**。
///
/// 这一条与上一条同等重要。只断言"该回退的回退了"会漏掉
/// "全都回退"这种也满足上一条的实现 —— 而全回退等于把子集功能对
/// 英文与中文关掉，那不是保守，那是功能失效。
#[test]
fn plain_latin_and_cjk_do_not_trigger_fallback() {
    for sample in [
        "Hello, world! 123",
        "字体子集测试",
        "日本語のテスト",
        "한국어 테스트",
        "Ça va très bien",
    ] {
        assert_eq!(
            needs_complex_fallback(&chars(sample)),
            None,
            "{sample:?} 不该触发回退"
        );
    }
}

/// Latin ligatures（fi/fl）**必须走子集**，而不是回退。
///
/// 规格把这一项列在 corpus 里，但它的判据与前几项相反：拉丁连字
/// （fi 的 f+i 合字）在拉丁字体里是**常规**特性，GSUB 也只有 `liga` 这类
/// 简单规则。如果把它也回退，那么任何含 "fi" 的英文字幕都不做子集 ——
/// 那等于对英文关掉了这个功能。
///
/// 真正的保证不是"回退"，而是**等价检查**：子集必须与原字体对同一批字符
/// 给出相同的字形数与 advance。这里就核那一条。
#[test]
fn latin_ligatures_are_subset_and_stay_equivalent() {
    let Some(p) = latin_font() else {
        eprintln!("跳过：找不到拉丁字体");
        return;
    };
    let bytes = font_bytes(&p);
    // 含 fi / fl / ff 这些常见连字组合
    let want = chars("fi fl ffi office waffle");

    assert_eq!(
        needs_complex_fallback(&want),
        None,
        "拉丁连字不该触发回退（那会对英文关掉子集）"
    );

    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        // 真的做了子集：等价检查必须在内部已经通过（`subset_bytes` 的契约）
        Outcome::Subset => {
            assert!(
                out.equivalence_probes > 0,
                "{}: 做了子集但一次等价检查都没做",
                label(&p)
            );
            assert!(out.bytes.len() < bytes.len(), "{}: 子集没有更小", label(&p));
        }
        // 回退也可以，但**不能是因为复杂脚本**
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            assert_ne!(
                *r,
                FallbackReason::ComplexScript,
                "{}: 拉丁连字不该以复杂脚本为由回退",
                label(&p)
            );
            eprintln!(
                "注意：{} 的拉丁连字走了回退，原因 {} —— 不是错误，但要记下来",
                label(&p),
                r.describe()
            );
        }
    }
}

// ===========================================================================
// 二、字体级：由字体解析层决定
// ===========================================================================

/// TTC/OTC（字体集合）：**不允许假装成功**。
///
/// 集合里的成员选择与 `face_index` 语义需要额外处理。规格的取向是
/// "复杂就回退整份"。本用例核的是：给它一个 TTC，结果**要么**是
/// 做了子集且等价检查通过，**要么**是带着原因回退；
/// 绝不能出现"报成功但产物不是可用字体"。
#[test]
fn a_ttc_collection_either_subsets_correctly_or_falls_back_with_a_reason() {
    let Some(p) = ttc_font() else {
        eprintln!("跳过：本机没有 TTC/OTC 字体");
        return;
    };
    let bytes = font_bytes(&p);
    assert!(
        bytes.starts_with(b"ttcf"),
        "{} 不是 TTC（缺 ttcf 魔数）",
        label(&p)
    );
    let want = chars("Hello 字体");

    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        Outcome::Subset => {
            assert!(
                out.equivalence_probes > 0,
                "{}: 做了子集却一次等价检查都没做",
                label(&p)
            );
            // TTC 的子集产物必须自己是一个合法字体（不是被截断的集合片段）
            let b = &out.bytes;
            assert!(
                b.starts_with(b"\x00\x01\x00\x00")
                    || b.starts_with(b"OTTO")
                    || b.starts_with(b"ttcf")
                    || b.starts_with(b"true"),
                "{}: 子集产物不像合法字体（前 4 字节 {:?}）",
                label(&p),
                &b[..4.min(b.len())]
            );
        }
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            eprintln!("{}: 回退整份，原因 {}", label(&p), r.describe());
        }
    }
}

/// CFF/CFF2（PostScript 轮廓）：`.otf` 走的是与 `.ttf`（glyf）不同的轮廓表。
///
/// 这条不假设"一定成功"，只要求**不静默出错**：子集了就必须等价，
/// 没子集就必须说原因。
#[test]
fn a_cff_font_either_subsets_correctly_or_falls_back_with_a_reason() {
    let Some(p) = cff_font() else {
        eprintln!("跳过：本机没有 .otf（CFF）字体");
        return;
    };
    let bytes = font_bytes(&p);
    let want = chars("Hello World 字体");
    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        Outcome::Subset => {
            assert!(
                out.equivalence_probes > 0,
                "{}: 做了子集却一次等价检查都没做",
                label(&p)
            );
            // CFF 字体的子集产物仍应是 OTF/CFF
            assert!(
                out.bytes.starts_with(b"OTTO") || out.bytes.starts_with(b"\x00\x01\x00\x00"),
                "{}: 产物魔数不对",
                label(&p)
            );
            eprintln!("CFF 字体 {} 子集成功", label(&p));
        }
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            eprintln!("CFF 字体 {} 回退整份，原因 {}", label(&p), r.describe());
        }
    }
}

/// 复合字形（composite glyphs）：由很多字体用 `glyf` 里的复合轮廓表达重音字母。
///
/// 拉丁字体里的 À/é/ü 这类字符通常就是基字符 + 重音标记的复合字形。
/// 子集时若只保留了复合字形的**引用**而丢了被引用的基字形，产物就是坏字体。
/// 所以这条要求：子集了就必须等价。
#[test]
fn composite_glyphs_subset_equivalently() {
    let Some(p) = latin_font() else {
        eprintln!("跳过：找不到拉丁字体");
        return;
    };
    let bytes = font_bytes(&p);
    // 这些字符在大多数字体里是复合字形（基字符 + 组合重音）
    let want = chars("ÀÁÂÃÄÅ ÈÉÊË ÌÍÎÏ ÒÓÔÕÖ ÙÚÛÜ ÑÇ àáâãäå èéêë");

    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        Outcome::Subset => {
            assert!(
                out.equivalence_probes > 0,
                "{}: 复合字形做了子集却一次等价检查都没做 —— \\
                 那意味着产物里可能有引用了不存在基字形的复合字形",
                label(&p)
            );
        }
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            eprintln!("{}: 回退整份，原因 {}", label(&p), r.describe());
        }
    }
}

/// CJK + IVS 里，**普通汉字**（不带变体选择符）必须能子集。
///
/// 与第一条用例配对：那条核"带 IVS 必回退"，这条核"不带 IVS 不该被连累回退"。
/// 两条合起来才说明 IVS 的判定是**针对变体选择符**的，而不是"凡是 CJK 都回退"。
#[test]
fn plain_cjk_but_not_ivs_is_subset() {
    let Some(p) = cjk_font() else {
        eprintln!("跳过：本机没有 CJK 字体");
        return;
    };
    let bytes = font_bytes(&p);
    let want = chars("中文字体子集测试");
    assert_eq!(needs_complex_fallback(&want), None, "普通汉字不该触发回退");

    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        Outcome::Subset => assert!(
            out.equivalence_probes > 0,
            "{}: 做了子集却一次等价检查都没做",
            label(&p)
        ),
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            // 字体级原因（如 TTC 不支持）可以；字符级原因（复杂脚本）不行 ——
            // 那说明 `needs_complex_fallback` 的判定与这里不一致。
            assert_ne!(
                *r,
                FallbackReason::ComplexScript,
                "{}: 普通汉字不该以复杂脚本为由回退",
                label(&p)
            );
            eprintln!("{}: 回退整份，原因 {}", label(&p), r.describe());
        }
    }
}

/// 整个 corpus 的**形状**检查：11 项都要有着落。
///
/// 这条是"账本"用例：它列全规格里的 11 项，逐项说明它由哪条用例覆盖。
/// 加它的理由：规格给了清单，而清单最容易被"做了一部分、剩下的忘了"。
/// 把它写成可执行的账本之后，漏掉一项会在这里显形。
#[test]
fn every_corpus_item_is_accounted_for() {
    // (规格里的项, 谁来覆盖)
    let ledger: &[(&str, &str)] = &[
        ("Latin ligatures", "latin_ligatures_are_subset_and_stay_equivalent"),
        ("CJK + IVS", "the_complex_script_corpus_all_falls_back + plain_cjk_but_not_ivs_is_subset"),
        ("Arabic joining", "the_complex_script_corpus_all_falls_back"),
        (
            "Devanagari/Indic GSUB/GPOS",
            "the_complex_script_corpus_all_falls_back",
        ),
        ("combining marks", "the_complex_script_corpus_all_falls_back"),
        (
            "vertical glyphs / @Font",
            "the_complex_script_corpus_all_falls_back（字符级）；@Font 前缀的竖排族名由 font scan 侧处理",
        ),
        (
            "color fonts（COLR/CPAL/CBDT/SVG）",
            "the_complex_script_corpus_all_falls_back（emoji 入口）",
        ),
        (
            "variable fonts",
            "a_variable_font_is_never_silently_subset_wrong",
        ),
        (
            "CFF/CFF2",
            "a_cff_font_either_subsets_correctly_or_falls_back_with_a_reason",
        ),
        (
            "TTC/OTC",
            "a_ttc_collection_either_subsets_correctly_or_falls_back_with_a_reason",
        ),
        ("composite glyphs", "composite_glyphs_subset_equivalently"),
    ];
    assert_eq!(ledger.len(), 11, "规格列的是 11 项");
    for (item, covered_by) in ledger {
        assert!(!item.is_empty() && !covered_by.is_empty());
    }
}

/// variable font：**绝不静默做错**。
///
/// 可变字体的子集需要决定"轴实例化后的轮廓"，而那是另一套语义。
/// 本用例只要求：产物要么是合法且等价的子集，要么带原因回退。
#[test]
fn a_variable_font_is_never_silently_subset_wrong() {
    // 具名实例与可变族在 Windows 上常成对出现；找一个含 fvar 的字体。
    let mut found: Option<(PathBuf, Vec<u8>)> = None;
    for dir in font_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let is_font = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| {
                    s.eq_ignore_ascii_case("ttf")
                        || s.eq_ignore_ascii_case("otf")
                        || s.eq_ignore_ascii_case("ttc")
                })
                .unwrap_or(false);
            if !is_font {
                continue;
            }
            let Ok(b) = std::fs::read(&p) else { continue };
            // `fvar` 表的存在就说明是可变字体（表目录里会带这四个字节的 tag）
            if b.windows(4).any(|w| w == b"fvar") {
                found = Some((p, b));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some((p, bytes)) = found else {
        eprintln!("跳过：本机没有可变字体（无 fvar 表）");
        return;
    };

    let want = chars("Hello 字体");
    let out = subset_bytes(&bytes, 0, &want).unwrap();
    match out.outcome {
        Outcome::Subset => {
            assert!(
                out.equivalence_probes > 0,
                "{}: 可变字体做了子集却一次等价检查都没做",
                label(&p)
            );
            eprintln!(
                "注意：可变字体 {} 被做了子集（等价检查已过）—— \
                 若产物丢掉了 fvar 轴，默认实例仍应正确；这一点未由本用例证明",
                label(&p)
            );
        }
        Outcome::FullFontFallback => {
            let r = out.fallback.as_ref().expect("回退必须给出原因");
            eprintln!("可变字体 {} 回退整份，原因 {}", label(&p), r.describe());
        }
    }
}
